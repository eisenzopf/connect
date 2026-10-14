//! Normalized observations retain peer communication without resuming terminal tasks.
use super::{effect_helpers::*, effect_providers::fixture, fixture::Fixture, grants, replies};
use crate::ProviderJob;
use connect_connector_api::{host_context::HostContext, lifecycle::*};
use connect_protocol::tasks::*;

fn receipt(job: &ProviderJob) -> MessageReceipt {
    let selected = HostContext::decode(
        job.context.as_deref().expect("selected task history"),
        1024 * 1024,
    )
    .unwrap()
    .messaging_items()
    .unwrap();
    assert!(!selected.is_empty());
    MessageReceipt {
        observation: Observation {
            mutation_id: job.id.clone(),
            external_id: Some(format!("remote-{}", job.id)),
            event_id: format!("reply:{}", job.id),
            state: State::Message(MessageState::Delivered),
            verification: Verification::AuthenticatedResponse,
        },
        reply: Some("Late source-backed answer".into()),
        reply_sources: selected.into_iter().map(|item| item.event_id).collect(),
    }
}
fn assert_unassigned(
    f: &Fixture,
    task: &TaskRecord,
    expected: TaskState,
    job: &ProviderJob,
    reply: &MessageReceipt,
) {
    assert_eq!(
        f.store.apply_connector_receipt("vapi", reply).unwrap(),
        Apply::Applied
    );
    assert_eq!(
        f.store.apply_connector_receipt("vapi", reply).unwrap(),
        Apply::Duplicate
    );
    assert_eq!(f.store.connector_receipt(&job.id, "vapi").unwrap(), *reply);
    assert_eq!(
        Fixture::record(f.run(&grants::get(task)).unwrap()).state,
        expected
    );
    let TaskOutcome::Replies { items, .. } = f
        .run(&TaskRequest::new(TaskCommand::Replies {
            conversation_id: f.cid.clone(),
            after: None,
            limit: 100,
        }))
        .unwrap()
    else {
        panic!("ordinary communication replies");
    };
    assert_eq!(items.len(), 1);
    assert!(items[0].task_id.is_none() && items[0].reply_to.is_some());
    let TaskOutcome::Inbox { events, .. } = f.run(&replies::inbox(task, None)).unwrap() else {
        panic!("retained task inbox");
    };
    assert!(events
        .iter()
        .all(|event| !matches!(event.data, TaskEventData::Reply(_))));
    assert_eq!(count(f, "messages"), 2);
    assert_eq!(count(f, "task_effects"), 1);
    assert_eq!(count(f, "provider_jobs"), 1);
    assert_eq!(count(f, "connector_receipt_events"), 1);
    // A reordered acceptance cannot resume the task, regress delivery, repeat
    // the message, or remove the retained answer/provenance.
    let mut older = reply.clone();
    older.observation.event_id = "late-acceptance".into();
    older.observation.state = State::Message(MessageState::Accepted);
    older.observation.verification = Verification::VerifiedCallback;
    older.reply = None;
    older.reply_sources.clear();
    assert_eq!(
        f.store.apply_connector_receipt("vapi", &older).unwrap(),
        Apply::Older
    );
    assert_eq!(f.store.connector_receipt(&job.id, "vapi").unwrap(), *reply);
    assert_eq!(count(f, "messages"), 2);
    assert_eq!(count(f, "task_effects"), 1);
}
#[test]
fn normalized_cancelled_task_answer_is_unassigned_once_with_original_sources() {
    let (f, _, task) = fixture();
    f.run(&send(&f, &task, "question", "Question", true))
        .unwrap();
    let job = f.store.claim_provider_job().unwrap().unwrap();
    let reply = receipt(&job);
    f.run(&TaskRequest::new(TaskCommand::Cancel {
        request_id: "cancel-after-dispatch".into(),
        task_id: task.id.clone(),
        reason: None,
    }))
    .unwrap();
    assert_unassigned(&f, &task, TaskState::Cancelled, &job, &reply);
}
#[test]
fn normalized_expired_task_answer_is_unassigned_without_new_effect_or_resume() {
    let (f, _, task) = fixture();
    f.run(&send(&f, &task, "question", "Question", true))
        .unwrap();
    let job = f.store.claim_provider_job().unwrap().unwrap();
    let reply = receipt(&job);
    let expired = f
        .store
        .execute_task_at(&f.key.id, &grants::get(&task), f.now + 601)
        .unwrap();
    assert_eq!(Fixture::record(expired).state, TaskState::Expired);
    assert_unassigned(&f, &task, TaskState::Expired, &job, &reply);
}
#[test]
fn terminal_task_receipt_still_requires_current_route_credential_before_replay() {
    let (f, peer, task) = fixture();
    f.run(&send(&f, &task, "question", "Question", true))
        .unwrap();
    let job = f.store.claim_provider_job().unwrap().unwrap();
    let reply = receipt(&job);
    f.run(&TaskRequest::new(TaskCommand::Cancel {
        request_id: "cancel-after-dispatch".into(),
        task_id: task.id.clone(),
        reason: None,
    }))
    .unwrap();
    assert_unassigned(&f, &task, TaskState::Cancelled, &job, &reply);
    let authority = f.store.authenticate(&f.key.token).unwrap();
    f.store.revoke(&authority, &peer.id).unwrap();
    assert!(f.store.apply_connector_receipt("vapi", &reply).is_err());
    assert!(f
        .store
        .connector_reconciliation_job(&job.id, "vapi")
        .is_err());
    assert_eq!(count(&f, "messages"), 2);
    assert_eq!(count(&f, "task_effects"), 1);
}
#[test]
fn terminal_task_disabled_route_cannot_admit_a_first_late_answer() {
    let (f, _, task) = fixture();
    f.run(&send(&f, &task, "question", "Question", true))
        .unwrap();
    let job = f.store.claim_provider_job().unwrap().unwrap();
    let reply = receipt(&job);
    f.run(&TaskRequest::new(TaskCommand::Cancel {
        request_id: "cancel-after-dispatch".into(),
        task_id: task.id.clone(),
        reason: None,
    }))
    .unwrap();
    let mut route = job.route.clone();
    route.enabled = false;
    f.store.configure_route(&f.key.id, &route).unwrap();
    assert!(f.store.apply_connector_receipt("vapi", &reply).is_err());
    assert!(f
        .store
        .connector_reconciliation_job(&job.id, "vapi")
        .is_err());
    assert_eq!(count(&f, "messages"), 1);
    assert_eq!(count(&f, "task_effects"), 1);
    assert_eq!(count(&f, "connector_observations"), 0);
}

fn sms_receipt(
    job: &ProviderJob,
    event: &str,
    state: MessageState,
    verification: Verification,
) -> MessageReceipt {
    MessageReceipt {
        observation: Observation {
            mutation_id: job.id.clone(),
            external_id: Some("sms-provider-message".into()),
            event_id: event.into(),
            state: State::Message(state),
            verification,
        },
        reply: None,
        reply_sources: vec![],
    }
}
#[test]
fn normalized_sms_final_receipt_after_stop_does_not_authorize_send_or_resurrect_policy() {
    use super::sms_fixture::*;
    let (f, peer) = setup();
    let task = Fixture::record(
        f.run(&create(&f, &peer, "sms-task", vec![reference()]))
            .unwrap(),
    );
    f.run(&send(&f, &task, "sms-question", "Requested SMS", false))
        .unwrap();
    let job = f.store.claim_provider_job().unwrap().unwrap();
    let accepted = sms_receipt(
        &job,
        "accepted",
        MessageState::Accepted,
        Verification::AuthenticatedResponse,
    );
    assert_eq!(
        f.store
            .apply_connector_receipt("telnyx", &accepted)
            .unwrap(),
        Apply::Applied
    );
    f.store
        .receive_sms_control("stop", "stop-digest", PHONE, SENDER, "STOP", f.now)
        .unwrap();
    let delivered = sms_receipt(
        &job,
        "delivered",
        MessageState::Delivered,
        Verification::VerifiedCallback,
    );
    assert_eq!(
        f.store
            .apply_connector_receipt("telnyx", &delivered)
            .unwrap(),
        Apply::Applied
    );
    assert_eq!(
        f.store
            .apply_connector_receipt("telnyx", &delivered)
            .unwrap(),
        Apply::Duplicate
    );
    assert_eq!(count(&f, "messages"), 1);
    assert_eq!(count(&f, "task_effects"), 1);
    let stopped: bool = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT stopped FROM sms_controls WHERE phone=?1",
            [PHONE],
            |row| row.get(0),
        )
        .unwrap();
    assert!(stopped);
    // Reply-bearing observations remain subject to the current SMS policy.
    let mut response = delivered.clone();
    response.observation.event_id = "forbidden-reply".into();
    response.reply = Some("Must not re-enter messaging after STOP".into());
    let before = crate::Store::inspect_snapshot(f.store.path()).unwrap();
    assert!(f
        .store
        .apply_connector_receipt("telnyx", &response)
        .is_err());
    assert_eq!(
        crate::Store::inspect_snapshot(f.store.path()).unwrap(),
        before
    );
    f.run(&send(
        &f,
        &task,
        "stopped-next",
        "A new native message",
        false,
    ))
    .unwrap();
    assert!(f.store.claim_provider_job().unwrap().is_none());
    let latest = decision(&f);
    assert_eq!(latest.0, "cancelled");
    assert_eq!(latest.1.as_deref(), Some("opted-out"));
    assert_eq!(
        f.store.connector_receipt(&job.id, "telnyx").unwrap(),
        delivered
    );
}
#[test]
fn normalized_sms_status_after_lease_expiry_retains_effect_without_refresh_or_new_send() {
    use super::sms_fixture::*;
    let (f, peer) = setup();
    let task = Fixture::record(
        f.run(&create(&f, &peer, "sms-task", vec![reference()]))
            .unwrap(),
    );
    f.run(&send(&f, &task, "sms-question", "Requested SMS", false))
        .unwrap();
    let job = f.store.claim_provider_job().unwrap().unwrap();
    let uncertain = message_attempt(&job.id, Err(connect_connector_api::Failure::Uncertain));
    assert_eq!(
        f.store
            .apply_connector_receipt("telnyx", &uncertain)
            .unwrap(),
        Apply::Applied
    );
    f.store.expire_sms_authorities().unwrap();
    assert_eq!(
        f.store
            .connector_reconciliation_job(&job.id, "telnyx")
            .unwrap()
            .id,
        job.id
    );
    let delivered = sms_receipt(
        &job,
        "status-delivered",
        MessageState::Delivered,
        Verification::Reconciliation,
    );
    assert_eq!(
        f.store
            .apply_connector_receipt("telnyx", &delivered)
            .unwrap(),
        Apply::Applied
    );
    assert_eq!(
        f.store
            .apply_connector_receipt("telnyx", &delivered)
            .unwrap(),
        Apply::Duplicate
    );
    assert_eq!(count(&f, "messages"), 1);
    assert_eq!(count(&f, "task_effects"), 1);
    let valid_until: i64 = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT valid_until FROM sms_authorities WHERE source='reviewed'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(valid_until, 0);
    f.run(&send(
        &f,
        &task,
        "stale-next",
        "A new native message",
        false,
    ))
    .unwrap();
    assert!(f.store.claim_provider_job().unwrap().is_none());
    assert_eq!(decision(&f).0, "cancelled");
    assert_eq!(
        decision(&f).1.as_deref(),
        Some("task-sms-enrollment-unavailable")
    );
}
