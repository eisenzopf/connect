//! Account-free artifact setup through the protected exchange and public SDK.
#![allow(clippy::unwrap_used)]
use super::{grants, native_tasks::Harness};
use connect_client::{tasks::*, ClientConfig, ConnectClient};
use connect_protocol::{Artifact, Operation, Outcome};

pub struct Fixture {
    pub h: Harness,
    pub task: TaskRecord,
    pub client: ConnectClient,
    pub config: ClientConfig,
    pub grant: ProtectedTaskGrant,
}

impl Fixture {
    pub async fn start() -> Self {
        let mut h = Harness::start().await;
        h.owner
            .negotiate_tasks("connect:agent@local", Some(&h.cid))
            .await
            .unwrap();
        let permissions = vec![
            TaskPermission::ArtifactBegin,
            TaskPermission::ArtifactAppend,
            TaskPermission::ArtifactFinish,
            TaskPermission::ArtifactGet,
            TaskPermission::ArtifactDelete,
        ];
        let task = super::native_tasks::record(
            h.owner
                .task(TaskCommand::Create {
                    request_id: "artifact-task".into(),
                    definition: TaskDefinition {
                        conversation_id: h.cid.clone(),
                        scope: TaskScope {
                            operations: permissions.clone(),
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
        let grant = grants::issue(
            &h,
            &task,
            "artifact-grant",
            permissions,
            chrono::Utc::now().timestamp() + 180,
        )
        .await;
        let config = h.owner_config.clone().with_task_grant(&grant).unwrap();
        let mut client = ConnectClient::open(config.clone()).await.unwrap();
        client.negotiate_task(&task.id).await.unwrap();
        Self {
            h,
            task,
            client,
            config,
            grant,
        }
    }

    pub async fn invoke(&mut self, id: &str, operation: Operation) -> Outcome {
        completed(
            self.client
                .task(grants::wrapped(&self.task.id, id, operation))
                .await
                .unwrap(),
        )
    }

    pub async fn begin(&mut self, size: u64, digest: &str) -> Artifact {
        let outcome = self
            .invoke(
                "begin",
                Operation::ArtifactBegin {
                    request_id: "begin".into(),
                    conversation_id: self.h.cid.clone(),
                    name: "binary.bin".into(),
                    media_type: "application/octet-stream".into(),
                    size,
                    sha256: digest.into(),
                    expires_at: None,
                },
            )
            .await;
        artifact(outcome)
    }

    pub async fn close(self) {
        self.client.close().await;
        self.h.close().await;
    }
}

pub fn completed(outcome: TaskOutcome) -> Outcome {
    let TaskOutcome::Tool(TaskToolResult::Completed { outcome, .. }) = outcome else {
        panic!("expected completed artifact operation")
    };
    *outcome
}

pub fn artifact(outcome: Outcome) -> Artifact {
    let Outcome::Artifact(artifact) = outcome else {
        panic!("expected artifact metadata")
    };
    artifact
}
