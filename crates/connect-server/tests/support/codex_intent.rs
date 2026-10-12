//! Owned fixture composition for actual Codex interruption boundaries.
#![allow(clippy::unwrap_used)]
use super::{barrier::Barrier, exec::Exec, fixture::Fixture, install, responses::Responses};
use connect_mcp::{FixedBindingAuth, RunningMcp};
use connect_protocol::tasks::{GrantDefinition, InvocationBinding, InvocationKind, TaskPermission};
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc, time::Duration};

pub struct Case {
    pub f: Fixture,
    pub barrier: Arc<Barrier>,
    pub responses: Responses,
    pub arguments: Value,
    directory: PathBuf,
    binary: PathBuf,
    mcp: Option<RunningMcp>,
}
impl Case {
    pub async fn start(request: &str, body: &str) -> Self {
        let binary = PathBuf::from(
            std::env::var_os("CONNECT_TEST_CODEX_BINARY").expect("Select actual installed Codex"),
        );
        let mut f = Fixture::start_permitted(
            true,
            vec![
                TaskPermission::Send,
                TaskPermission::History,
                TaskPermission::Inbox,
            ],
        )
        .await;
        // Protected composition registers the installation handle before model
        // startup. The later Codex UUID is correlation, never minting authority.
        let binding = InvocationBinding {
            kind: InvocationKind::HostSession,
            adapter: "connect-codex-fixture".into(),
            external_id: format!("installed-{}", f.task.id),
        };
        f.server
            .service
            .register_task_binding(
                f.owner.credential_id.clone(),
                f.task.id.clone(),
                binding.clone(),
            )
            .await
            .unwrap();
        let issued = f
            .server
            .service
            .issue_task_grant(
                f.owner.credential_id.clone(),
                "codex-fixture-grant".into(),
                GrantDefinition {
                    task_id: f.task.id.clone(),
                    binding: binding.clone(),
                    operations: f.issued.grant.operations.clone(),
                    expires_at: f.issued.grant.expires_at,
                },
            )
            .await
            .unwrap();
        f.binding = binding;
        f.issued = issued;
        let directory = install::export(&f, "codex-fixture-grant");
        let barrier = Arc::new(Barrier::new(
            FixedBindingAuth::new(f.server.service.clone(), f.binding.clone()).unwrap(),
        ));
        let mcp = RunningMcp::start_local_auxiliary(
            f.server.service.clone(),
            barrier.clone(),
            "127.0.0.1:0".parse().unwrap(),
        )
        .await
        .unwrap();
        let arguments =
            json!({"request_id":request,"step_id":"draft","body":body,"wait_for_reply":true});
        let responses = Responses::start(barrier.clone(), arguments.clone()).await;
        Self {
            f,
            barrier,
            responses,
            arguments,
            directory,
            binary,
            mcp: Some(mcp),
        }
    }
    pub fn launch(&self, session: Option<&str>) -> Exec {
        Exec::start(
            &self.binary,
            &self.directory,
            &self.responses.url,
            self.mcp.as_ref().unwrap().address,
            self.f.issued.expose_secret(),
            session,
        )
    }
    pub async fn checkpoint(&self, first: &Exec) -> Value {
        tokio::time::timeout(Duration::from_secs(15), self.barrier.entered.notified())
            .await
            .unwrap();
        let saved = super::exec::saved_call(&first.home);
        assert!(saved["input"]
            .as_str()
            .unwrap()
            .contains(&format!("const intent = {};", self.arguments)));
        saved
    }
    pub async fn returned(&self) {
        tokio::time::timeout(
            Duration::from_secs(15),
            self.responses.script.returned.notified(),
        )
        .await
        .unwrap();
    }
    /// Drain the old paused admission before exposing a replacement listener.
    pub async fn restart_mcp(&mut self) {
        let mcp = self.mcp.take().unwrap();
        let address = mcp.address;
        mcp.shutdown().await.unwrap();
        self.mcp = Some(
            RunningMcp::start_local_auxiliary(
                self.f.server.service.clone(),
                Arc::new(
                    FixedBindingAuth::new(self.f.server.service.clone(), self.f.binding.clone())
                        .unwrap(),
                ),
                address,
            )
            .await
            .unwrap(),
        );
    }
    pub async fn close(mut self) {
        let wire = serde_json::to_string(&self.responses.requests()).unwrap();
        assert!(!wire.contains(self.f.issued.expose_secret()));
        assert!(!wire.contains(&self.f.owner.credential));
        self.mcp.take().unwrap().shutdown().await.unwrap();
        self.f.close().await;
    }
}
