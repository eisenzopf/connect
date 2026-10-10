use super::{fixture::Fixture, native_resource_fixture::MediaFixture};
use connect_protocol::{tasks::*, Operation};

fn setup() -> MediaFixture {
    let mut m = MediaFixture::new();
    let mut create = m.f.create();
    let TaskCommand::Create {
        request_id,
        definition,
    } = &mut create.command
    else {
        panic!("create")
    };
    *request_id = "history-media-task".into();
    definition.scope.operations = vec![
        TaskPermission::SessionInvite,
        TaskPermission::SessionGet,
        TaskPermission::History,
    ];
    definition.scope.recipients = vec![m.peer_id.clone()];
    m.task = Fixture::record(m.f.run(&create).unwrap());
    m.f.store
        .execute(
            &m.f.key.id,
            &Operation::Send {
                request_id: "old-fact".into(),
                conversation_id: m.f.cid.clone(),
                body: "HISTORY-CANARY".into(),
            },
        )
        .unwrap();
    m
}
fn permissions(history: bool) -> Vec<TaskPermission> {
    let mut ops = vec![TaskPermission::SessionInvite, TaskPermission::SessionGet];
    if history {
        ops.push(TaskPermission::History);
    }
    ops
}
fn count(m: &MediaFixture) -> u64 {
    m.f.store
        .connection()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM context_deliveries", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn voice_context_uses_invocation_ceiling_and_persists_exact_source_selection() {
    let m = setup();
    let session = m.accepted_outbound("voice", true);
    let narrow = m.grant(&m.task, "no-history", permissions(false));
    let selected =
        m.f.store
            .prepare_vapi_session_context(&narrow, &session.id)
            .unwrap();
    assert!(!selected.json.contains("HISTORY-CANARY"));
    let empty: TaskOutcome = serde_json::from_str(&selected.json).unwrap();
    assert!(
        matches!(empty, TaskOutcome::Recall {sources,next:None,omitted:0} if sources.is_empty())
    );
    assert!(
        m.f.store.context_for_session(&narrow, &session.id).is_err(),
        "reserved subjects cannot use the owner projection"
    );
    let wide = m.grant(&m.task, "with-history", permissions(true));
    let selected =
        m.f.store
            .prepare_vapi_session_context(&wide, &session.id)
            .unwrap();
    assert!(selected.json.contains("HISTORY-CANARY"));
    assert!(selected.json.contains("conversation-journal"));
    let (saved, state, watermark): (String, String, u64) =
        m.f.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT selection_json,state,watermark FROM context_deliveries WHERE id=?1",
                [&selected.delivery_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
    assert_eq!(saved, selected.json);
    assert_eq!(state, "pending");
    assert!(watermark > 0);
    assert_eq!(count(&m), 2);
}

#[test]
fn current_revocation_and_terminal_task_prevent_new_voice_context_intents() {
    for cancel in [false, true] {
        let m = setup();
        let session = m.accepted_outbound("voice", true);
        let subject = m.grant(&m.task, "grant", permissions(true));
        m.f.store
            .prepare_vapi_session_context(&subject, &session.id)
            .unwrap();
        if cancel {
            m.f.run(&TaskRequest::new(TaskCommand::Cancel {
                request_id: "cancel".into(),
                task_id: m.task.id.clone(),
                reason: None,
            }))
            .unwrap();
        } else {
            let grant = connect_protocol::tasks::task_grant_id(&subject).unwrap();
            m.f.store.revoke_task_grant(&m.f.key.id, grant).unwrap();
        }
        assert_eq!(
            m.f.store
                .prepare_vapi_session_context(&subject, &session.id)
                .err()
                .unwrap()
                .code,
            "forbidden"
        );
        assert_eq!(count(&m), 1);
    }
}

#[test]
fn storage_failure_rolls_back_voice_intent_and_allows_definite_local_retry() {
    let m = setup();
    let session = m.accepted_outbound("voice", true);
    let subject = m.grant(&m.task, "grant", permissions(true));
    m.f.store.connection().unwrap().execute_batch(
        "CREATE TRIGGER fail_voice_context BEFORE INSERT ON context_deliveries BEGIN SELECT RAISE(ABORT,'fixture'); END;"
    ).unwrap();
    assert!(m
        .f
        .store
        .prepare_vapi_session_context(&subject, &session.id)
        .is_err());
    assert_eq!(count(&m), 0);
    m.f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_voice_context")
        .unwrap();
    m.f.store
        .prepare_vapi_session_context(&subject, &session.id)
        .unwrap();
    assert_eq!(count(&m), 1);
}

#[test]
fn native_voice_selection_preserves_budget_omissions_cursor_and_conversation_boundary() {
    let m = setup();
    for i in 0..5 {
        m.f.store
            .execute(
                &m.f.key.id,
                &Operation::Send {
                    request_id: format!("large-{i}"),
                    conversation_id: m.f.cid.clone(),
                    body: format!("FACT-{i}: {}", "x".repeat(16_000)),
                },
            )
            .unwrap();
    }
    let connect_protocol::Outcome::Conversation(other) =
        m.f.store
            .execute(
                &m.f.key.id,
                &Operation::Create {
                    request_id: "other-conversation".into(),
                    participants: vec![m.peer_id.clone()],
                },
            )
            .unwrap()
    else {
        panic!("conversation")
    };
    m.f.store
        .execute(
            &m.f.key.id,
            &Operation::Send {
                request_id: "other-fact".into(),
                conversation_id: other.id,
                body: "OTHER-CONVERSATION-CANARY".into(),
            },
        )
        .unwrap();
    let session = m.accepted_outbound("voice", true);
    let subject = m.grant(&m.task, "grant", permissions(true));
    let selected =
        m.f.store
            .prepare_vapi_session_context(&subject, &session.id)
            .unwrap();
    assert!(selected.json.len() <= TASK_CONTEXT_BYTES);
    assert!(!selected.json.contains("OTHER-CONVERSATION-CANARY"));
    assert!(selected.json.contains("HISTORY-CANARY"));
    assert!(selected.json.contains("conversation-journal"));
    let TaskOutcome::Recall {
        sources,
        next,
        omitted,
    } = serde_json::from_str(&selected.json).unwrap()
    else {
        panic!("recall")
    };
    assert!(!sources.is_empty());
    assert!(next.is_some());
    assert!(omitted > 0);
}
