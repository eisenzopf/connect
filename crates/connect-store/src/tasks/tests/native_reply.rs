use super::{
    fixture::Fixture,
    replies::{setup, task},
};
use connect_protocol::{tasks::*, Operation, Outcome};

/// Seeds the accepted effect boundary that G4-010 will produce. This fixture
/// exercises reply admission, not a claim that task Invoke is implemented.
fn question(f: &Fixture, task: &TaskRecord, id: &str) -> String {
    let operation = Operation::Send {
        request_id: id.into(),
        conversation_id: f.cid.clone(),
        body: format!("question {id}"),
    };
    let outcome = f.store.execute(&f.key.id, &operation).unwrap();
    let Outcome::Accepted { message_id, .. } = &outcome else {
        panic!("question")
    };
    f.store.connection().unwrap().execute("INSERT INTO task_effects(id,task_id,step_id,request_id,digest,operation_json,state,outcome_json,created_at,updated_at) VALUES(?1,?2,'question',?3,'fixture',?4,'succeeded',?5,?6,?6)",rusqlite::params![format!("effect-{id}"),task.id,id,serde_json::to_string(&operation).unwrap(),serde_json::to_string(&outcome).unwrap(),f.now]).unwrap();
    message_id.clone()
}
fn reply(f: &Fixture, question: &str, id: &str) -> TaskRequest {
    TaskRequest::new(TaskCommand::Reply {
        request_id: id.into(),
        conversation_id: f.cid.clone(),
        reply_to: question.into(),
        body: format!("answer {id}"),
    })
}

#[test]
fn sender_send_authority_answers_the_exact_task_without_owner_credentials() {
    let (f, key, human) = setup();
    let one = task(&f, &human, "one");
    let two = task(&f, &human, "two");
    let a = question(&f, &one, "a");
    let b = question(&f, &two, "b");
    let request = reply(&f, &b, "answer");
    let accepted = f.store.execute_task(&key.id, &request).unwrap();
    assert!(
        matches!(&accepted,TaskOutcome::ReplyAccepted {reply,..} if reply.task_id.as_ref()==Some(&two.id) && reply.reply_to.as_ref()==Some(&b) && reply.assigned_by.is_none())
    );
    assert_eq!(f.store.execute_task(&key.id, &request).unwrap(), accepted);
    let inspect = TaskRequest::new(TaskCommand::Get {
        task_id: two.id.clone(),
    });
    assert_eq!(
        f.store.execute_task(&key.id, &inspect).unwrap_err().code,
        "forbidden"
    );
    let TaskOutcome::Inbox { events, .. } = f.run(&super::replies::inbox(&one, None)).unwrap()
    else {
        panic!("inbox")
    };
    assert!(!events
        .iter()
        .any(|e| matches!(e.data, TaskEventData::Reply(_))));
    let TaskOutcome::Inbox { events, .. } = f.run(&super::replies::inbox(&two, None)).unwrap()
    else {
        panic!("inbox")
    };
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(&e.data,TaskEventData::Reply(r) if r.reply_to.as_ref()==Some(&b)))
            .count(),
        1
    );
    assert!(f
        .store
        .execute_task(&key.id, &reply(&f, &a, "second-task"))
        .is_ok());
}

#[test]
fn unknown_wrong_conversation_undelivered_and_ambiguous_questions_do_not_mutate() {
    let (f, key, human) = setup();
    let one = task(&f, &human, "one");
    let a = question(&f, &one, "a");
    let before = crate::Store::inspect_snapshot(&f._dir.path().join("state.sqlite3")).unwrap();
    assert_eq!(
        f.store
            .execute_task(&key.id, &reply(&f, "unknown", "unknown"))
            .unwrap_err()
            .code,
        "forbidden"
    );
    let mut wrong = reply(&f, &a, "wrong");
    let TaskCommand::Reply {
        conversation_id, ..
    } = &mut wrong.command
    else {
        panic!("reply")
    };
    *conversation_id = "other".into();
    assert_eq!(
        f.store.execute_task(&key.id, &wrong).unwrap_err().code,
        "forbidden"
    );
    assert_eq!(
        crate::Store::inspect_snapshot(&f._dir.path().join("state.sqlite3")).unwrap(),
        before
    );
    let c = f.store.connection().unwrap();
    c.execute(
        "DELETE FROM journal WHERE message_id=?1 AND recipient=?2",
        rusqlite::params![a, human],
    )
    .unwrap();
    drop(c);
    assert_eq!(
        f.store
            .execute_task(&key.id, &reply(&f, &a, "undelivered"))
            .unwrap_err()
            .code,
        "forbidden"
    );
    crate::conversations::insert_event(
        &f.store.connection().unwrap(),
        &human,
        &f.cid,
        "message",
        Some(&a),
    )
    .unwrap();
    let two = task(&f, &human, "two");
    let c = f.store.connection().unwrap();
    c.execute("INSERT INTO task_effects SELECT 'ambiguous',?1,step_id,request_id,digest,operation_json,state,outcome_json,reply_to,created_at,updated_at FROM task_effects WHERE id='effect-a'",[&two.id]).unwrap();
    drop(c);
    assert_eq!(
        f.store
            .execute_task(&key.id, &reply(&f, &a, "ambiguous"))
            .unwrap_err()
            .code,
        "forbidden"
    );
}

#[test]
fn a_terminal_or_revoked_task_retains_new_reply_unassigned_without_reopening() {
    for revoke in [false, true] {
        let (f, key, human) = setup();
        let task = task(&f, &human, "task");
        let question = question(&f, &task, "question");
        if revoke {
            f.store
                .revoke(&f.store.authenticate(&f.key.token).unwrap(), &f.key.id)
                .unwrap();
        } else {
            f.run(&TaskRequest::new(TaskCommand::Cancel {
                request_id: "cancel".into(),
                task_id: task.id.clone(),
                reason: None,
            }))
            .unwrap();
        }
        let outcome = f
            .store
            .execute_task(&key.id, &reply(&f, &question, "late"))
            .unwrap();
        assert!(
            matches!(outcome,TaskOutcome::ReplyAccepted {reply,..} if reply.task_id.is_none() && reply.reply_to.as_ref()==Some(&question))
        );
        let c = f.store.connection().unwrap();
        let state: String = c
            .query_row("SELECT state FROM tasks WHERE id=?1", [&task.id], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(state, if revoke { "ready" } else { "cancelled" });
        let count: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM task_events WHERE json_extract(data_json,'$.kind')='reply'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }
}

#[test]
fn receipt_failure_rolls_back_message_reply_and_task_transition_then_retry_is_safe() {
    let (f, key, human) = setup();
    let task = task(&f, &human, "task");
    let question = question(&f, &task, "question");
    let running = Fixture::record(
        f.run(&Fixture::transition(&task, "run", TaskState::Running))
            .unwrap(),
    );
    f.run(&Fixture::transition(&running, "wait", TaskState::Waiting))
        .unwrap();
    let request = reply(&f, &question, "answer");
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER fail_reply BEFORE INSERT ON task_requests BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    let before = crate::Store::inspect_snapshot(&f._dir.path().join("state.sqlite3")).unwrap();
    assert_eq!(
        f.store.execute_task(&key.id, &request).unwrap_err().code,
        "storage-unavailable"
    );
    assert_eq!(
        crate::Store::inspect_snapshot(&f._dir.path().join("state.sqlite3")).unwrap(),
        before
    );
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_reply;")
        .unwrap();
    let accepted = f.store.execute_task(&key.id, &request).unwrap();
    assert_eq!(f.store.execute_task(&key.id, &request).unwrap(), accepted);
    let current = Fixture::record(
        f.run(&TaskRequest::new(TaskCommand::Get { task_id: task.id }))
            .unwrap(),
    );
    assert_eq!(current.state, TaskState::Ready);
}
