#![allow(clippy::unwrap_used)] // Fixture failures intentionally panic.
use connect_protocol::{Limits, Operation, Outcome};
use connect_store::{Scope, Store};
use std::sync::Arc;

#[test]
fn cursor_pagination_utf8_bounds_and_participant_normalization() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::initialize(
        &temp.path().join("state.sqlite3"),
        Limits {
            message_bytes: 5,
            page_size: 1,
            participants: 2,
            ..Limits::default()
        },
    )
    .unwrap();
    let app = store.create_identity("app").unwrap();
    let agent = store.create_identity("agent").unwrap();
    let third = store.create_identity("third").unwrap();
    let key = store
        .issue(
            &app.id,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 60,
        )
        .unwrap();
    let id = match store
        .execute(
            &key.id,
            &Operation::Create {
                request_id: "create".into(),
                request: connect_protocol::conversations::CreateRequest::persistent(vec![
                    agent.id.clone(),
                    agent.id,
                ]),
            },
        )
        .unwrap()
    {
        Outcome::Conversation(c) => {
            assert_eq!(c.participants.len(), 2);
            c.id
        }
        _ => panic!("create"),
    };
    assert_eq!(
        store
            .execute(
                &key.id,
                &Operation::Create {
                    request_id: "too-many".into(),
                    request: connect_protocol::conversations::CreateRequest::persistent(vec![
                        third.id,
                        "nonexistent".into()
                    ])
                }
            )
            .unwrap_err()
            .code,
        "invalid-request"
    );
    for (request, body) in [("one", "éé"), ("two", "12345")] {
        store
            .execute(
                &key.id,
                &Operation::Send {
                    request_id: request.into(),
                    conversation_id: id.clone(),
                    message: connect_protocol::messages::MessageDraft::text_all(body),
                },
            )
            .unwrap();
    }
    assert_eq!(
        store
            .execute(
                &key.id,
                &Operation::Send {
                    request_id: "oversize".into(),
                    conversation_id: id.clone(),
                    message: connect_protocol::messages::MessageDraft::text_all("ééé")
                }
            )
            .unwrap_err()
            .code,
        "invalid-request"
    );
    let first = store
        .execute(
            &key.id,
            &Operation::History {
                conversation_id: id.clone(),
                after: None,
                limit: 1,
            },
        )
        .unwrap();
    let cursor = match first {
        Outcome::History { messages, next } => {
            assert_eq!(messages[0].body, "éé");
            next.unwrap()
        }
        _ => panic!("history"),
    };
    let second = store
        .execute(
            &key.id,
            &Operation::History {
                conversation_id: id.clone(),
                after: Some(cursor.clone()),
                limit: 1,
            },
        )
        .unwrap();
    assert!(
        matches!(second,Outcome::History {messages,..} if messages.len()==1 && messages[0].body=="12345")
    );
    assert_eq!(
        store
            .execute(
                &key.id,
                &Operation::Inbox {
                    cursor: Some(cursor.clone()),
                    limit: 1,
                    wait_ms: 0
                }
            )
            .unwrap_err()
            .code,
        "invalid-request"
    );
    let mut tampered = cursor.into_bytes();
    tampered[0] = if tampered[0] == b'0' { b'1' } else { b'0' };
    assert_eq!(
        store
            .execute(
                &key.id,
                &Operation::History {
                    conversation_id: id.clone(),
                    after: Some(String::from_utf8(tampered).unwrap()),
                    limit: 1
                }
            )
            .unwrap_err()
            .code,
        "invalid-request"
    );
    for limit in [0, 2] {
        assert_eq!(
            store
                .execute(
                    &key.id,
                    &Operation::History {
                        conversation_id: id.clone(),
                        after: None,
                        limit
                    }
                )
                .unwrap_err()
                .code,
            "invalid-request"
        );
    }
    assert_eq!(
        store
            .execute(
                &key.id,
                &Operation::Inbox {
                    cursor: None,
                    limit: 1,
                    wait_ms: 5001
                }
            )
            .unwrap_err()
            .code,
        "invalid-request"
    );
}
#[test]
fn concurrent_admission_cannot_overshoot_journal_capacity() {
    let temp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        Store::initialize(
            &temp.path().join("state.sqlite3"),
            Limits {
                journal_events: 6,
                ..Limits::default()
            },
        )
        .unwrap(),
    );
    let app = store.create_identity("app").unwrap();
    let agent = store.create_identity("agent").unwrap();
    let key = store
        .issue(
            &app.id,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 60,
        )
        .unwrap();
    let id = match store
        .execute(
            &key.id,
            &Operation::Create {
                request_id: "create".into(),
                request: connect_protocol::conversations::CreateRequest::persistent(vec![agent.id]),
            },
        )
        .unwrap()
    {
        Outcome::Conversation(c) => c.id,
        _ => panic!("create"),
    };
    let threads = (0..8)
        .map(|index| {
            let store = store.clone();
            let actor = key.id.clone();
            let id = id.clone();
            std::thread::spawn(move || {
                store.execute(
                    &actor,
                    &Operation::Send {
                        request_id: format!("send-{index}"),
                        conversation_id: id,
                        message: connect_protocol::messages::MessageDraft::text_all("bounded"),
                    },
                )
            })
        })
        .collect::<Vec<_>>();
    let results = threads
        .into_iter()
        .map(|t| t.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 2);
    assert!(results
        .iter()
        .filter_map(|r| r.as_ref().err())
        .all(|e| e.code == "capacity-exceeded"));
    let db = rusqlite::Connection::open(store.path()).unwrap();
    assert_eq!(
        db.query_row::<i64, _, _>("SELECT COUNT(*) FROM journal", [], |r| r.get(0))
            .unwrap(),
        6
    );
    assert_eq!(
        db.query_row::<i64, _, _>("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
            .unwrap(),
        2
    );
}

#[test]
fn current_authority_filters_inbox_without_hiding_storage_failure() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::initialize(&temp.path().join("state.sqlite3"), Limits::default()).unwrap();
    let app = store.create_identity("app").unwrap();
    let agent = store.create_identity("agent").unwrap();
    let outsider = store.create_identity("outsider").unwrap();
    let expiry = chrono::Utc::now().timestamp() + 60;
    let broad = store.issue(&app.id, Scope::operator(), expiry).unwrap();
    let id = match store
        .execute(
            &broad.id,
            &Operation::Create {
                request_id: "create".into(),
                request: connect_protocol::conversations::CreateRequest::persistent(vec![agent
                    .id
                    .clone()]),
            },
        )
        .unwrap()
    {
        Outcome::Conversation(c) => c.id,
        _ => panic!("create"),
    };
    let stranger = store
        .issue(&outsider.id, Scope::operator(), expiry)
        .unwrap();
    assert_eq!(
        store
            .execute(
                &stranger.id,
                &Operation::History {
                    conversation_id: id.clone(),
                    after: None,
                    limit: 100
                }
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    let peer = store
        .issue(&agent.id, Scope::messaging(vec![app.id.clone()]), expiry)
        .unwrap();
    store
        .execute(
            &peer.id,
            &Operation::Send {
                request_id: "private-from-peer".into(),
                conversation_id: id.clone(),
                message: connect_protocol::messages::MessageDraft::text_all("UNPERMITTED-SOURCE"),
            },
        )
        .unwrap();
    let narrow = store
        .issue(&app.id, Scope::messaging(vec![]), expiry)
        .unwrap();
    let inbox = store
        .execute(
            &narrow.id,
            &Operation::Inbox {
                cursor: None,
                limit: 100,
                wait_ms: 0,
            },
        )
        .unwrap();
    // Own Conversation metadata remains visible; the peer's source is
    // filtered before pagination under the narrowed recipient ceiling.
    assert!(
        matches!(inbox,Outcome::Inbox {events,cursor:Some(_)} if events.len()==1 && events[0].kind=="conversation_created" && events[0].message.is_none())
    );
    let db = rusqlite::Connection::open(store.path()).unwrap();
    // Deliberate corruption in a disposable fixture still surfaces storage
    // failure, rather than treating a broken policy view as an empty inbox.
    db.execute_batch("PRAGMA foreign_keys=OFF; DROP TABLE participants;")
        .unwrap();
    assert_eq!(
        store
            .execute(
                &broad.id,
                &Operation::Inbox {
                    cursor: None,
                    limit: 100,
                    wait_ms: 0
                }
            )
            .unwrap_err()
            .code,
        "storage-unavailable"
    );
}
