//! Independent protected HostSession installations on one shared Connect server.
#![allow(clippy::unwrap_used)]
use super::fixture::Fixture;
use connect_protocol::tasks::*;
use connect_server::cli::{
    init::{private_directory, private_write},
    task_access::{codex, GrantFile},
};
use connect_service::tasks::IssuedTaskGrant;
use std::path::PathBuf;

pub struct Host {
    pub directory: PathBuf,
    pub grant: IssuedTaskGrant,
    pub task: TaskRecord,
}
impl Host {
    /// Preserve the first resumed attempt before reusing the shared CLI probe.
    pub fn archive_resume(&self) {
        for extension in ["jsonl", "stderr"] {
            let destination = self.directory.join(format!("exec-resumed-1.{extension}"));
            assert!(!destination.exists());
            std::fs::rename(
                self.directory.join(format!("exec-resumed.{extension}")),
                destination,
            )
            .unwrap();
        }
    }
    pub async fn issue(f: &Fixture, task: TaskRecord, name: &str) -> Self {
        let binding = InvocationBinding {
            kind: InvocationKind::HostSession,
            adapter: "connect-codex-workflow-fixture".into(),
            external_id: name.into(),
        };
        f.server
            .service
            .register_task_binding(
                f.owner.credential_id.clone(),
                task.id.clone(),
                binding.clone(),
            )
            .await
            .unwrap();
        let definition = GrantDefinition {
            task_id: task.id.clone(),
            binding,
            operations: task.scope.operations.clone(),
            expires_at: f.issued.grant.expires_at,
        };
        let request_id = format!("{name}-grant");
        let grant = f
            .server
            .service
            .issue_task_grant(
                f.owner.credential_id.clone(),
                request_id.clone(),
                definition.clone(),
            )
            .await
            .unwrap();
        let parent = f.directory().join(name);
        private_directory(&parent).unwrap();
        let source = parent.join("grant.json");
        let file = GrantFile {
            profile: TASK_ACCESS_PROFILE.into(),
            request: TaskAccessRequest::new(TaskAccessCommand::Issue {
                request_id,
                definition,
            }),
            grant: Some(ProtectedTaskGrant::new(
                grant.grant.clone(),
                grant.expose_secret().into(),
            )),
        };
        private_write(&source, &serde_json::to_vec(&file).unwrap()).unwrap();
        let directory = parent.join("host");
        codex::export(&source, f.server.mcp_address.unwrap(), &directory).unwrap();
        Self {
            directory: std::fs::canonicalize(directory).unwrap(),
            grant,
            task,
        }
    }
}

/// Create two independently bound tasks through the native public API.
pub async fn pair(f: &Fixture) -> [Host; 2] {
    pair_with_scope(
        f,
        false,
        vec![
            TaskPermission::Send,
            TaskPermission::History,
            TaskPermission::Inbox,
        ],
    )
    .await
}

/// Overlapping work shares conversation context but keeps distinct task bindings.
pub async fn pair_with_scope(
    f: &Fixture,
    same_conversation: bool,
    permissions: Vec<TaskPermission>,
) -> [Host; 2] {
    let mut owner = connect_client::ConnectClient::open(f.owner.clone())
        .await
        .unwrap();
    owner.negotiate_task(&f.task.id).await.unwrap();
    let mut tasks = vec![];
    for index in 0..2 {
        let cid = if index == 0 || same_conversation {
            f.task.conversation_id.clone()
        } else {
            owner
                .create("codex-other-conversation", vec![f.peer.identity_id.clone()])
                .await
                .unwrap()
                .id
        };
        let outcome = owner
            .task(TaskCommand::Create {
                request_id: format!("codex-workflow-task-{index}"),
                definition: TaskDefinition {
                    conversation_id: cid,
                    scope: TaskScope {
                        operations: permissions.clone(),
                        recipients: vec![f.peer.identity_id.clone()],
                        sms_enrollments: vec![],
                    },
                    expires_at: f.task.expires_at,
                    initiating_event_id: None,
                },
            })
            .await
            .unwrap();
        let TaskOutcome::Record(record) = outcome else {
            panic!("task record");
        };
        tasks.push(*record);
    }
    owner.close().await;
    [
        Host::issue(f, tasks.remove(0), "host-a").await,
        Host::issue(f, tasks.remove(0), "host-b").await,
    ]
}
