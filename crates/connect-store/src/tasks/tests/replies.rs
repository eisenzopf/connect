use super::fixture::Fixture;
use crate::{IssuedCredential, Scope, Store};
use connect_protocol::{tasks::*, Limits, Operation, Outcome};

pub(super) fn setup() -> (Fixture, IssuedCredential, String) {
    let mut f = Fixture::new();
    let human = f.store.create_identity("human").unwrap().id;
    let key = f
        .store
        .issue(
            &human,
            Scope::messaging(vec![f.owner.clone()]),
            f.now + 3_600,
        )
        .unwrap();
    f.cid = match f
        .store
        .execute(
            &f.key.id,
            &Operation::Create {
                request_id: "shared-conversation".into(),
                request: connect_protocol::conversations::CreateRequest::persistent(vec![
                    human.clone()
                ]),
            },
        )
        .unwrap()
    {
        Outcome::Conversation(c) => c.id,
        _ => panic!("conversation"),
    };
    (f, key, human)
}
pub(super) fn task(f: &Fixture, human: &str, id: &str) -> TaskRecord {
    let mut create = f.create();
    let TaskCommand::Create {
        request_id,
        definition,
    } = &mut create.command
    else {
        panic!("create")
    };
    *request_id = id.into();
    definition.scope.recipients = vec![human.into()];
    definition.scope.operations.push(TaskPermission::Inbox);
    Fixture::record(f.run(&create).unwrap())
}
pub(super) fn incoming(f: &Fixture, human: &IssuedCredential, id: &str) -> String {
    let message = match f
        .store
        .execute(
            &human.id,
            &Operation::Send {
                request_id: id.into(),
                conversation_id: f.cid.clone(),
                message: connect_protocol::messages::MessageDraft::text_all(format!("reply {id}")),
            },
        )
        .unwrap()
    {
        Outcome::Accepted { message_id, .. } => message_id,
        _ => panic!("message"),
    };
    f.store
        .connection()
        .unwrap()
        .query_row(
            "SELECT id FROM journal WHERE message_id=?1 AND recipient=?2",
            rusqlite::params![message, f.owner],
            |r| r.get(0),
        )
        .unwrap()
}
pub(super) fn assign(task: &TaskRecord, event: &str, id: &str) -> TaskRequest {
    TaskRequest::new(TaskCommand::AssignReply {
        request_id: id.into(),
        task_id: task.id.clone(),
        event_id: event.into(),
        reply_to: None,
    })
}
pub(super) fn inbox(task: &TaskRecord, after: Option<String>) -> TaskRequest {
    TaskRequest::new(TaskCommand::Inbox {
        task_id: task.id.clone(),
        after,
        limit: 100,
        wait_ms: 0,
    })
}
fn waiting(f: &Fixture, task: &TaskRecord) -> TaskRecord {
    let running = Fixture::record(
        f.run(&Fixture::transition(task, "running", TaskState::Running))
            .unwrap(),
    );
    Fixture::record(
        f.run(&Fixture::transition(
            &running,
            "waiting",
            TaskState::Waiting,
        ))
        .unwrap(),
    )
}

#[test]
fn two_tasks_keep_unassigned_replies_and_route_only_by_explicit_assignment() {
    let (f, key, human) = setup();
    let one = waiting(&f, &task(&f, &human, "one"));
    let two = task(&f, &human, "two");
    let a = incoming(&f, &key, "a");
    let b = incoming(&f, &key, "b");
    let list = TaskRequest::new(TaskCommand::Replies {
        conversation_id: f.cid.clone(),
        after: None,
        limit: 100,
    });
    assert!(
        matches!(f.run(&list).unwrap(), TaskOutcome::Replies {items,..} if items.len()==2 && items.iter().all(|item| item.task_id.is_none()))
    );
    f.run(&assign(&two, &b, "assign-b")).unwrap();
    let one = Fixture::record(f.run(&assign(&one, &a, "assign-a")).unwrap());
    assert_eq!(one.state, TaskState::Ready);
    for (task, event, other) in [(&one, &a, &b), (&two, &b, &a)] {
        let TaskOutcome::Inbox { events, next } = f.run(&inbox(task, None)).unwrap() else {
            panic!("inbox")
        };
        assert!(events
            .iter()
            .any(|e| matches!(&e.data,TaskEventData::Reply(reply) if &reply.event_id==event)));
        assert!(!events
            .iter()
            .any(|e| matches!(&e.data,TaskEventData::Reply(reply) if &reply.event_id==other)));
        assert!(
            matches!(f.run(&inbox(task,next.clone())).unwrap(),TaskOutcome::Inbox {events,next:new} if events.is_empty() && new==next)
        );
    }
    assert_eq!(
        f.run(&assign(&two, &a, "steal")).unwrap_err().code,
        "forbidden"
    );
    assert_eq!(
        f.store
            .execute_task(&key.id, &assign(&one, &a, "human-assign"))
            .unwrap_err()
            .code,
        "forbidden"
    );
}

#[test]
fn matching_replay_after_restart_and_consumption_never_advances_twice() {
    let (f, key, human) = setup();
    let task = task(&f, &human, "task");
    let event = incoming(&f, &key, "reply");
    let assignment = assign(&task, &event, "assign");
    let assigned = f.run(&assignment).unwrap();
    let task = Fixture::record(assigned.clone());
    assert_eq!(
        f.run(&assign(&task, &event, "duplicate")).unwrap_err().code,
        "reply-conflict"
    );
    let consume = TaskRequest::new(TaskCommand::ConsumeReply {
        request_id: "consume".into(),
        task_id: task.id.clone(),
        event_id: event.clone(),
        expected_revision: task.revision,
    });
    let consumed = f.run(&consume).unwrap();
    let restored = Store::open(&f._dir.path().join("state.sqlite3"), Limits::default()).unwrap();
    assert_eq!(
        restored.execute_task(&f.key.id, &consume).unwrap(),
        consumed
    );
    assert_eq!(
        restored.execute_task(&f.key.id, &assignment).unwrap(),
        assigned
    );
    let task = Fixture::record(consumed);
    let again = TaskRequest::new(TaskCommand::ConsumeReply {
        request_id: "again".into(),
        task_id: task.id.clone(),
        event_id: event,
        expected_revision: task.revision,
    });
    assert_eq!(f.run(&again).unwrap_err().code, "reply-consumed");
    let TaskOutcome::Inbox { events, .. } = f.run(&inbox(&task, None)).unwrap() else {
        panic!("inbox")
    };
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(&e.data,TaskEventData::Reply(r) if r.consumed))
            .count(),
        1
    );
}

#[test]
fn task_cursor_is_bound_to_task_identity_installation_and_current_authority() {
    let (f, key, human) = setup();
    let one = task(&f, &human, "one");
    let two = task(&f, &human, "two");
    let TaskOutcome::Inbox { next, .. } = f.run(&inbox(&one, None)).unwrap() else {
        panic!("inbox")
    };
    assert_eq!(
        f.run(&inbox(&two, next.clone())).unwrap_err().code,
        "invalid-request"
    );
    let limited = f
        .store
        .issue(
            &f.owner,
            Scope {
                operations: vec!["task_inbox".into()],
                recipients: vec![],
            },
            f.now + 600,
        )
        .unwrap();
    assert_eq!(
        f.store
            .execute_task(&limited.id, &inbox(&one, next.clone()))
            .unwrap_err()
            .code,
        "forbidden"
    );
    f.store
        .revoke(&f.store.authenticate(&f.key.token).unwrap(), &f.key.id)
        .unwrap();
    assert!(f.store.execute_task(&key.id, &inbox(&one, next)).is_err());
}

#[test]
fn assignment_and_consumption_roll_back_if_receipt_commit_fails() {
    let (f, key, human) = setup();
    let task = task(&f, &human, "task");
    let event = incoming(&f, &key, "reply");
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER fail_receipt BEFORE INSERT ON task_requests BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    let request = assign(&task, &event, "assign");
    assert_eq!(f.run(&request).unwrap_err().code, "storage-unavailable");
    let c = f.store.connection().unwrap();
    let count: i64 = c
        .query_row("SELECT COUNT(*) FROM task_replies", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
    c.execute_batch("DROP TRIGGER fail_receipt;").unwrap();
    drop(c);
    let task = Fixture::record(f.run(&request).unwrap());
    let consume = TaskRequest::new(TaskCommand::ConsumeReply {
        request_id: "consume".into(),
        task_id: task.id.clone(),
        event_id: event,
        expected_revision: task.revision,
    });
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER fail_receipt BEFORE INSERT ON task_requests BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(f.run(&consume).is_err());
    let c = f.store.connection().unwrap();
    let consumed: bool = c
        .query_row("SELECT consumed FROM task_replies", [], |r| r.get(0))
        .unwrap();
    assert!(!consumed);
    c.execute_batch("DROP TRIGGER fail_receipt;").unwrap();
    drop(c);
    assert!(f.run(&consume).is_ok());
}
