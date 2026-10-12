#![allow(clippy::unwrap_used)] // Isolated lifecycle assertions deliberately panic.

use super::Observer;
use connect_protocol::{Limits, Operation, Outcome};
use connect_store::{Scope, Store};
use rvoip_vapi::VapiEvent;
use std::sync::Arc;
use tokio::sync::broadcast;

fn fixture() -> (tempfile::TempDir, Arc<Store>, String, String, String) {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(
        Store::initialize(&directory.path().join("state.sqlite3"), Limits::default()).unwrap(),
    );
    let human = store.create_identity("human").unwrap();
    let agent = store.create_identity("agent").unwrap();
    let expiry = chrono::Utc::now().timestamp() + 3600;
    let key = store
        .issue(
            &human.id,
            Scope::communications(vec![agent.id.clone()]),
            expiry,
        )
        .unwrap();
    let agent_key = store
        .issue(&agent.id, Scope::communications(vec![human.id]), expiry)
        .unwrap();
    let Outcome::Conversation(conversation) = store
        .execute(
            &key.id,
            &Operation::Create {
                request_id: "observer-conversation".into(),
                participants: vec![agent.id.clone()],
            },
        )
        .unwrap()
    else {
        panic!("expected conversation");
    };
    let Outcome::Session(session) = store
        .execute(
            &key.id,
            &Operation::SessionInvite {
                request_id: "observer-session".into(),
                conversation_id: conversation.id.clone(),
                participant_id: agent.id,
                media: vec!["audio".into()],
            },
        )
        .unwrap()
    else {
        panic!("expected Session");
    };
    store
        .execute(
            &agent_key.id,
            &Operation::SessionAccept {
                request_id: "observer-accept".into(),
                conversation_id: conversation.id.clone(),
                session_id: session.id.clone(),
            },
        )
        .unwrap();
    (directory, store, key.id, conversation.id, session.id)
}

fn context(store: &Store, key: &str, conversation: &str) -> connect_protocol::ContextView {
    let Outcome::ContextView(view) = store
        .execute(
            key,
            &Operation::ContextView {
                conversation_id: conversation.into(),
                limit: 100,
            },
        )
        .unwrap()
    else {
        panic!("expected context");
    };
    view
}

fn transcript() -> VapiEvent {
    VapiEvent::Transcript {
        role: Some("user".into()),
        transcript_type: Some("final".into()),
        transcript: Some("Our decision is paper lantern.".into()),
    }
}

#[tokio::test]
async fn teardown_drains_queued_transcripts_and_records_missing_report() {
    let (_directory, store, key, conversation, session) = fixture();
    let (sender, receiver) = broadcast::channel(8);
    let observer = Observer::start(store.clone(), session, "provider-one".into(), receiver);
    sender.send(transcript()).unwrap();
    sender.send(transcript()).unwrap();
    drop(sender);
    observer.finish().await;
    let view = context(&store, &key, &conversation);
    assert_eq!(
        view.items
            .iter()
            .filter(|item| item.kind == "voice_transcript")
            .count(),
        1
    );
    assert!(view
        .items
        .iter()
        .any(|item| item.kind == "voice_context_status" && item.body.contains("end_report")));
    assert!(!view.items.iter().any(|item| item.kind == "voice_outcome"));
}

#[tokio::test]
async fn queued_provider_report_is_not_replaced_by_missing_status() {
    let (_directory, store, key, conversation, session) = fixture();
    let (sender, receiver) = broadcast::channel(8);
    let observer = Observer::start(store.clone(), session, "provider-two".into(), receiver);
    sender
        .send(VapiEvent::EndOfCallReport {
            report: serde_json::json!({"endedReason":"customer-ended-call"}),
        })
        .unwrap();
    observer.finish().await;
    let view = context(&store, &key, &conversation);
    assert!(view
        .items
        .iter()
        .any(|item| item.kind == "voice_outcome" && item.body.contains("customer-ended-call")));
    assert!(!view
        .items
        .iter()
        .any(|item| item.kind == "voice_context_status"));
}

#[tokio::test]
async fn cancelling_observer_drain_does_not_detach_the_receiver() {
    let (_directory, store, _key, _conversation, session) = fixture();
    let (sender, receiver) = broadcast::channel(8);
    let observer = Observer::start(store, session, "provider-three".into(), receiver);
    let drain = tokio::spawn(observer.finish());
    tokio::task::yield_now().await;
    drain.abort();
    let _ = drain.await;
    tokio::task::yield_now().await;
    assert_eq!(sender.receiver_count(), 0);
}
