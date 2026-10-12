use super::{effect_helpers::*, fixture::Fixture, grants, replies};
use crate::{IssuedCredential, Route};
use connect_protocol::{tasks::*, Outcome};

pub(super) fn fixture() -> (Fixture, IssuedCredential, TaskRecord) {
    let (f, human, identity) = replies::setup();
    f.store
        .configure_route(
            &f.key.id,
            &Route {
                id: "vapi".into(),
                identity_id: identity.clone(),
                credential_id: human.id.clone(),
                conversation_id: f.cid.clone(),
                connector: "vapi".into(),
                target: "12345678-1234-1234-1234-123456789012".into(),
                source: None,
                enabled: true,
            },
        )
        .unwrap();
    let task = replies::task(&f, &identity, "task");
    (f, human, task)
}

#[test]
fn send_only_task_cannot_inherit_owner_context_or_route_chat_history() {
    let (f, _, task) = fixture();
    f.store
        .execute(
            &f.key.id,
            &connect_protocol::Operation::Send {
                request_id: "ordinary".into(),
                conversation_id: f.cid.clone(),
                body: "Prior private conversation fact".into(),
            },
        )
        .unwrap();
    let seed = f.store.claim_provider_job().unwrap().unwrap();
    f.store
        .finish_provider_job(&seed, "completed", Some("ordinary-chat"), None)
        .unwrap();
    let handle = handle(&f, &task);
    f.store
        .execute_task_invocation(
            &handle,
            &send(&f, &task, "narrow", "Only this message", false),
        )
        .unwrap();
    let job = f.store.claim_provider_job().unwrap().unwrap();
    assert!(
        job.context.is_none(),
        "Send-only invocation disclosed backing-owner history"
    );
    assert!(
        job.previous_chat_id.is_none(),
        "Task inherited route-wide remote history"
    );
}

#[test]
fn completed_provider_reply_wakes_only_its_bound_task() {
    let (f, _, one) = fixture();
    let recipient = one.scope.recipients[0].clone();
    let two = replies::task(&f, &recipient, "second-task");
    f.run(&send(&f, &one, "first", "First question", true))
        .unwrap();
    f.run(&send(&f, &two, "second", "Second question", true))
        .unwrap();
    let first = f.store.claim_provider_job().unwrap().unwrap();
    let second = f.store.claim_provider_job().unwrap().unwrap();
    f.store
        .finish_provider_job(
            &second,
            "completed",
            Some("second-chat"),
            Some("Answer two"),
        )
        .unwrap();
    for (task, expected) in [(&one, 0), (&two, 1)] {
        let TaskOutcome::Inbox { events, .. } = f.run(&replies::inbox(task, None)).unwrap() else {
            panic!("inbox")
        };
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e.data, TaskEventData::Reply(_)))
                .count(),
            expected
        );
    }
    assert_eq!(
        Fixture::record(f.run(&grants::get(&one)).unwrap()).state,
        TaskState::Waiting
    );
    assert_eq!(
        Fixture::record(f.run(&grants::get(&two)).unwrap()).state,
        TaskState::Ready
    );
    f.store
        .finish_provider_job(&first, "completed", Some("first-chat"), Some("Answer one"))
        .unwrap();
    let before = crate::Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(
        f.store
            .finish_provider_job(&first, "completed", Some("first-chat"), Some("Answer one"))
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(
        crate::Store::inspect_snapshot(f.store.path()).unwrap(),
        before
    );
}
#[test]
fn queued_provider_effect_is_pending_and_only_recorded_completion_resolves_it() {
    let (f, _, task) = fixture();
    let a = send(&f, &task, "send", "A provider turn", false);
    assert!(matches!(
        f.run(&a).unwrap(),
        TaskOutcome::Tool(TaskToolResult::Pending { .. })
    ));
    assert_eq!(effect(&f, &task).state, EffectState::Dispatched);
    let job = f.store.claim_provider_job().unwrap().unwrap();
    assert!(matches!(
        f.run(&a).unwrap(),
        TaskOutcome::Tool(TaskToolResult::Uncertain { .. })
    ));
    f.store
        .finish_provider_job(&job, "completed", Some("chat-1"), Some("Received"))
        .unwrap();
    assert_eq!(effect(&f, &task).state, EffectState::Succeeded);
    assert!(matches!(
        f.run(&a).unwrap(),
        TaskOutcome::Tool(TaskToolResult::Completed { .. })
    ));
    assert!(f.store.claim_provider_job().unwrap().is_none());
    assert_eq!(count(&f, "provider_jobs"), 1);
}
#[test]
fn interrupted_provider_job_remains_uncertain_and_is_never_requeued_on_replay() {
    let (f, _, task) = fixture();
    let a = send(&f, &task, "send", "Uncertain", false);
    f.run(&a).unwrap();
    f.store.claim_provider_job().unwrap().unwrap();
    assert_eq!(f.store.recover_provider_jobs().unwrap(), 1);
    let first = f.run(&a).unwrap();
    assert!(matches!(
        &first,
        TaskOutcome::Tool(TaskToolResult::Uncertain { .. })
    ));
    let before = crate::Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(f.run(&a).unwrap(), first);
    assert_eq!(
        crate::Store::inspect_snapshot(f.store.path()).unwrap(),
        before
    );
    assert!(f.store.claim_provider_job().unwrap().is_none());
    assert_eq!(count(&f, "provider_jobs"), 1);
}
#[test]
fn task_grant_revocation_blocks_dispatch_even_while_account_authority_remains_valid() {
    let (f, _, task) = fixture();
    let handle = handle(&f, &task);
    let a = send(&f, &task, "send", "Must not leave", false);
    f.store.execute_task_invocation(&handle, &a).unwrap();
    f.store
        .revoke_task_grant(&f.key.id, handle.grant_id())
        .unwrap();
    assert!(
        crate::identity::actor_from_connection(&f.store.connection().unwrap(), &f.key.id).is_ok()
    );
    assert!(f.store.claim_provider_job().unwrap().is_none());
    let e = effect(&f, &task);
    assert_eq!(e.state, EffectState::Failed);
    assert_eq!(e.error.unwrap().code, "provider-failed");
    assert_eq!(
        f.store
            .execute_task_invocation(&handle, &a)
            .unwrap_err()
            .code,
        "forbidden"
    );
}
#[test]
fn cancellation_blocks_queued_dispatch_but_preserves_the_conversation_message() {
    let (f, _, task) = fixture();
    let a = send(&f, &task, "send", "Recorded locally", false);
    f.run(&a).unwrap();
    f.run(&TaskRequest::new(TaskCommand::Cancel {
        request_id: "cancel".into(),
        task_id: task.id.clone(),
        reason: None,
    }))
    .unwrap();
    assert!(f.store.claim_provider_job().unwrap().is_none());
    assert_eq!(effect(&f, &task).state, EffectState::Failed);
    let Outcome::History { messages, .. } = f
        .store
        .execute(
            &f.key.id,
            &connect_protocol::Operation::History {
                conversation_id: f.cid.clone(),
                after: None,
                limit: 10,
            },
        )
        .unwrap()
    else {
        panic!("history")
    };
    assert_eq!(messages.len(), 1);
    assert_eq!(
        Fixture::record(f.run(&grants::get(&task)).unwrap()).state,
        TaskState::Cancelled
    );
}
