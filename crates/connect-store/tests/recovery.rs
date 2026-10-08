use connect_protocol::{Limits, Operation, Outcome};
use connect_store::{Scope, Store};

#[test]
fn online_snapshot_recovers_wal_receipts_events_and_cursors() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap();
    let identity = store.create_identity("app").unwrap();
    let key = store
        .issue(
            &identity.id,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 60,
        )
        .unwrap();
    let conversation = match store
        .execute(
            &key.id,
            &Operation::Create {
                request_id: "create".into(),
                participants: vec![],
            },
        )
        .unwrap()
    {
        Outcome::Conversation(c) => c.id,
        _ => panic!("create"),
    };
    let send = Operation::Send {
        request_id: "send".into(),
        conversation_id: conversation.clone(),
        body: "committed in WAL".into(),
    };
    let accepted = store.execute(&key.id, &send).unwrap();
    let inbox = Operation::Inbox {
        cursor: None,
        limit: 100,
        wait_ms: 0,
    };
    let page = store.execute(&key.id, &inbox).unwrap();
    let snapshot = store.backup(&dir.path().join("snapshot.sqlite3")).unwrap();
    assert_eq!(
        snapshot,
        Store::inspect_snapshot(&dir.path().join("snapshot.sqlite3")).unwrap()
    );
    assert!(store.backup(&dir.path().join("snapshot.sqlite3")).is_err());
    let restored = Store::open(&dir.path().join("snapshot.sqlite3"), Limits::default()).unwrap();
    assert_eq!(
        restored.authenticate(&key.token).unwrap().identity_id,
        identity.id
    );
    assert_eq!(restored.execute(&key.id, &send).unwrap(), accepted);
    assert_eq!(restored.execute(&key.id, &inbox).unwrap(), page);
    let cursor = match page {
        Outcome::Inbox { cursor, .. } => cursor,
        _ => panic!("inbox"),
    };
    assert!(
        matches!(restored.execute(&key.id,&Operation::Inbox {cursor,limit:100,wait_ms:0}).unwrap(),Outcome::Inbox {events,..} if events.is_empty())
    );
}
fn legacy(path: &std::path::Path) {
    let c = rusqlite::Connection::open(path).unwrap();
    c.execute_batch(include_str!("../migrations/001_identity.sql"))
        .unwrap();
    c.execute(
        "INSERT INTO installation VALUES(1,'legacy-id','legacy-key')",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO identities VALUES('identity-id','app','[\"messaging\"]')",
        [],
    )
    .unwrap();
}
#[test]
fn migration_requires_matching_backup_and_refuses_newer_schema() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("legacy.sqlite3");
    legacy(&database);
    let backup = dir.path().join("backup.sqlite3");
    let expected = Store::backup_database(&database, &backup).unwrap();
    assert_eq!(expected.schema, 1);
    let c = rusqlite::Connection::open(&database).unwrap();
    c.execute("INSERT INTO identities VALUES('another','agent','[]')", [])
        .unwrap();
    assert_eq!(
        Store::migrate(&database, &backup, &expected)
            .unwrap_err()
            .code,
        "invalid-backup"
    );
    assert_eq!(Store::inspect_snapshot(&database).unwrap().schema, 1);
    c.execute("DELETE FROM identities WHERE id='another'", [])
        .unwrap();
    let upgraded = Store::migrate(&database, &backup, &expected).unwrap();
    assert_eq!(upgraded.schema, 2);
    assert_eq!(upgraded.installation_id, expected.installation_id);
    assert_eq!(
        Store::open(&database, Limits::default())
            .unwrap()
            .identity("identity-id")
            .unwrap()
            .alias,
        "app"
    );
    assert_eq!(Store::inspect_snapshot(&backup).unwrap(), expected);
    c.pragma_update(None, "user_version", 99).unwrap();
    assert_eq!(
        Store::inspect_snapshot(&database).unwrap_err().code,
        "unsupported-schema"
    );
}
#[test]
fn receipt_capacity_bounds_rejected_operations_without_losing_existing_receipts() {
    let dir = tempfile::tempdir().unwrap();
    let limits = Limits {
        operation_receipts: 2,
        ..Limits::default()
    };
    let store = Store::initialize(&dir.path().join("state.sqlite3"), limits).unwrap();
    let app = store.create_identity("app").unwrap();
    let key = store
        .issue(
            &app.id,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 60,
        )
        .unwrap();
    let create = Operation::Create {
        request_id: "create".into(),
        participants: vec![],
    };
    let first = store.execute(&key.id, &create).unwrap();
    let id = match &first {
        Outcome::Conversation(c) => c.id.clone(),
        _ => panic!("create"),
    };
    assert_eq!(
        store
            .execute(
                &key.id,
                &Operation::Send {
                    request_id: "invalid".into(),
                    conversation_id: id.clone(),
                    body: String::new()
                }
            )
            .unwrap_err()
            .code,
        "invalid-request"
    );
    assert_eq!(
        store
            .execute(
                &key.id,
                &Operation::Send {
                    request_id: "new".into(),
                    conversation_id: id,
                    body: "valid".into()
                }
            )
            .unwrap_err()
            .code,
        "capacity-exceeded"
    );
    assert_eq!(store.execute(&key.id, &create).unwrap(), first);
    assert!(
        matches!(store.execute(&key.id,&Operation::RequestStatus {request_id:"new".into()}).unwrap(),Outcome::RequestStatus {state,..} if state=="not_found")
    );
}
