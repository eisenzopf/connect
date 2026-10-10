use super::{effect_helpers::*, fixture::Fixture, grants, replies};
use crate::Scope;
use connect_protocol::{tasks::*, Outcome};
use rusqlite::params;

#[test]
fn real_task_question_waits_for_its_own_reply_and_resumes_after_restore() {
    let (f, human, identity) = replies::setup();
    let one = replies::task(&f, &identity, "one");
    let two = replies::task(&f, &identity, "two");
    let ask = send(&f, &one, "ask", "Please review this draft", true);
    assert!(matches!(
        f.run(&ask).unwrap(),
        TaskOutcome::Tool(TaskToolResult::Pending {
            effect_id: Some(_),
            ..
        })
    ));
    let question = effect(&f, &one);
    assert_eq!(question.state, EffectState::Succeeded);
    let Outcome::Accepted { message_id, .. } = question.outcome.unwrap().as_ref().clone() else {
        panic!("question")
    };
    let waiting = Fixture::record(f.run(&grants::get(&one)).unwrap());
    assert_eq!(waiting.state, TaskState::Waiting);
    let unrelated = replies::incoming(&f, &human, "unrelated");
    f.run(&replies::assign(&two, &unrelated, "assign")).unwrap();
    assert!(matches!(
        f.run(&ask).unwrap(),
        TaskOutcome::Tool(TaskToolResult::Pending { .. })
    ));
    let reply = TaskRequest::new(TaskCommand::Reply {
        request_id: "answer".into(),
        conversation_id: f.cid.clone(),
        reply_to: message_id,
        body: "Use version B".into(),
    });
    let accepted = f.store.execute_task(&human.id, &reply).unwrap();
    assert!(
        matches!(&accepted,TaskOutcome::ReplyAccepted {reply,..} if reply.task_id.as_deref()==Some(one.id.as_str()))
    );
    assert_eq!(f.store.execute_task(&human.id, &reply).unwrap(), accepted);
    assert_eq!(
        Fixture::record(f.run(&grants::get(&one)).unwrap()).state,
        TaskState::Ready
    );
    let backup = f._dir.path().join("reply.sqlite3");
    f.store.backup(&backup).unwrap();
    let restored = crate::Store::open(&backup, connect_protocol::Limits::default()).unwrap();
    assert!(matches!(
        restored.execute_task(&f.key.id, &ask).unwrap(),
        TaskOutcome::Tool(TaskToolResult::Completed { .. })
    ));
    assert_eq!(count(&f, "messages"), 3); // question, unrelated incoming, bound answer
}

#[test]
fn another_task_can_answer_the_verified_initiating_question_under_its_own_grant() {
    let (f, human, identity) = replies::setup();
    let a = replies::task(&f, &identity, "a");
    let ask = send(&f, &a, "ask", "What does your app think?", true);
    f.run(&ask).unwrap();
    let Outcome::Accepted { message_id, .. } = effect(&f, &a).outcome.unwrap().as_ref().clone()
    else {
        panic!("question")
    };
    let event: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT id FROM journal WHERE message_id=?1 AND recipient=?2",
            params![message_id, identity],
            |r| r.get(0),
        )
        .unwrap();
    let owner = f
        .store
        .issue(&identity, Scope::operator(), f.now + 600)
        .unwrap();
    let b = Fixture::record(
        f.store
            .execute_task(
                &owner.id,
                &TaskRequest::new(TaskCommand::Create {
                    request_id: "b".into(),
                    definition: TaskDefinition {
                        conversation_id: f.cid.clone(),
                        scope: TaskScope {
                            sms_enrollments: Vec::new(),
                            operations: vec![TaskPermission::Send],
                            recipients: vec![f.owner.clone()],
                        },
                        expires_at: f.now + 300,
                        initiating_event_id: Some(event.clone()),
                    },
                }),
            )
            .unwrap(),
    );
    let origin = grants::binding("other-agent", InvocationKind::Native);
    f.store
        .register_task_binding(&owner.id, &b.id, &origin)
        .unwrap();
    let grant = f
        .store
        .issue_task_grant(
            &owner.id,
            "grant-b",
            &grants::definition(&b, origin.clone(), f.now),
        )
        .unwrap();
    let invocation = f
        .store
        .authenticate_task_grant(grant.expose_secret(), &origin)
        .unwrap();
    let mut response = send(&f, &b, "answer", "Approve the draft", false);
    let TaskCommand::Invoke { reply_to, .. } = &mut response.command else {
        panic!("invoke")
    };
    *reply_to = Some(event);
    let first = f
        .store
        .execute_task_invocation(&invocation, &response)
        .unwrap();
    assert_eq!(
        f.store
            .execute_task_invocation(&invocation, &response)
            .unwrap(),
        first
    );
    assert!(matches!(
        f.run(&ask).unwrap(),
        TaskOutcome::Tool(TaskToolResult::Completed { .. })
    ));
    let reply_task: String = f
        .store
        .connection()
        .unwrap()
        .query_row("SELECT task_id FROM task_replies", [], |r| r.get(0))
        .unwrap();
    assert_eq!(reply_task, a.id);
    assert_eq!(count(&f, "messages"), 2);
    assert_eq!(count(&f, "task_effects"), 2);
    // The ordinary send-only human credential still cannot administer either task.
    assert_eq!(
        f.store
            .execute_task(&human.id, &grants::get(&a))
            .unwrap_err()
            .code,
        "forbidden"
    );
}

#[test]
fn arbitrary_received_event_cannot_be_used_as_a_task_reply_binding() {
    let (f, human, identity) = replies::setup();
    let a = replies::task(&f, &identity, "a");
    let event = replies::incoming(&f, &human, "received");
    let mut response = send(&f, &a, "answer", "Unbound source", false);
    let TaskCommand::Invoke { reply_to, .. } = &mut response.command else {
        panic!("invoke")
    };
    *reply_to = Some(event);
    let before = crate::Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(f.run(&response).unwrap_err().code, "forbidden");
    assert_eq!(
        crate::Store::inspect_snapshot(f.store.path()).unwrap(),
        before
    );
}
