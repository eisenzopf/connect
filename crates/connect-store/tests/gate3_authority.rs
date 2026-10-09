#![allow(clippy::unwrap_used)] // Integration assertions deliberately panic.

use connect_protocol::{AvailabilityState, Limits, Operation, Outcome};
use connect_store::{Scope, Store};

fn conversation_id(outcome: Outcome) -> String {
    match outcome {
        Outcome::Conversation(conversation) => conversation.id,
        other => panic!("expected conversation, got {other:?}"),
    }
}

fn session(outcome: Outcome) -> connect_protocol::Session {
    match outcome {
        Outcome::Session(session) => session,
        other => panic!("expected session, got {other:?}"),
    }
}

#[test]
fn gate_three_authority_and_provider_outcomes_fail_closed() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("state.sqlite3");
    let store = Store::initialize(&database, Limits::default()).unwrap();
    let app = store.create_identity("app").unwrap();
    let agent = store.create_identity("agent").unwrap();
    let stranger = store.create_identity("stranger").unwrap();
    let expiry = chrono::Utc::now().timestamp() + 3600;
    let app_key = store
        .issue(
            &app.id,
            Scope::communications(vec![agent.id.clone()]),
            expiry,
        )
        .unwrap();
    let agent_key = store
        .issue(
            &agent.id,
            Scope::communications(vec![app.id.clone()]),
            expiry,
        )
        .unwrap();
    let stranger_key = store
        .issue(&stranger.id, Scope::communications(vec![]), expiry)
        .unwrap();
    let conversation = conversation_id(
        store
            .execute(
                &app_key.id,
                &Operation::Create {
                    request_id: "authority-create".into(),
                    participants: vec![agent.id.clone()],
                },
            )
            .unwrap(),
    );
    let other_conversation = conversation_id(
        store
            .execute(
                &app_key.id,
                &Operation::Create {
                    request_id: "authority-other-create".into(),
                    participants: vec![agent.id.clone()],
                },
            )
            .unwrap(),
    );
    store
        .execute(
            &app_key.id,
            &Operation::Send {
                request_id: "authority-other-fact".into(),
                conversation_id: other_conversation.clone(),
                body: "The unrelated fact is scarlet.".into(),
            },
        )
        .unwrap();
    let invited = session(
        store
            .execute(
                &app_key.id,
                &Operation::SessionInvite {
                    request_id: "authority-invite".into(),
                    conversation_id: conversation.clone(),
                    participant_id: agent.id.clone(),
                    media: vec!["audio".into()],
                },
            )
            .unwrap(),
    );

    let initiator_accept = store
        .execute(
            &app_key.id,
            &Operation::SessionAccept {
                request_id: "authority-wrong-accept".into(),
                conversation_id: conversation.clone(),
                session_id: invited.id.clone(),
            },
        )
        .unwrap_err();
    assert_eq!(initiator_accept.code, "forbidden");
    let hidden = store
        .execute(
            &stranger_key.id,
            &Operation::SessionGet {
                conversation_id: conversation.clone(),
                session_id: invited.id.clone(),
            },
        )
        .unwrap_err();
    assert_eq!(hidden.code, "forbidden");
    let duplicate_media = store
        .execute(
            &app_key.id,
            &Operation::SessionInvite {
                request_id: "authority-duplicate-media".into(),
                conversation_id: conversation.clone(),
                participant_id: agent.id.clone(),
                media: vec!["audio".into(), "audio".into()],
            },
        )
        .unwrap_err();
    assert_eq!(duplicate_media.code, "invalid-request");

    let short_lived = match store
        .execute(
            &agent_key.id,
            &Operation::AvailabilitySet {
                request_id: "authority-availability".into(),
                state: AvailabilityState::Available,
                preferred_route: Some("native-quic".into()),
                expires_at: chrono::Utc::now().timestamp() + 1,
            },
        )
        .unwrap()
    {
        Outcome::Availability(value) => value,
        other => panic!("expected availability, got {other:?}"),
    };
    assert_eq!(short_lived.preferred_route.as_deref(), Some("native-quic"));
    std::thread::sleep(std::time::Duration::from_millis(1_100));
    let expired = match store
        .execute(
            &app_key.id,
            &Operation::AvailabilityGet {
                identity_id: agent.id.clone(),
            },
        )
        .unwrap()
    {
        Outcome::Availability(value) => value,
        other => panic!("expected availability, got {other:?}"),
    };
    assert_eq!(expired.state, "unknown");
    assert!(expired.preferred_route.is_none());
    assert_eq!(
        store
            .execute(
                &app_key.id,
                &Operation::AvailabilityGet {
                    identity_id: "unknown-identity".into(),
                },
            )
            .unwrap_err()
            .code,
        "forbidden"
    );

    store
        .execute(
            &agent_key.id,
            &Operation::SessionAccept {
                request_id: "authority-accept".into(),
                conversation_id: conversation.clone(),
                session_id: invited.id.clone(),
            },
        )
        .unwrap();
    assert!(store
        .record_session_transcript(
            &invited.id,
            "provider-connection",
            "assistant",
            "partial",
            "unfinished",
        )
        .unwrap()
        .is_none());
    let first = store
        .record_session_transcript(
            &invited.id,
            "provider-connection",
            "assistant",
            "final",
            "The durable decision is blue.",
        )
        .unwrap()
        .unwrap();
    let replay = store
        .record_session_transcript(
            &invited.id,
            "provider-connection",
            "assistant",
            "final",
            "The durable decision is blue.",
        )
        .unwrap()
        .unwrap();
    assert_eq!(first, replay);
    assert_eq!(first.sender, agent.id);
    let corrected = store
        .record_session_transcript(
            &invited.id,
            "provider-connection",
            "assistant",
            "final",
            "The durable decision is green.",
        )
        .unwrap()
        .unwrap();
    let corrected_replay = store
        .record_session_transcript(
            &invited.id,
            "provider-connection",
            "assistant",
            "final",
            "The durable decision is green.",
        )
        .unwrap()
        .unwrap();
    assert_eq!(corrected, corrected_replay);
    let correction: serde_json::Value = serde_json::from_str(&corrected.body).unwrap();
    assert_eq!(correction["corrects_source_id"], first.id);
    assert_eq!(correction["body"], "The durable decision is green.");
    let report = serde_json::json!({
        "endedReason": "assistant-ended-call",
        "analysis": {
            "summary": "The durable decision is blue.",
            "successEvaluation": "true"
        },
        "untrusted": "not retained"
    });
    let first_outcome = store
        .record_session_outcome(&invited.id, "provider-connection", &report)
        .unwrap();
    let replayed_outcome = store
        .record_session_outcome(&invited.id, "provider-connection", &report)
        .unwrap();
    assert_eq!(first_outcome, replayed_outcome);
    let missing = store
        .record_session_context_gap(&invited.id, "partial-provider", "end_report")
        .unwrap();
    assert_eq!(
        missing,
        store
            .record_session_context_gap(&invited.id, "partial-provider", "end_report")
            .unwrap()
    );
    assert!(missing.body.contains("connect-bridge"));
    assert_eq!(
        store
            .record_session_context_gap("forged-session", "partial-provider", "end_report")
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert!(!first_outcome.body.contains("not retained"));
    assert_eq!(
        store
            .record_session_outcome("forged-session", "provider-connection", &report)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        store
            .record_session_transcript(
                &invited.id,
                "provider-connection",
                "system",
                "final",
                "forged role",
            )
            .unwrap_err()
            .code,
        "invalid-request"
    );

    drop(store);
    let store = Store::open(&database, Limits::default()).unwrap();
    let context = match store
        .execute(
            &app_key.id,
            &Operation::ContextView {
                conversation_id: conversation,
                limit: 100,
            },
        )
        .unwrap()
    {
        Outcome::ContextView(context) => context,
        other => panic!("expected context, got {other:?}"),
    };
    let transcripts = context
        .items
        .iter()
        .filter(|item| item.kind == "voice_transcript")
        .collect::<Vec<_>>();
    assert_eq!(transcripts.len(), 1);
    assert_eq!(transcripts[0].source_id, first.id);
    assert_eq!(transcripts[0].actor_id, agent.id);
    let corrections = context
        .items
        .iter()
        .filter(|item| item.kind == "voice_transcript_correction")
        .collect::<Vec<_>>();
    assert_eq!(corrections.len(), 1);
    assert_eq!(corrections[0].source_id, corrected.id);
    assert!(context
        .items
        .iter()
        .any(|item| item.kind == "voice_outcome" && item.source_id == first_outcome.id));
    assert!(!context
        .items
        .iter()
        .any(|item| item.body.contains("scarlet")));
    store
        .execute(
            &app_key.id,
            &Operation::Send {
                request_id: "authority-after-call".into(),
                conversation_id: context.conversation_id.clone(),
                body: "The persistent conversation remains open.".into(),
            },
        )
        .unwrap();
    let other_context = match store
        .execute(
            &agent_key.id,
            &Operation::ContextView {
                conversation_id: other_conversation,
                limit: 100,
            },
        )
        .unwrap()
    {
        Outcome::ContextView(context) => context,
        other => panic!("expected context, got {other:?}"),
    };
    assert!(other_context
        .items
        .iter()
        .any(|item| item.body.contains("scarlet")));
    assert!(!other_context
        .items
        .iter()
        .any(|item| item.body.contains("durable decision")));
}
