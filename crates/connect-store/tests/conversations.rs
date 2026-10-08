#![allow(clippy::unwrap_used)] // Fixture failures intentionally panic.
use connect_protocol::{Limits, Operation, Outcome};
use connect_store::{Scope, Store};
use std::sync::Arc;

fn setup(limits: Limits) -> (tempfile::TempDir, Arc<Store>, String, String, String) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::initialize(&dir.path().join("state.sqlite3"), limits).unwrap());
    let app = store.create_identity("app").unwrap();
    let agent = store.create_identity("agent").unwrap();
    let expiry = chrono::Utc::now().timestamp() + 60;
    let app_key = store
        .issue(&app.id, Scope::messaging(vec![agent.id.clone()]), expiry)
        .unwrap();
    let agent_key = store
        .issue(&agent.id, Scope::messaging(vec![app.id.clone()]), expiry)
        .unwrap();
    (dir, store, app_key.id, agent_key.id, agent.id)
}
fn create(store: &Store, actor: &str, peer: &str) -> String {
    match store
        .execute(
            actor,
            &Operation::Create {
                request_id: "create-1".into(),
                participants: vec![peer.into()],
            },
        )
        .unwrap()
    {
        Outcome::Conversation(c) => c.id,
        _ => panic!("unexpected outcome"),
    }
}
fn send(id: &str, body: &str, request: &str) -> Operation {
    Operation::Send {
        request_id: request.into(),
        conversation_id: id.into(),
        body: body.into(),
    }
}

#[test]
fn replay_conflict_receipt_history_cursor_and_close_survive_restart() {
    let (dir, store, app, agent, peer) = setup(Limits::default());
    let id = create(&store, &app, &peer);
    let operation = send(&id, "hello", "send-1");
    let accepted = store.execute(&app, &operation).unwrap();
    assert_eq!(accepted, store.execute(&app, &operation).unwrap());
    assert_eq!(
        store
            .execute(&app, &send(&id, "changed", "send-1"))
            .unwrap_err()
            .code,
        "conflict"
    );
    match store
        .execute(
            &agent,
            &Operation::Inbox {
                cursor: None,
                limit: 1,
                wait_ms: 0,
            },
        )
        .unwrap()
    {
        Outcome::Inbox { events, cursor } => {
            assert_eq!(events.len(), 1);
            let cursor = cursor.unwrap();
            match store
                .execute(
                    &agent,
                    &Operation::Inbox {
                        cursor: Some(cursor.clone()),
                        limit: 1,
                        wait_ms: 0,
                    },
                )
                .unwrap()
            {
                Outcome::Inbox { events, .. } => {
                    assert_eq!(events[0].message.as_ref().unwrap().body, "hello")
                }
                _ => panic!("inbox"),
            }
            assert_eq!(
                store
                    .execute(
                        &app,
                        &Operation::Inbox {
                            cursor: Some(cursor),
                            limit: 1,
                            wait_ms: 0
                        }
                    )
                    .unwrap_err()
                    .code,
                "invalid-request"
            );
        }
        _ => panic!("inbox"),
    }
    let receipt = store
        .execute(
            &app,
            &Operation::RequestStatus {
                request_id: "send-1".into(),
            },
        )
        .unwrap();
    assert!(matches!(receipt,Outcome::RequestStatus {state,outcome:Some(_)} if state=="accepted"));
    drop(store);
    let store = Store::open(&dir.path().join("state.sqlite3"), Limits::default()).unwrap();
    assert_eq!(accepted, store.execute(&app, &operation).unwrap());
    match store
        .execute(
            &agent,
            &Operation::History {
                conversation_id: id.clone(),
                after: None,
                limit: 100,
            },
        )
        .unwrap()
    {
        Outcome::History { messages, .. } => assert_eq!(messages.len(), 1),
        _ => panic!("history"),
    }
    store
        .execute(
            &agent,
            &Operation::Close {
                request_id: "close-1".into(),
                conversation_id: id.clone(),
            },
        )
        .unwrap();
    assert_eq!(
        store
            .execute(&app, &send(&id, "closed", "send-2"))
            .unwrap_err()
            .code,
        "conversation-closed"
    );
    assert!(
        matches!(store.execute(&app,&Operation::RequestStatus {request_id:"send-2".into()}).unwrap(),Outcome::RequestStatus {state,..} if state=="rejected")
    );
    assert_eq!(accepted, store.execute(&app, &operation).unwrap());
    store.integrity_check().unwrap();
}
#[test]
fn concurrent_duplicate_requests_have_one_effect() {
    let (_dir, store, app, _agent, peer) = setup(Limits::default());
    let id = create(&store, &app, &peer);
    let mut handles = Vec::new();
    for _ in 0..8 {
        let store = store.clone();
        let app = app.clone();
        let op = send(&id, "one effect", "same-request");
        handles.push(std::thread::spawn(move || {
            store.execute(&app, &op).unwrap()
        }));
    }
    let results = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect::<Vec<_>>();
    assert!(results.iter().all(|r| r == &results[0]));
    match store
        .execute(
            &app,
            &Operation::History {
                conversation_id: id,
                after: None,
                limit: 100,
            },
        )
        .unwrap()
    {
        Outcome::History { messages, .. } => assert_eq!(messages.len(), 1),
        _ => panic!("history"),
    }
}
#[test]
fn serialized_capacity_and_failed_sqlite_write_never_acknowledge() {
    let limits = Limits {
        message_storage_bytes: 5,
        message_bytes: 5,
        journal_events: 6,
        ..Limits::default()
    };
    let (dir, store, app, agent, peer) = setup(limits);
    let id = create(&store, &app, &peer);
    let mut handles = Vec::new();
    for request in ["one", "two"] {
        let store = store.clone();
        let app = app.clone();
        let op = send(&id, "hello", request);
        handles.push(std::thread::spawn(move || store.execute(&app, &op)));
    }
    let results = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        store
            .execute(&app, &send(&id, "123456", "oversize"))
            .unwrap_err()
            .code,
        "invalid-request"
    );
    let db = rusqlite::Connection::open(dir.path().join("state.sqlite3")).unwrap();
    db.execute_batch("CREATE TRIGGER write_fault BEFORE INSERT ON requests BEGIN SELECT RAISE(ABORT,'injected storage failure'); END;").unwrap();
    assert_eq!(
        store
            .execute(
                &app,
                &Operation::Close {
                    request_id: "fault-close".into(),
                    conversation_id: id.clone()
                }
            )
            .unwrap_err()
            .code,
        "storage-unavailable"
    );
    db.execute_batch("DROP TRIGGER write_fault;").unwrap();
    assert!(
        matches!(store.execute(&app,&Operation::RequestStatus {request_id:"fault-close".into()}).unwrap(),Outcome::RequestStatus {state,..} if state=="not_found")
    );
    // Closing was rolled back along with its failed receipt and events.
    match store
        .execute(
            &agent,
            &Operation::History {
                conversation_id: id,
                after: None,
                limit: 100,
            },
        )
        .unwrap()
    {
        Outcome::History { messages, .. } => assert_eq!(messages.len(), 1),
        _ => panic!("history"),
    }
    assert_eq!(
        db.query_row::<i64, _, _>("SELECT SUM(closed) FROM conversations", [], |r| r.get(0))
            .unwrap(),
        0
    );
}
#[test]
fn lesser_scope_cannot_disclose_previous_receipts() {
    let (_dir, store, app, _agent, peer) = setup(Limits::default());
    let id = create(&store, &app, &peer);
    store.execute(&app, &send(&id, "secret", "send-1")).unwrap();
    let identity = store.revalidate(&app).unwrap().identity_id;
    let narrow = store
        .issue(
            &identity,
            Scope {
                operations: vec!["request_status".into()],
                recipients: vec![],
            },
            chrono::Utc::now().timestamp() + 60,
        )
        .unwrap();
    assert_eq!(
        store
            .execute(
                &narrow.id,
                &Operation::RequestStatus {
                    request_id: "send-1".into()
                }
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
}
