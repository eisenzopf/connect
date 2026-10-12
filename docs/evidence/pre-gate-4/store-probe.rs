#![allow(clippy::unwrap_used)]
use connect_protocol::{Limits, Operation, Outcome};
use connect_store::{Scope, Store};

#[test]
fn review_discovery_and_delivery_history() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("state.sqlite3");
    let store = Store::initialize(&database, Limits::default()).unwrap();
    let a = store.create_identity("reviewer").unwrap();
    let b = store.create_identity("assistant").unwrap();
    let expires = chrono::Utc::now().timestamp() + 3600;
    let ak = store.issue(&a.id, Scope::communications(vec![b.id.clone()]), expires).unwrap();
    let bk = store.issue(&b.id, Scope::communications(vec![a.id.clone()]), expires).unwrap();
    let Outcome::Conversation(c) = store.execute(&ak.id, &Operation::Create {
        request_id: "create".into(), participants: vec![b.id.clone()],
    }).unwrap() else { panic!("conversation") };
    store.execute(&ak.id, &Operation::Send { request_id: "fact".into(),
        conversation_id: c.id.clone(), body: "Retained fact".into() }).unwrap();
    let Outcome::Session(s) = store.execute(&ak.id, &Operation::SessionInvite {
        request_id: "invite".into(), conversation_id: c.id.clone(),
        participant_id: b.id.clone(), media: vec!["audio".into()],
    }).unwrap() else { panic!("session") };
    store.execute(&bk.id, &Operation::SessionAccept { request_id: "accept".into(),
        conversation_id: c.id.clone(), session_id: s.id.clone() }).unwrap();
    let Outcome::Identity(identity) = store.execute(&ak.id, &Operation::Resolve {
        address: "connect:assistant@local".into(),
    }).unwrap() else { panic!("identity") };
    assert_eq!(identity.capabilities, vec!["messaging"]);
    let selection = store.context_for_session(&ak.id, &s.id).unwrap();
    let mut newest = String::new();
    for i in 0..101 {
        newest = store.begin_context_delivery(&ak.id, &s.id, "vapi", &selection).unwrap();
        store.finish_context_delivery(&newest, "delivered", Some("local-fixture")).unwrap();
        // Deterministic ordering in this disposable fixture, without sleeping.
        rusqlite::Connection::open(&database).unwrap().execute(
            "UPDATE context_deliveries SET created_at=?1 WHERE id=?2", rusqlite::params![i, newest],
        ).unwrap();
    }
    let Outcome::ContextView(view) = store.execute(&ak.id, &Operation::ContextView {
        conversation_id: c.id, limit: 100,
    }).unwrap() else { panic!("context") };
    assert_eq!(view.deliveries.len(), 100);
    assert!(!view.deliveries.iter().any(|d| d.id == newest));
    assert_eq!(view.omitted, 0, "source omissions do not disclose omitted delivery records");
    println!("REPRODUCED: accepted audio participant resolves as messaging only; newest of 101 deliveries hidden, with no delivery continuation or omission field");
}
