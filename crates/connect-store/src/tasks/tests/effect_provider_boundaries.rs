use super::{effect_helpers::*, effect_providers::fixture, fixture::Fixture, grants, replies};
use connect_protocol::{tasks::*, Operation, Outcome};

fn scoped_handle(f: &Fixture, task: &TaskRecord) -> crate::TaskInvocation {
    let binding = grants::binding("provider-context", InvocationKind::Native);
    f.store
        .register_task_binding(&f.key.id, &task.id, &binding)
        .unwrap();
    let grant = f
        .store
        .issue_task_grant(
            &f.key.id,
            "provider-context-grant",
            &GrantDefinition {
                task_id: task.id.clone(),
                binding: binding.clone(),
                operations: vec![TaskPermission::Send, TaskPermission::History],
                expires_at: f.now + 120,
            },
        )
        .unwrap();
    f.store
        .authenticate_task_grant(grant.expose_secret(), &binding)
        .unwrap()
}

#[test]
fn permitted_task_context_is_bounded_source_backed_and_does_not_inherit_remote_history() {
    let (f, _, task) = fixture();
    for index in 0..5 {
        f.store
            .execute(
                &f.key.id,
                &Operation::Send {
                    request_id: format!("seed-{index}"),
                    conversation_id: f.cid.clone(),
                    message: connect_protocol::messages::MessageDraft::text_all(format!(
                        "Fact {index}: {}",
                        "x".repeat(16_000)
                    )),
                },
            )
            .unwrap();
        let seed = f.store.claim_provider_job().unwrap().unwrap();
        f.store
            .finish_provider_job(&seed, "completed", Some(&format!("remote-{index}")), None)
            .unwrap();
    }
    let Outcome::Conversation(other) = f
        .store
        .execute(
            &f.key.id,
            &Operation::Create {
                request_id: "other-cid".into(),
                request: connect_protocol::conversations::CreateRequest::persistent(vec![]),
            },
        )
        .unwrap()
    else {
        panic!("conversation")
    };
    f.store
        .execute(
            &f.key.id,
            &Operation::Send {
                request_id: "other-secret".into(),
                conversation_id: other.id,
                message: connect_protocol::messages::MessageDraft::text_all(
                    "DO-NOT-DISCLOSE-OTHER-CONVERSATION",
                ),
            },
        )
        .unwrap();
    let handle = scoped_handle(&f, &task);
    f.store
        .execute_task_invocation(
            &handle,
            &send(&f, &task, "context", "Current question", true),
        )
        .unwrap();
    let job = f.store.claim_provider_job().unwrap().unwrap();
    assert!(job.previous_chat_id.is_none());
    let raw = job.context.unwrap();
    assert!(raw.len() <= TASK_CONTEXT_BYTES);
    assert!(!raw.contains("DO-NOT-DISCLOSE"));
    let connect_connector_api::host_context::TaskProjection::Recall {
        sources,
        next,
        omitted,
    } = serde_json::from_str(&raw).unwrap();
    assert!(!sources.is_empty() && next.is_none() && omitted.is_none());
    assert!(sources
        .iter()
        .all(|s| s.provenance.starts_with("conversation-journal:") && !s.source_id.is_empty()));
    assert!(sources[0].body.as_ref().unwrap().starts_with("Fact 0:"));
    let (selection, watermark): (String, u64) = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT selection_json,watermark FROM context_deliveries WHERE id=?1",
            [format!("provider-job:{}", job.id)],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(selection, raw);
    assert!(watermark > 0);
}

#[test]
fn owner_task_without_history_permission_also_avoids_backing_context() {
    let (f, _, base) = fixture();
    let task = Fixture::record(
        f.run(&TaskRequest::new(TaskCommand::Create {
            request_id: "send-only-owner".into(),
            definition: TaskDefinition {
                conversation_id: f.cid.clone(),
                scope: TaskScope {
                    operations: vec![TaskPermission::Send],
                    recipients: base.scope.recipients,
                    sms_enrollments: vec![],
                },
                expires_at: f.now + 600,
                initiating_event_id: None,
            },
        }))
        .unwrap(),
    );
    f.run(&send(&f, &task, "owner-send", "No history", false))
        .unwrap();
    let job = f.store.claim_provider_job().unwrap().unwrap();
    assert!(job.context.is_none() && job.previous_chat_id.is_none());
    assert_eq!(count(&f, "context_deliveries"), 0);
}

#[test]
fn cancelled_task_preserves_late_reply_as_unassigned_communication() {
    let (f, _, task) = fixture();
    f.run(&send(&f, &task, "question", "Question", true))
        .unwrap();
    let job = f.store.claim_provider_job().unwrap().unwrap();
    f.run(&TaskRequest::new(TaskCommand::Cancel {
        request_id: "cancel-after-dispatch".into(),
        task_id: task.id.clone(),
        reason: None,
    }))
    .unwrap();
    f.store
        .finish_provider_job(&job, "completed", Some("late"), Some("Late answer"))
        .unwrap();
    assert_eq!(
        Fixture::record(f.run(&grants::get(&task)).unwrap()).state,
        TaskState::Cancelled
    );
    let TaskOutcome::Replies { items, .. } = f
        .run(&TaskRequest::new(TaskCommand::Replies {
            conversation_id: f.cid.clone(),
            after: None,
            limit: 100,
        }))
        .unwrap()
    else {
        panic!("replies")
    };
    assert_eq!(items.len(), 1);
    assert!(items[0].task_id.is_none() && items[0].reply_to.is_some());
    let TaskOutcome::Inbox { events, .. } = f.run(&replies::inbox(&task, None)).unwrap() else {
        panic!("retained owner journal")
    };
    assert!(events.iter().any(|event| matches!(
        event.data,
        TaskEventData::State {
            state: TaskState::Cancelled,
            ..
        }
    )));
    assert!(events
        .iter()
        .all(|event| !matches!(event.data, TaskEventData::Reply(_))));
    let reply_events: u64 = f.store.connection().unwrap().query_row(
        "SELECT COUNT(*) FROM task_events WHERE task_id=?1 AND json_extract(data_json,'$.kind')='reply'", [&task.id], |r| r.get(0),
    ).unwrap();
    assert_eq!(reply_events, 0);
    assert_eq!(count(&f, "messages"), 2);
}

#[test]
fn revoked_invocation_does_not_erase_a_currently_authorized_incoming_reply() {
    let (f, _, task) = fixture();
    let handle = handle(&f, &task);
    f.store
        .execute_task_invocation(&handle, &send(&f, &task, "question", "Question", true))
        .unwrap();
    let job = f.store.claim_provider_job().unwrap().unwrap();
    f.store
        .revoke_task_grant(&f.key.id, handle.grant_id())
        .unwrap();
    f.store
        .finish_provider_job(&job, "completed", Some("answer"), Some("Answer"))
        .unwrap();
    assert_eq!(
        Fixture::record(f.run(&grants::get(&task)).unwrap()).state,
        TaskState::Ready
    );
    assert_eq!(
        f.store
            .execute_task_invocation(&handle, &grants::get(&task))
            .unwrap_err()
            .code,
        "forbidden"
    );
    let TaskOutcome::Inbox { events, .. } = f.run(&replies::inbox(&task, None)).unwrap() else {
        panic!("inbox")
    };
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e.data, TaskEventData::Reply(_)))
            .count(),
        1
    );
}

#[test]
fn failure_after_reply_insertion_rolls_back_message_task_and_provider_completion() {
    let (f, _, task) = fixture();
    f.run(&send(&f, &task, "question", "Question", true))
        .unwrap();
    let job = f.store.claim_provider_job().unwrap().unwrap();
    let c = f.store.connection().unwrap();
    c.execute_batch("CREATE TRIGGER reject_completion BEFORE INSERT ON provider_context BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
    drop(c);
    let before = crate::Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(
        f.store
            .finish_provider_job(&job, "completed", Some("answer"), Some("Answer"))
            .unwrap_err()
            .code,
        connect_protocol::Error::storage().code
    );
    assert_eq!(
        crate::Store::inspect_snapshot(f.store.path()).unwrap(),
        before
    );
    assert_eq!(count(&f, "messages"), 1);
    assert_eq!(count(&f, "task_replies"), 0);
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_completion")
        .unwrap();
    f.store
        .finish_provider_job(&job, "completed", Some("answer"), Some("Answer"))
        .unwrap();
    assert_eq!(count(&f, "messages"), 2);
    assert_eq!(count(&f, "task_replies"), 1);
}

#[test]
fn expired_task_preserves_late_reply_without_a_new_task_effect() {
    let (f, _, task) = fixture();
    f.run(&send(&f, &task, "question", "Question", true))
        .unwrap();
    let job = f.store.claim_provider_job().unwrap().unwrap();
    let expired = f
        .store
        .execute_task_at(&f.key.id, &grants::get(&task), f.now + 601)
        .unwrap();
    assert_eq!(Fixture::record(expired).state, TaskState::Expired);
    f.store
        .finish_provider_job(&job, "completed", Some("late"), Some("Late answer"))
        .unwrap();
    assert_eq!(
        Fixture::record(f.run(&grants::get(&task)).unwrap()).state,
        TaskState::Expired
    );
    let TaskOutcome::Replies { items, .. } = f
        .run(&TaskRequest::new(TaskCommand::Replies {
            conversation_id: f.cid.clone(),
            after: None,
            limit: 100,
        }))
        .unwrap()
    else {
        panic!("replies")
    };
    assert_eq!(items.len(), 1);
    assert!(items[0].task_id.is_none() && items[0].reply_to.is_some());
    assert_eq!(count(&f, "messages"), 2);
    assert_eq!(count(&f, "task_effects"), 1);
}
