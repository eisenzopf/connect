//! Native media setup through protected grant exchange and public SDK operations.
#![allow(clippy::unwrap_used, dead_code)]
use super::{grants, native_tasks::Harness};
use connect_client::{tasks::*, ClientConfig, ConnectClient};
use connect_protocol::{Operation, Outcome, Session};
use connect_store::Scope;
use std::time::Duration;

pub(crate) struct Fixture {
    pub h: Harness,
    pub task: TaskRecord,
    pub peer_config: ClientConfig,
}
impl Fixture {
    pub async fn start() -> Self {
        Self::start_with_vapi(None).await
    }
    pub async fn start_with_vapi(adapter: Option<std::sync::Arc<rvoip_vapi::VapiAdapter>>) -> Self {
        let mut h = Harness::start_with_vapi(adapter).await;
        let key = h
            .server
            .store
            .issue(
                &h.peer_id,
                Scope::operator(),
                chrono::Utc::now().timestamp() + 3600,
            )
            .unwrap();
        let mut peer_config = h.owner_config.clone();
        peer_config.identity_id = h.peer_id.clone();
        peer_config.credential_id = key.id;
        peer_config.credential = key.token;
        h.peer.close().await;
        h.peer = ConnectClient::open(peer_config.clone()).await.unwrap();
        h.owner
            .negotiate_tasks("connect:agent@local", Some(&h.cid))
            .await
            .unwrap();
        let task = super::native_tasks::record(
            h.owner
                .task(TaskCommand::Create {
                    request_id: "media-task".into(),
                    definition: TaskDefinition {
                        conversation_id: h.cid.clone(),
                        scope: TaskScope {
                            operations: vec![
                                TaskPermission::SessionInvite,
                                TaskPermission::SessionGet,
                                TaskPermission::Send,
                                TaskPermission::History,
                            ],
                            recipients: vec![h.peer_id.clone()],
                            sms_enrollments: vec![],
                        },
                        expires_at: chrono::Utc::now().timestamp() + 600,
                        initiating_event_id: None,
                    },
                })
                .await
                .unwrap(),
        );
        Self {
            h,
            task,
            peer_config,
        }
    }
    pub async fn grant(
        &self,
        id: &str,
        ttl: i64,
        permissions: Vec<TaskPermission>,
    ) -> (ProtectedTaskGrant, ClientConfig) {
        let grant = grants::issue(
            &self.h,
            &self.task,
            id,
            permissions,
            chrono::Utc::now().timestamp() + ttl,
        )
        .await;
        let config = self.h.owner_config.clone().with_task_grant(&grant).unwrap();
        (grant, config)
    }
    pub fn permissions() -> Vec<TaskPermission> {
        vec![
            TaskPermission::SessionGet,
            TaskPermission::SessionInvite,
            TaskPermission::Send,
            TaskPermission::History,
        ]
    }
    pub async fn session(&mut self, client: &mut ConnectClient, id: &str) -> Session {
        client.negotiate_task(&self.task.id).await.unwrap();
        let result = client
            .task(grants::wrapped(
                &self.task.id,
                id,
                Operation::SessionInvite {
                    request_id: id.into(),
                    conversation_id: self.h.cid.clone(),
                    participant_id: self.h.peer_id.clone(),
                    media: vec!["audio".into()],
                },
            ))
            .await
            .unwrap();
        let TaskOutcome::Tool(TaskToolResult::Completed { outcome, .. }) = result else {
            panic!("invite")
        };
        let Outcome::Session(session) = *outcome else {
            panic!("session")
        };
        self.h
            .peer
            .session_accept(&format!("accept-{id}"), &self.h.cid, &session.id)
            .await
            .unwrap()
    }
    pub async fn binding(&self, grant: &str, expected: &str) {
        let c = rusqlite::Connection::open_with_flags(
            self.h.server.store.path(),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let count: u64 = c
                    .query_row(
                        "SELECT COUNT(*) FROM session_bindings WHERE task_grant_id=?1 AND state=?2",
                        [grant, expected],
                        |r| r.get(0),
                    )
                    .unwrap();
                if count > 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
    }
    pub async fn revoke(&self, grant: &str) {
        let response: TaskAccessResponse = grants::http()
            .post(format!("{}/v1/task-access", grants::url(&self.h)))
            .bearer_auth(&self.h.owner_config.credential)
            .json(&TaskAccessRequest::new(TaskAccessCommand::Revoke {
                grant_id: grant.into(),
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        response.result.unwrap();
    }
    pub async fn participant_grant(&self, task: &TaskRecord) -> (ProtectedTaskGrant, ClientConfig) {
        let response: TaskAccessResponse = grants::http()
            .post(format!("{}/v1/task-access", grants::url(&self.h)))
            .bearer_auth(&self.peer_config.credential)
            .json(&TaskAccessRequest::new(TaskAccessCommand::Issue {
                request_id: "participant-grant".into(),
                definition: GrantDefinition {
                    task_id: task.id.clone(),
                    binding: InvocationBinding {
                        kind: InvocationKind::Native,
                        adapter: "sdk-native-test".into(),
                        external_id: "participant".into(),
                    },
                    operations: vec![TaskPermission::SessionGet, TaskPermission::SessionAccept],
                    expires_at: chrono::Utc::now().timestamp() + 120,
                },
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let TaskAccessOutcome::Issued(grant) = response.result.unwrap() else {
            panic!("grant")
        };
        let config = self.peer_config.clone().with_task_grant(&grant).unwrap();
        (grant, config)
    }
    pub async fn close(self) {
        self.h.peer.close().await;
        self.h.owner.close().await;
        tokio::time::timeout(Duration::from_secs(5), self.h.server.shutdown())
            .await
            .unwrap()
            .unwrap();
    }
}
