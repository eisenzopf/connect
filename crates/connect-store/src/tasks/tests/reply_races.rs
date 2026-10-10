use super::{
    fixture::Fixture,
    replies::{assign, incoming, setup, task},
};
use crate::Store;
use connect_protocol::{tasks::*, Limits};

#[test]
fn concurrent_assignment_to_two_tasks_commits_exactly_one_route() {
    let (f, key, human) = setup();
    let one = task(&f, &human, "one");
    let two = task(&f, &human, "two");
    let event = incoming(&f, &key, "reply");
    let other = Store::open(&f._dir.path().join("state.sqlite3"), Limits::default()).unwrap();
    let barrier = std::sync::Barrier::new(2);
    let result = std::thread::scope(|scope| {
        let one = scope.spawn(|| {
            barrier.wait();
            f.store
                .execute_task(&f.key.id, &assign(&one, &event, "assign-one"))
        });
        let two = scope.spawn(|| {
            barrier.wait();
            other.execute_task(&f.key.id, &assign(&two, &event, "assign-two"))
        });
        (one.join().unwrap(), two.join().unwrap())
    });
    assert_eq!(u8::from(result.0.is_ok()) + u8::from(result.1.is_ok()), 1);
    let c = f.store.connection().unwrap();
    let count: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM task_replies WHERE event_id=?1 AND task_id IS NOT NULL",
            [&event],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    let count: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM task_events WHERE json_extract(data_json,'$.kind')='reply'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn concurrent_consumers_cannot_ack_the_same_reply_twice() {
    let (f, key, human) = setup();
    let task = task(&f, &human, "task");
    let event = incoming(&f, &key, "reply");
    let task = Fixture::record(f.run(&assign(&task, &event, "assign")).unwrap());
    let other = Store::open(&f._dir.path().join("state.sqlite3"), Limits::default()).unwrap();
    let barrier = std::sync::Barrier::new(2);
    let consume = |id: &str| {
        TaskRequest::new(TaskCommand::ConsumeReply {
            request_id: id.into(),
            task_id: task.id.clone(),
            event_id: event.clone(),
            expected_revision: task.revision,
        })
    };
    let result = std::thread::scope(|scope| {
        let one = scope.spawn(|| {
            barrier.wait();
            f.store.execute_task(&f.key.id, &consume("consumer-one"))
        });
        let two = scope.spawn(|| {
            barrier.wait();
            other.execute_task(&f.key.id, &consume("consumer-two"))
        });
        (one.join().unwrap(), two.join().unwrap())
    });
    assert_eq!(u8::from(result.0.is_ok()) + u8::from(result.1.is_ok()), 1);
    let error = match (result.0, result.1) {
        (Err(error), Ok(_)) | (Ok(_), Err(error)) => error,
        _ => panic!("Exactly one consumer must commit"),
    };
    assert_eq!(error.code, "task-state-conflict");
}
