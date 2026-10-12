use super::fixture::Fixture;
use crate::{Scope, Store};
use connect_protocol::{tasks::*, Limits, Operation, Outcome};

#[test]
fn receipt_failure_rolls_back_task_and_journal_and_same_intent_can_retry() {
    let f = Fixture::new();
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER reject_receipt BEFORE INSERT ON task_requests BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert_eq!(f.run(&f.create()).unwrap_err().code, "storage-unavailable");
    {
        let c = f.store.connection().unwrap();
        for table in ["tasks", "task_events", "task_requests"] {
            let count: u64 = c
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 0);
        }
    }
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_receipt")
        .unwrap();
    f.task();
}

#[test]
fn concurrent_process_handles_serialize_revision_updates() {
    let f = Fixture::new();
    let task = f.task();
    let first = Store::open(f.store.path(), Limits::default()).unwrap();
    let second = Store::open(f.store.path(), Limits::default()).unwrap();
    let a = Fixture::transition(&task, "worker-a", TaskState::Running);
    let b = Fixture::transition(&task, "worker-b", TaskState::Running);
    let key = f.key.id.clone();
    let now = f.now;
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let start = barrier.clone();
    let second_key = key.clone();
    let worker = std::thread::spawn(move || {
        start.wait();
        first.execute_task_at(&key, &a, now)
    });
    barrier.wait();
    let result = second.execute_task_at(&second_key, &b, now);
    let other = worker.join().unwrap();
    assert_ne!(result.is_ok(), other.is_ok());
    let error = result.err().or_else(|| other.err()).unwrap();
    assert_eq!(error.code, "task-state-conflict");
    let c = f.store.connection().unwrap();
    assert_eq!(
        c.query_row::<u64, _, _>("SELECT COUNT(*) FROM task_events", [], |r| r.get(0))
            .unwrap(),
        2
    );
}

#[test]
fn task_events_and_receipts_share_existing_installation_capacity() {
    let dir = tempfile::tempdir().unwrap();
    let limits = Limits {
        // Conversation + task admission + its protected terminal record.
        journal_events: 3,
        operation_receipts: 3,
        ..Limits::default()
    };
    let store = Store::initialize(&dir.path().join("state.sqlite3"), limits).unwrap();
    let owner = store.create_identity("owner").unwrap();
    let now = crate::now();
    let key = store
        .issue(&owner.id, Scope::operator(), now + 600)
        .unwrap();
    let cid = match store
        .execute(
            &key.id,
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
    let task = TaskRequest::new(TaskCommand::Create {
        request_id: "task".into(),
        definition: TaskDefinition {
            conversation_id: cid.clone(),
            scope: TaskScope {
                sms_enrollments: Vec::new(),
                operations: vec![TaskPermission::Send],
                recipients: vec![owner.id],
            },
            expires_at: now + 300,
            initiating_event_id: None,
        },
    });
    store.execute_task(&key.id, &task).unwrap();
    let send = Operation::Send {
        request_id: "send".into(),
        conversation_id: cid,
        body: "Would exceed the shared bound".into(),
    };
    assert_eq!(
        store.execute(&key.id, &send).unwrap_err().code,
        "capacity-exceeded"
    );
    let record = Fixture::record(store.execute_task(&key.id, &task).unwrap());
    assert_eq!(
        store
            .execute_task(
                &key.id,
                &Fixture::transition(&record, "run", TaskState::Running)
            )
            .unwrap_err()
            .code,
        "capacity-exceeded"
    );
    let c = store.connection().unwrap();
    assert_eq!(
        c.query_row::<u64, _, _>("SELECT revision FROM tasks", [], |r| r.get(0))
            .unwrap(),
        0
    );
}

#[test]
fn active_and_retained_task_admission_have_distinct_hard_bounds() {
    let f = Fixture::new();
    let task = f.task();
    {
        let mut c = f.store.connection().unwrap();
        let tx = c.transaction().unwrap();
        for i in 1..MAX_RETAINED_TASKS {
            // Populate isolated fixture metadata without effects, while retaining
            // exactly the declared active ceiling. The SQL API is test-only here.
            let state = if i < MAX_ACTIVE_TASKS {
                "ready"
            } else {
                "completed"
            };
            tx.execute("INSERT INTO tasks(id,owner_id,credential_id,conversation_id,scope_json,state,revision,expires_at,created_at,updated_at) SELECT ?1,owner_id,credential_id,conversation_id,scope_json,?2,0,expires_at,created_at,updated_at FROM tasks WHERE id=?3",rusqlite::params![format!("fixture-{i}"),state,task.id]).unwrap();
        }
        tx.commit().unwrap();
    }
    let mut request = f.create();
    let TaskCommand::Create { request_id, .. } = &mut request.command else {
        panic!("create")
    };
    *request_id = "another".into();
    assert_eq!(f.run(&request).unwrap_err().code, "capacity-exceeded");
    // Free active capacity while retained metadata remains full: independently denied.
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE tasks SET state='completed' WHERE id='fixture-1'",
            [],
        )
        .unwrap();
    assert_eq!(f.run(&request).unwrap_err().code, "capacity-exceeded");
    f.store
        .connection()
        .unwrap()
        .execute("UPDATE tasks SET state='ready' WHERE id='fixture-1'", [])
        .unwrap();
    // Free one retained slot: active capacity independently still blocks it.
    f.store
        .connection()
        .unwrap()
        .execute(
            "DELETE FROM tasks WHERE id=?1",
            [format!("fixture-{}", MAX_RETAINED_TASKS - 1)],
        )
        .unwrap();
    assert_eq!(f.run(&request).unwrap_err().code, "capacity-exceeded");
    // Retained and active bounds both have capacity after this terminal transition.
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE tasks SET state='completed' WHERE id='fixture-1'",
            [],
        )
        .unwrap();
    assert!(f.run(&request).is_ok());
}
