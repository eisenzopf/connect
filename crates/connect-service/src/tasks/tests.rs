#![allow(clippy::unwrap_used)]
use crate::Service;
use connect_protocol::{tasks::*, Limits, Operation, Outcome};
use connect_store::{Scope, Store};
use std::sync::Arc;

struct Fixture {
    _dir: tempfile::TempDir,
    service: Arc<Service>,
    key: String,
    task: TaskRecord,
    cursor: Option<String>,
    reached: Arc<tokio::sync::Barrier>,
    release: Arc<tokio::sync::Barrier>,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(
            Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap(),
        );
        let owner = store.create_identity("owner").unwrap().id;
        let now = chrono::Utc::now().timestamp();
        let key = store
            .issue(&owner, Scope::operator(), now + 600)
            .unwrap()
            .id;
        let cid = match store
            .execute(
                &key,
                &Operation::Create {
                    request_id: "conversation".into(),
                    participants: vec![],
                },
            )
            .unwrap()
        {
            Outcome::Conversation(c) => c.id,
            _ => panic!("conversation"),
        };
        let task = match store
            .execute_task(
                &key,
                &TaskRequest::new(TaskCommand::Create {
                    request_id: "task".into(),
                    definition: TaskDefinition {
                        conversation_id: cid,
                        scope: TaskScope {
                            sms_enrollments: Vec::new(),
                            operations: vec![TaskPermission::Inbox],
                            recipients: vec![owner],
                        },
                        expires_at: now + 300,
                        initiating_event_id: None,
                    },
                }),
            )
            .unwrap()
        {
            TaskOutcome::Record(task) => *task,
            _ => panic!("task"),
        };
        let cursor = match store
            .execute_task(
                &key,
                &TaskRequest::new(TaskCommand::Inbox {
                    task_id: task.id.clone(),
                    after: None,
                    limit: 100,
                    wait_ms: 0,
                }),
            )
            .unwrap()
        {
            TaskOutcome::Inbox { next, .. } => next,
            _ => panic!("cursor"),
        };
        let reached = Arc::new(tokio::sync::Barrier::new(2));
        let release = Arc::new(tokio::sync::Barrier::new(2));
        let service = Arc::new(Service {
            store,
            readiness: Arc::default(),
            changed: Default::default(),
            runtime: Default::default(),
            wait_boundary: Some((reached.clone(), release.clone())),
        });
        Self {
            _dir: dir,
            service,
            key,
            task,
            cursor,
            reached,
            release,
        }
    }
    fn wait(&self) -> TaskRequest {
        TaskRequest::new(TaskCommand::Wait {
            task_id: self.task.id.clone(),
            after: self.cursor.clone(),
            limit: 100,
            wait_ms: 5_000,
        })
    }
    async fn grant(&self) -> super::TaskInvocation {
        let binding = InvocationBinding {
            kind: InvocationKind::Native,
            adapter: "fixture".into(),
            external_id: "origin".into(),
        };
        self.service
            .register_task_binding(self.key.clone(), self.task.id.clone(), binding.clone())
            .await
            .unwrap();
        let grant = self
            .service
            .issue_task_grant(
                self.key.clone(),
                "issue".into(),
                GrantDefinition {
                    task_id: self.task.id.clone(),
                    binding: binding.clone(),
                    operations: vec![TaskPermission::Inbox],
                    expires_at: chrono::Utc::now().timestamp() + 120,
                },
            )
            .await
            .unwrap();
        self.service
            .authenticate_task_grant(grant.expose_secret().into(), binding)
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn commit_between_task_catchup_and_wait_is_not_lost_for_owner_or_grant() {
    for use_grant in [false, true] {
        let f = Fixture::new();
        let grant = f.grant().await;
        let service = f.service.clone();
        let key = f.key.clone();
        let wait = f.wait();
        let pending = tokio::spawn(async move {
            if use_grant {
                service.execute_task_invocation(grant, wait).await
            } else {
                service.execute_task(key, wait).await
            }
        });
        f.reached.wait().await; // the first journal read definitively returned empty
        f.service
            .execute_task(
                f.key.clone(),
                TaskRequest::new(TaskCommand::Transition {
                    request_id: "run".into(),
                    task_id: f.task.id.clone(),
                    expected_revision: f.task.revision,
                    state: TaskState::Running,
                    reason: None,
                }),
            )
            .await
            .unwrap();
        f.release.wait().await; // commit happened before awaiting the wake hint
        let result = tokio::time::timeout(std::time::Duration::from_secs(1), pending)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(
            matches!(result,TaskOutcome::Inbox {events,..} if events.len()==1 && matches!(events[0].data,TaskEventData::State {state:TaskState::Running,..}))
        );
    }
}

#[tokio::test]
async fn revocation_during_wait_is_rechecked_before_returning_events() {
    let f = Fixture::new();
    let grant = f.grant().await;
    let id = grant.grant_id().into();
    let service = f.service.clone();
    let wait = f.wait();
    let pending = tokio::spawn(async move { service.execute_task_invocation(grant, wait).await });
    f.reached.wait().await;
    f.service
        .revoke_task_grant(f.key.clone(), id)
        .await
        .unwrap();
    f.release.wait().await;
    let error = tokio::time::timeout(std::time::Duration::from_secs(1), pending)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(error.code, "forbidden");
}
