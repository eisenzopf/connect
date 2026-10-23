#![allow(clippy::unwrap_used)]
use crate::Store;
use crate::{IssuedCredential, Scope};
use connect_protocol::Result;
use connect_protocol::{tasks::*, Limits, Operation, Outcome};

pub(super) struct Fixture {
    pub(super) _dir: tempfile::TempDir,
    pub(super) store: Store,
    pub(super) key: IssuedCredential,
    pub(super) owner: String,
    pub(super) cid: String,
    pub(super) now: i64,
}
impl Fixture {
    pub(super) fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store =
            Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap();
        let owner = store.create_identity("owner").unwrap().id;
        let now = crate::now();
        let key = store.issue(&owner, Scope::operator(), now + 3_600).unwrap();
        let cid = match store
            .execute(
                &key.id,
                &Operation::Create {
                    request_id: "conversation".into(),
                    request: connect_protocol::conversations::CreateRequest::persistent(vec![]),
                },
            )
            .unwrap()
        {
            Outcome::Conversation(c) => c.id,
            _ => panic!("conversation"),
        };
        Self {
            _dir: dir,
            store,
            key,
            owner,
            cid,
            now,
        }
    }
    pub(super) fn create(&self) -> TaskRequest {
        TaskRequest::new(TaskCommand::Create {
            request_id: "task-create".into(),
            definition: TaskDefinition {
                conversation_id: self.cid.clone(),
                scope: TaskScope {
                    sms_enrollments: Vec::new(),
                    operations: vec![TaskPermission::Send, TaskPermission::History],
                    recipients: vec![self.owner.clone()],
                },
                expires_at: self.now + 600,
                initiating_event_id: None,
            },
        })
    }
    pub(super) fn run(&self, request: &TaskRequest) -> Result<TaskOutcome> {
        self.store.execute_task_at(&self.key.id, request, self.now)
    }
    pub(super) fn record(outcome: TaskOutcome) -> TaskRecord {
        match outcome {
            TaskOutcome::Record(record) => *record,
            _ => panic!("record"),
        }
    }
    pub(super) fn task(&self) -> TaskRecord {
        Self::record(self.run(&self.create()).unwrap())
    }
    pub(super) fn transition(task: &TaskRecord, id: &str, state: TaskState) -> TaskRequest {
        TaskRequest::new(TaskCommand::Transition {
            request_id: id.into(),
            task_id: task.id.clone(),
            expected_revision: task.revision,
            state,
            reason: None,
        })
    }
}
