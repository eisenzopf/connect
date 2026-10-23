use super::fixture::Fixture;
use crate::Store;
use connect_protocol::{tasks::*, Limits, Operation};

fn bounded(limit: u64) -> Fixture {
    let mut f = Fixture::new();
    f.store.limits.journal_events = limit;
    f.store.limits.operation_receipts = limit;
    f
}
fn cancel(task: &TaskRecord, id: &str) -> TaskRequest {
    TaskRequest::new(TaskCommand::Cancel {
        request_id: id.into(),
        task_id: task.id.clone(),
        reason: Some("Owner stopped the task".into()),
    })
}
fn send(f: &Fixture) -> Operation {
    Operation::Send {
        request_id: "ordinary-send".into(),
        conversation_id: f.cid.clone(),
        message: connect_protocol::messages::MessageDraft::text_all(
            "Must not spend the cancellation reserve",
        ),
    }
}
fn counts(f: &Fixture) -> (u64, u64) {
    f.store
        .connection()
        .unwrap()
        .query_row(
            "SELECT (SELECT COUNT(*) FROM journal)+(SELECT COUNT(*) FROM task_events),
                (SELECT COUNT(*) FROM operations)+(SELECT COUNT(*) FROM task_requests)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
}

#[test]
fn ordinary_work_cannot_spend_the_last_cancel_slot_and_matching_cancel_replays_at_capacity() {
    let f = bounded(3);
    let task = f.task();
    assert_eq!(counts(&f), (2, 2));
    let before = Store::inspect_snapshot(&f._dir.path().join("state.sqlite3")).unwrap();
    assert_eq!(
        f.store.execute(&f.key.id, &send(&f)).unwrap_err().code,
        "capacity-exceeded"
    );
    assert_eq!(
        f.run(&Fixture::transition(&task, "run", TaskState::Running))
            .unwrap_err()
            .code,
        "capacity-exceeded"
    );
    assert_eq!(
        Store::inspect_snapshot(&f._dir.path().join("state.sqlite3")).unwrap(),
        before
    );
    let request = cancel(&task, "cancel");
    let result = f.run(&request).unwrap();
    assert_eq!(Fixture::record(result.clone()).state, TaskState::Cancelled);
    assert_eq!(counts(&f), (3, 3));
    assert_eq!(f.run(&request).unwrap(), result);
    assert_eq!(counts(&f), (3, 3));
    let backup = f._dir.path().join("cancelled.sqlite3");
    f.store.backup(&backup).unwrap();
    let restored = Store::open(
        &backup,
        Limits {
            journal_events: 3,
            operation_receipts: 3,
            ..Limits::default()
        },
    )
    .unwrap();
    assert_eq!(restored.execute_task(&f.key.id, &request).unwrap(), result);
    let get = TaskRequest::new(TaskCommand::Get { task_id: task.id });
    assert_eq!(restored.execute_task(&f.key.id, &get).unwrap(), result);
}

#[test]
fn concurrent_cancellations_each_keep_their_own_reserved_records() {
    let f = bounded(5);
    let one = f.task();
    let mut create = f.create();
    let TaskCommand::Create { request_id, .. } = &mut create.command else {
        panic!("create")
    };
    *request_id = "second-task".into();
    let two = Fixture::record(f.run(&create).unwrap());
    assert_eq!(counts(&f), (3, 3));
    let other = Store::open(&f._dir.path().join("state.sqlite3"), f.store.limits.clone()).unwrap();
    let barrier = std::sync::Barrier::new(2);
    let (a, b) = std::thread::scope(|s| {
        let a = s.spawn(|| {
            barrier.wait();
            f.run(&cancel(&one, "cancel-one"))
        });
        let b = s.spawn(|| {
            barrier.wait();
            other.execute_task(&f.key.id, &cancel(&two, "cancel-two"))
        });
        (a.join().unwrap(), b.join().unwrap())
    });
    assert_eq!(Fixture::record(a.unwrap()).state, TaskState::Cancelled);
    assert_eq!(Fixture::record(b.unwrap()).state, TaskState::Cancelled);
    assert_eq!(counts(&f), (5, 5));
}

#[test]
fn unobserved_expiry_keeps_reservation_until_its_durable_terminal_event() {
    let f = bounded(3);
    let task = f.task();
    let transition = Fixture::transition(&task, "late-run", TaskState::Running);
    assert_eq!(
        f.store
            .execute_task_at(&f.key.id, &transition, task.expires_at)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(counts(&f), (2, 2));
    assert_eq!(
        f.store.execute(&f.key.id, &send(&f)).unwrap_err().code,
        "capacity-exceeded"
    );
    let get = TaskRequest::new(TaskCommand::Get { task_id: task.id });
    let expired = f
        .store
        .execute_task_at(&f.key.id, &get, task.expires_at)
        .unwrap();
    assert_eq!(Fixture::record(expired.clone()).state, TaskState::Expired);
    assert_eq!(counts(&f), (3, 2));
    assert_eq!(
        f.store
            .execute_task_at(&f.key.id, &get, task.expires_at + 1)
            .unwrap(),
        expired
    );
    assert_eq!(counts(&f), (3, 2));
}

#[test]
fn terminal_transition_receipt_is_retained_but_cannot_authorize_new_work() {
    let f = bounded(3);
    let task = f.task();
    let fail = Fixture::transition(&task, "fail", TaskState::Failed);
    let failed = f.run(&fail).unwrap();
    assert_eq!(Fixture::record(failed.clone()).state, TaskState::Failed);
    assert_eq!(counts(&f), (3, 3));
    assert_eq!(f.run(&fail).unwrap(), failed);
    assert_eq!(
        f.run(&Fixture::transition(&task, "new-run", TaskState::Running))
            .unwrap_err()
            .code,
        "forbidden"
    );
    let actor = f.store.authenticate(&f.key.token).unwrap();
    f.store.revoke(&actor, &f.key.id).unwrap();
    assert_eq!(f.run(&fail).unwrap_err().code, "unauthorized");
    let replacement = f
        .store
        .issue(&f.owner, crate::Scope::operator(), f.now + 600)
        .unwrap();
    assert_eq!(
        f.store.execute_task(&replacement.id, &fail).unwrap(),
        failed
    );
    assert_eq!(counts(&f), (3, 3));
}

#[test]
fn failed_cancellation_does_not_ack_or_consume_reservation_and_same_id_can_retry() {
    let f = bounded(3);
    let task = f.task();
    let grant = super::grants::grant(&f, &task);
    let request = cancel(&task, "cancel");
    let before = Store::inspect_snapshot(&f._dir.path().join("state.sqlite3")).unwrap();
    let c = f.store.connection().unwrap();
    c.execute_batch("CREATE TRIGGER fail_cancel_receipt BEFORE INSERT ON task_requests WHEN NEW.id='cancel' BEGIN SELECT RAISE(ABORT,'isolated write failure'); END;").unwrap();
    drop(c);
    assert_eq!(f.run(&request).unwrap_err().code, "storage-unavailable");
    assert_eq!(
        Store::inspect_snapshot(&f._dir.path().join("state.sqlite3")).unwrap(),
        before
    );
    assert_eq!(counts(&f), (2, 2));
    assert!(f
        .store
        .authenticate_task_grant(grant.expose_secret(), &grant.grant.binding)
        .is_ok());
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_cancel_receipt;")
        .unwrap();
    assert_eq!(
        Fixture::record(f.run(&request).unwrap()).state,
        TaskState::Cancelled
    );
    assert_eq!(counts(&f), (3, 3));
    assert_eq!(
        f.store
            .authenticate_task_grant(grant.expose_secret(), &grant.grant.binding)
            .unwrap_err()
            .code,
        "forbidden"
    );
}

#[test]
fn admission_without_terminal_room_rolls_back_task_and_creation_receipt() {
    let f = bounded(2);
    let before = Store::inspect_snapshot(&f._dir.path().join("state.sqlite3")).unwrap();
    assert_eq!(f.run(&f.create()).unwrap_err().code, "capacity-exceeded");
    assert_eq!(
        Store::inspect_snapshot(&f._dir.path().join("state.sqlite3")).unwrap(),
        before
    );
    assert_eq!(counts(&f), (1, 1));
}

#[test]
fn lowering_configured_capacity_cannot_falsely_ack_cancellation_and_recovery_keeps_id() {
    let mut f = bounded(3);
    let task = f.task();
    let request = cancel(&task, "cancel-after-config-recovery");
    f.store.limits.journal_events = 2;
    let before = Store::inspect_snapshot(&f._dir.path().join("state.sqlite3")).unwrap();
    assert_eq!(f.run(&request).unwrap_err().code, "capacity-exceeded");
    assert_eq!(
        Store::inspect_snapshot(&f._dir.path().join("state.sqlite3")).unwrap(),
        before
    );
    f.store.limits.journal_events = 3;
    assert_eq!(
        Fixture::record(f.run(&request).unwrap()).state,
        TaskState::Cancelled
    );
    assert_eq!(counts(&f), (3, 3));
}
