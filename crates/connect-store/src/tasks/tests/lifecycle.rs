use super::fixture::Fixture;
use crate::Store;
use connect_protocol::{tasks::*, Limits};

#[test]
fn lifecycle_receipts_survive_restart_without_reopening_terminal_work() {
    let f = Fixture::new();
    let task = f.task();
    assert_eq!(f.task(), task);
    let run = Fixture::transition(&task, "run", TaskState::Running);
    let running = Fixture::record(f.run(&run).unwrap());
    assert_eq!(running.revision, 1);
    assert_eq!(Fixture::record(f.run(&run).unwrap()), running);
    assert_eq!(
        f.run(&Fixture::transition(&task, "stale", TaskState::Running))
            .unwrap_err()
            .code,
        "task-state-conflict"
    );
    let wait = Fixture::record(
        f.run(&Fixture::transition(&running, "wait", TaskState::Waiting))
            .unwrap(),
    );
    let ready = Fixture::record(
        f.run(&Fixture::transition(&wait, "ready", TaskState::Ready))
            .unwrap(),
    );
    let running = Fixture::record(
        f.run(&Fixture::transition(
            &ready,
            "run-again",
            TaskState::Running,
        ))
        .unwrap(),
    );
    let finish = Fixture::transition(&running, "finish", TaskState::Completed);
    let completed = Fixture::record(f.run(&finish).unwrap());
    assert_eq!(completed.state, TaskState::Completed);
    assert_eq!(
        Fixture::record(f.run(&finish).unwrap()),
        completed,
        "Uncertain completion response can reconcile its receipt"
    );
    assert!(f
        .run(&Fixture::transition(&completed, "reopen", TaskState::Ready))
        .is_err());
    let backup = f._dir.path().join("backup.sqlite3");
    f.store.backup(&backup).unwrap();
    let restored = Store::open(&backup, Limits::default()).unwrap();
    assert_eq!(
        restored.execute_task(&f.key.id, &finish).unwrap(),
        TaskOutcome::Record(Box::new(completed))
    );
    let c = restored.connection().unwrap();
    let events: u64 = c
        .query_row("SELECT COUNT(*) FROM task_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(events, 6);
}
#[test]
fn cancelled_creation_replay_is_denied_but_owner_can_inspect_and_repeat_cancel() {
    let f = Fixture::new();
    let task = f.task();
    let cancel = TaskRequest::new(TaskCommand::Cancel {
        request_id: "cancel".into(),
        task_id: task.id.clone(),
        reason: Some("Owner withdrew delegation".into()),
    });
    let cancelled = f.run(&cancel).unwrap();
    assert_eq!(f.run(&cancel).unwrap(), cancelled);
    assert_eq!(f.run(&f.create()).unwrap_err().code, "forbidden");
    let get = TaskRequest::new(TaskCommand::Get { task_id: task.id });
    assert_eq!(f.run(&get).unwrap(), cancelled);
    let backup = f._dir.path().join("cancelled.sqlite3");
    f.store.backup(&backup).unwrap();
    let restored = Store::open(&backup, Limits::default()).unwrap();
    assert_eq!(restored.execute_task(&f.key.id, &get).unwrap(), cancelled);
    assert_eq!(
        restored.execute_task(&f.key.id, &cancel).unwrap(),
        cancelled
    );
}
#[test]
fn fake_clock_expiry_commits_at_boundary_and_remains_expired_after_restart() {
    let f = Fixture::new();
    let task = f.task();
    let get = TaskRequest::new(TaskCommand::Get {
        task_id: task.id.clone(),
    });
    let before = Fixture::record(
        f.store
            .execute_task_at(&f.key.id, &get, task.expires_at - 1)
            .unwrap(),
    );
    assert_eq!(before.state, TaskState::Ready);
    let expired = Fixture::record(
        f.store
            .execute_task_at(&f.key.id, &get, task.expires_at)
            .unwrap(),
    );
    assert_eq!(expired.state, TaskState::Expired);
    assert!(f
        .store
        .execute_task_at(
            &f.key.id,
            &Fixture::transition(&task, "expired-run", TaskState::Running),
            task.expires_at
        )
        .is_err());
    let backup = f._dir.path().join("expired.sqlite3");
    f.store.backup(&backup).unwrap();
    let restored = Store::open(&backup, Limits::default()).unwrap();
    assert_eq!(
        restored.execute_task(&f.key.id, &get).unwrap(),
        TaskOutcome::Record(Box::new(expired))
    );
}
#[test]
fn valid_origin_and_failed_terminal_state_are_retained() {
    let f = Fixture::new();
    let source: String = f
        .store
        .connection()
        .unwrap()
        .query_row("SELECT id FROM journal LIMIT 1", [], |r| r.get(0))
        .unwrap();
    let mut create = f.create();
    let TaskCommand::Create { definition, .. } = &mut create.command else {
        panic!("create")
    };
    definition.initiating_event_id = Some(source.clone());
    let task = Fixture::record(f.run(&create).unwrap());
    assert_eq!(task.initiating_event_id, Some(source));
    let fail = Fixture::transition(&task, "fail", TaskState::Failed);
    let failed = Fixture::record(f.run(&fail).unwrap());
    assert_eq!(failed.state, TaskState::Failed);
    assert!(f
        .run(&Fixture::transition(&failed, "retry", TaskState::Ready))
        .is_err());
    let backup = f._dir.path().join("failed.sqlite3");
    f.store.backup(&backup).unwrap();
    let restored = Store::open(&backup, Limits::default()).unwrap();
    assert_eq!(
        restored
            .execute_task(
                &f.key.id,
                &TaskRequest::new(TaskCommand::Get { task_id: task.id })
            )
            .unwrap(),
        TaskOutcome::Record(Box::new(failed))
    );
}
