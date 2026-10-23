#![allow(clippy::unwrap_used)]
use connect_protocol::{Limits, Operation, Outcome};
use connect_store::{Scope, Store};

#[test]
fn discovery_and_delivery_pages_are_authorized_and_survive_restart() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("state.sqlite3");
    let store = Store::initialize(&database, Limits::default()).unwrap();
    let a = store.create_identity("reviewer").unwrap();
    let b = store.create_identity("assistant").unwrap();
    let expires = chrono::Utc::now().timestamp() + 3600;
    let ak = store
        .issue(&a.id, Scope::communications(vec![b.id.clone()]), expires)
        .unwrap();
    let bk = store
        .issue(&b.id, Scope::communications(vec![a.id.clone()]), expires)
        .unwrap();
    let Outcome::Conversation(c) = store
        .execute(
            &ak.id,
            &Operation::Create {
                request_id: "create".into(),
                request: connect_protocol::conversations::CreateRequest::persistent(vec![b
                    .id
                    .clone()]),
            },
        )
        .unwrap()
    else {
        panic!("conversation")
    };
    store
        .execute(
            &ak.id,
            &Operation::Send {
                request_id: "fact".into(),
                conversation_id: c.id.clone(),
                message: connect_protocol::messages::MessageDraft::text_all("Retained fact"),
            },
        )
        .unwrap();
    let Outcome::Session(s) = store
        .execute(
            &ak.id,
            &Operation::SessionInvite {
                request_id: "invite".into(),
                conversation_id: c.id.clone(),
                participant_id: b.id.clone(),
                media: vec!["audio".into()],
            },
        )
        .unwrap()
    else {
        panic!("session")
    };
    store
        .execute(
            &bk.id,
            &Operation::SessionAccept {
                request_id: "accept".into(),
                conversation_id: c.id.clone(),
                session_id: s.id.clone(),
            },
        )
        .unwrap();
    let runtime = connect_store::DiscoveryRuntime {
        native_audio: true,
        ..Default::default()
    };
    let Outcome::Capabilities(capabilities) = store
        .execute_with_runtime(
            &ak.id,
            &Operation::Capabilities {
                address: "connect:assistant@local".into(),
                conversation_id: Some(c.id.clone()),
            },
            &runtime,
        )
        .unwrap()
    else {
        panic!("capabilities")
    };
    assert!(capabilities
        .routes
        .iter()
        .any(|r| r.media.contains(&"audio".into()) && r.readiness == "configured"));
    assert!(capabilities
        .routes
        .iter()
        .any(|r| r.media.contains(&"files".into())));
    let restricted = store
        .issue(
            &a.id,
            Scope {
                operations: vec!["resolve".into()],
                recipients: vec![b.id.clone()],
            },
            expires,
        )
        .unwrap();
    let Outcome::Capabilities(limited) = store
        .execute_with_runtime(
            &restricted.id,
            &Operation::Capabilities {
                address: "connect:assistant@local".into(),
                conversation_id: Some(c.id.clone()),
            },
            &runtime,
        )
        .unwrap()
    else {
        panic!()
    };
    assert!(limited.routes.is_empty());
    assert!(limited.availability.is_none());
    let binding = connect_protocol::BrowserBinding {
        conversation_id: c.id.clone(),
        participant_id: b.id.clone(),
        session_id: None,
        signaling_url: "ws://127.0.0.1:8181".into(),
    };
    let runtime = connect_store::DiscoveryRuntime {
        browser: Some(connect_store::BrowserRoute {
            credential_id: ak.id.clone(),
            binding: binding.clone(),
        }),
        ..runtime
    };
    assert_eq!(
        store
            .execute_with_runtime(
                &ak.id,
                &Operation::BrowserBinding {
                    conversation_id: None,
                    session_id: None
                },
                &runtime
            )
            .unwrap(),
        Outcome::BrowserBinding(binding)
    );
    assert!(store
        .execute_with_runtime(
            &bk.id,
            &Operation::BrowserBinding {
                conversation_id: None,
                session_id: None
            },
            &runtime
        )
        .is_err());
    let selection = store.context_for_session(&ak.id, &s.id).unwrap();
    let mut newest = String::new();
    for i in 0..101 {
        newest = store
            .begin_context_delivery(&ak.id, &s.id, "vapi", &selection)
            .unwrap();
        store
            .finish_context_delivery(
                &newest,
                if i == 100 { "failed" } else { "delivered" },
                Some("local-fixture"),
            )
            .unwrap();
        // Deterministic ordering in this disposable fixture, without sleeping.
        rusqlite::Connection::open(&database)
            .unwrap()
            .execute(
                "UPDATE context_deliveries SET created_at=?1 WHERE id=?2",
                rusqlite::params![i, newest],
            )
            .unwrap();
    }
    let Outcome::ContextView(view) = store
        .execute(
            &ak.id,
            &Operation::ContextView {
                conversation_id: c.id.clone(),
                limit: 100,
            },
        )
        .unwrap()
    else {
        panic!("context")
    };
    assert_eq!(view.deliveries.len(), 100);
    assert!(view.deliveries.iter().any(|d| d.id == newest));
    assert_eq!(view.deliveries_omitted, 1);
    assert_eq!(view.deliveries[0].state, "failed");
    assert_eq!(
        view.omitted, 0,
        "source omissions do not disclose omitted delivery records"
    );
    let Outcome::ContextDeliveries(page) = store
        .execute(
            &ak.id,
            &Operation::ContextDeliveries {
                conversation_id: c.id.clone(),
                before: None,
                limit: 100,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(page.deliveries[0].id, newest);
    assert!(page.next.is_some());
    drop(store);
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute_batch("VACUUM;")
        .unwrap();
    let store = Store::open(&database, Limits::default()).unwrap();
    let Outcome::ContextDeliveries(rest) = store
        .execute(
            &ak.id,
            &Operation::ContextDeliveries {
                conversation_id: c.id.clone(),
                before: page.next.clone(),
                limit: 100,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(rest.deliveries.len(), 1);
    assert!(rest.next.is_none());
    assert!(!page
        .deliveries
        .iter()
        .any(|d| d.id == rest.deliveries[0].id));
    assert!(store
        .execute(
            &bk.id,
            &Operation::ContextDeliveries {
                conversation_id: c.id.clone(),
                before: page.next.clone(),
                limit: 100
            }
        )
        .is_err());
    assert!(store
        .execute(
            &ak.id,
            &Operation::ContextDeliveries {
                conversation_id: c.id.clone(),
                before: Some("forged".into()),
                limit: 100
            }
        )
        .is_err());
    let op = store.issue(&a.id, Scope::operator(), expires).unwrap();
    let actor = store.revalidate(&op.id).unwrap();
    store.revoke(&actor, &ak.id).unwrap();
    assert!(store
        .execute(
            &ak.id,
            &Operation::ContextDeliveries {
                conversation_id: c.id,
                before: page.next,
                limit: 100
            }
        )
        .is_err());
}
