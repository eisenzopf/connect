use super::{effect_helpers::*, fixture::Fixture};
use crate::{Limits, Store};
use connect_protocol::tasks::*;

#[test]
fn failure_after_ordinary_mutation_rolls_back_effect_receipt_message_and_state() {
    let f = Fixture::new();
    let task = f.task();
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER reject_effect BEFORE INSERT ON task_events WHEN json_extract(NEW.data_json,'$.data.state')='succeeded' BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let a = send(&f, &task, "retry", "only once", false);
    assert_eq!(f.run(&a).unwrap_err().code, "storage-unavailable");
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_effect")
        .unwrap();
    f.run(&a).unwrap();
    assert_eq!(count(&f, "messages"), 1);
    assert_eq!(count(&f, "task_effects"), 1);
}
#[test]
fn competing_database_connections_commit_one_effect_and_one_message() {
    let f = Fixture::new();
    let task = f.task();
    let request = send(&f, &task, "concurrent", "one", false);
    let start = std::sync::Arc::new(std::sync::Barrier::new(8));
    let workers: Vec<_> = (0..8)
        .map(|_| {
            let store = Store::open(f.store.path(), Limits::default()).unwrap();
            let start = start.clone();
            let key = f.key.id.clone();
            let request = request.clone();
            std::thread::spawn(move || {
                start.wait();
                store.execute_task(&key, &request)
            })
        })
        .collect();
    let results: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap().unwrap())
        .collect();
    assert!(results.iter().all(|result| result == &results[0]));
    assert_eq!(count(&f, "messages"), 1);
    assert_eq!(count(&f, "task_effects"), 1);
    assert_eq!(count(&f, "operation_control_receipts"), 2);
}
#[test]
fn completed_task_replays_known_effect_but_cannot_begin_another() {
    let f = Fixture::new();
    let task = f.task();
    let a = send(&f, &task, "original", "work", false);
    let saved = f.run(&a).unwrap();
    let task = Fixture::record(
        f.run(&TaskRequest::new(TaskCommand::Get { task_id: task.id }))
            .unwrap(),
    );
    f.run(&Fixture::transition(&task, "finish", TaskState::Completed))
        .unwrap();
    assert_eq!(f.run(&a).unwrap(), saved);
    assert_eq!(
        f.run(&send(&f, &task, "new", "new work", false))
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(count(&f, "messages"), 1);
}
