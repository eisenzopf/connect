//! Missing/duplicate wake hints and authority changes must not alter journal truth.
#![allow(clippy::unwrap_used)]
use super::*;
use connect_protocol::{events::*, messages::MessageDraft, Limits};
use connect_store::Scope;
#[tokio::test]
async fn event_catchup_requeries_the_journal_after_missing_or_duplicate_wake_hints() {
    for notify in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(
            Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap(),
        );
        let alice = store.create_identity("alice").unwrap().id;
        let bob = store.create_identity("bob").unwrap().id;
        let expiry = chrono::Utc::now().timestamp() + 300;
        let a = store.issue(&alice, Scope::operator(), expiry).unwrap().id;
        let b = store.issue(&bob, Scope::operator(), expiry).unwrap().id;
        let Outcome::Conversation(conv) = store
            .execute(
                &a,
                &Operation::Create {
                    request_id: "create".into(),
                    request: connect_protocol::conversations::CreateRequest::persistent(vec![bob]),
                },
            )
            .unwrap()
        else {
            panic!()
        };
        let cursor = store
            .conversation_snapshot(&b, &conv.id, SnapshotQuery::default())
            .unwrap()
            .cursor;
        let reached = Arc::new(tokio::sync::Barrier::new(2));
        let release = Arc::new(tokio::sync::Barrier::new(2));
        let service = Arc::new(Service {
            assurance_policy: Default::default(),
            canonical_runtime: Default::default(),
            record_references: Default::default(),
            store: store.clone(),
            changed: tokio::sync::Notify::new(),
            readiness: Arc::new(readiness::OperationalReadiness::default()),
            wait_boundary: Some((reached.clone(), release.clone())),
            blocking_boundary: None,
            runtime: Default::default(),
        });
        let waiter = service.clone();
        let cid = conv.id.clone();
        let task = tokio::spawn(async move {
            waiter
                .execute(
                    b,
                    Operation::ConversationEvents {
                        conversation_id: cid,
                        query: EventQuery {
                            after: Some(cursor),
                            wait_ms: if notify { 5000 } else { 30 },
                            ..Default::default()
                        },
                    },
                )
                .await
        });
        reached.wait().await;
        store
            .execute(
                &a,
                &Operation::Send {
                    request_id: "message".into(),
                    conversation_id: conv.id,
                    message: MessageDraft::text_all("committed without relying on a wake hint"),
                },
            )
            .unwrap();
        if notify {
            service.notify_changed();
            service.notify_changed();
        }
        release.wait().await;
        let result = tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let Outcome::ConversationEvents(page) = result else {
            panic!()
        };
        assert_eq!(page.events.len(), 1);
        assert!(
            matches!(&page.events[0].data,JournalData::Message(message)if message.body.contains("wake hint"))
        );
    }
}
#[tokio::test]
async fn event_wait_reauthorizes_before_delivering_after_membership_removal() {
    let dir = tempfile::tempdir().unwrap();
    let store =
        Arc::new(Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap());
    let alice = store.create_identity("alice").unwrap().id;
    let bob = store.create_identity("bob").unwrap().id;
    let expiry = chrono::Utc::now().timestamp() + 300;
    let a = store.issue(&alice, Scope::operator(), expiry).unwrap().id;
    let b = store.issue(&bob, Scope::operator(), expiry).unwrap().id;
    let Outcome::Conversation(conv) = store
        .execute(
            &a,
            &Operation::Create {
                request_id: "create".into(),
                request: connect_protocol::conversations::CreateRequest::persistent(vec![
                    bob.clone()
                ]),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let snapshot = store.membership_snapshot(&a, &conv.id).unwrap();
    let participant = snapshot
        .participants
        .iter()
        .find(|m| m.identity_id == bob)
        .unwrap()
        .participant_id
        .clone();
    let cursor = store
        .conversation_snapshot(&b, &conv.id, SnapshotQuery::default())
        .unwrap()
        .cursor;
    let reached = Arc::new(tokio::sync::Barrier::new(2));
    let release = Arc::new(tokio::sync::Barrier::new(2));
    let service = Arc::new(Service {
        assurance_policy: Default::default(),
        canonical_runtime: Default::default(),
        record_references: Default::default(),
        store: store.clone(),
        changed: tokio::sync::Notify::new(),
        readiness: Arc::new(readiness::OperationalReadiness::default()),
        wait_boundary: Some((reached.clone(), release.clone())),
        blocking_boundary: None,
        runtime: Default::default(),
    });
    let waiter = service.clone();
    let cid = conv.id.clone();
    let task = tokio::spawn(async move {
        waiter
            .execute(
                b,
                Operation::ConversationEvents {
                    conversation_id: cid,
                    query: EventQuery {
                        after: Some(cursor),
                        wait_ms: 5000,
                        ..Default::default()
                    },
                },
            )
            .await
    });
    reached.wait().await;
    store
        .execute_membership(
            &a,
            connect_protocol::membership::MembershipRequest {
                operation_id: "remove".into(),
                conversation_id: conv.id,
                expected_revision: 0,
                change: connect_protocol::membership::MembershipAction::Remove {
                    participant_id: participant,
                    reason: None,
                },
            },
        )
        .unwrap();
    service.notify_changed();
    release.wait().await;
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err()
            .code,
        "forbidden"
    );
}

#[tokio::test]
async fn closing_a_physical_peer_cancels_both_journal_and_observation_waits() {
    use rvoip_auth_core::BearerValidator;
    let dir = tempfile::tempdir().unwrap();
    let store =
        Arc::new(Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap());
    let alice = store.create_identity("alice").unwrap().id;
    let bob = store.create_identity("bob").unwrap().id;
    let expiry = chrono::Utc::now().timestamp() + 300;
    let a = store.issue(&alice, Scope::operator(), expiry).unwrap();
    let b = store.issue(&bob, Scope::operator(), expiry).unwrap();
    let Outcome::Conversation(room) = store
        .execute(
            &a.id,
            &Operation::Create {
                request_id: "create".into(),
                request: connect_protocol::conversations::CreateRequest::persistent(vec![
                    bob.clone()
                ]),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let Outcome::Session(session) = store
        .execute(
            &a.id,
            &Operation::SessionInvite {
                request_id: "invite".into(),
                conversation_id: room.id.clone(),
                participant_id: bob,
                media: vec!["audio".into()],
            },
        )
        .unwrap()
    else {
        panic!()
    };
    store
        .execute(
            &b.id,
            &Operation::SessionAccept {
                request_id: "accept".into(),
                conversation_id: room.id.clone(),
                session_id: session.id.clone(),
            },
        )
        .unwrap();
    let Outcome::Inbox { cursor, .. } = store
        .execute(
            &a.id,
            &Operation::Inbox {
                cursor: None,
                limit: 100,
                wait_ms: 0,
            },
        )
        .unwrap()
    else {
        panic!("inbox")
    };
    let service = Service::new(store.clone());
    for operation in [
        // Canonical conversation events no longer enter the earlier profile.
        // The explicit application inbox still owns a cancellable journal wait.
        Operation::Inbox {
            cursor,
            limit: 100,
            wait_ms: 5000,
        },
        Operation::SessionObservations {
            conversation_id: room.id.clone(),
            session_id: session.id,
            query: ObservationQuery {
                wait_ms: 5000,
                ..Default::default()
            },
        },
    ] {
        let principal = StoreValidator::new(store.clone())
            .validate_principal(&a.token)
            .await
            .unwrap();
        let (outbound, _) = tokio::sync::mpsc::channel(1);
        let closed = tokio_util::sync::CancellationToken::new();
        let context = ApplicationContext {
            principal,
            outbound,
            closed: closed.clone(),
        };
        let cid = operation.conversation_id().map(str::to_owned);
        let mut envelope = UctpEnvelope::new(
            MessageType::Unknown(REQUEST_TYPE.into()),
            serde_json::to_value(Request::new(operation)).unwrap(),
        );
        envelope.cid = cid;
        let reader = service.clone();
        let waiting = tokio::spawn(async move { reader.handle(context, envelope).await.unwrap() });
        closed.cancel();
        let result = tokio::time::timeout(std::time::Duration::from_secs(1), waiting)
            .await
            .unwrap()
            .unwrap();
        let response: Response = result.decode_payload().unwrap();
        assert_eq!(response.result.unwrap_err().code, "cancelled");
    }
}
