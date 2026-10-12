#![allow(clippy::unwrap_used)] // Fixture failures intentionally panic.
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
    assert_eq!(upgraded.schema, connect_store::SCHEMA_VERSION);
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
fn schema_six_upgrade_preserves_conversation_voice_receipt_and_journal_rows() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("schema-six.sqlite3");
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .execute_batch(include_str!("../migrations/001_identity.sql"))
        .unwrap();
    connection
        .execute_batch(include_str!("../migrations/002_conversations.sql"))
        .unwrap();
    connection
        .execute_batch(include_str!("../migrations/003_providers.sql"))
        .unwrap();
    connection
        .execute_batch(include_str!("../migrations/004_voice.sql"))
        .unwrap();
    connection
        .execute_batch(include_str!("../migrations/005_inbound_voice.sql"))
        .unwrap();
    connection
        .execute_batch(include_str!("../migrations/006_voice_recovery.sql"))
        .unwrap();
    connection
        .execute_batch(
            r#"
            INSERT INTO installation VALUES(1,'schema-six-installation','cursor-secret');
            INSERT INTO identities VALUES('app-id','app','["messaging"]');
            INSERT INTO credentials(id,identity_id,verifier,operations,recipients,expires_at) VALUES('credential-id','app-id','verifier','["*"]','["*"]',4102444800);
            INSERT INTO conversations VALUES('conversation-id',0);
            INSERT INTO participants VALUES('conversation-id','app-id');
            INSERT INTO messages(id,conversation_id,sender,body) VALUES('message-id','conversation-id','app-id','schema six message');
            INSERT INTO requests VALUES('app-id','request-id','digest','send','["app-id"]','{"Ok":{"kind":"accepted","data":{"request_id":"request-id","message_id":"message-id","conversation_id":"conversation-id"}}}');
            INSERT INTO journal(id,recipient,conversation_id,kind,message_id,call_json) VALUES('journal-id','app-id','conversation-id','message','message-id',NULL);
            INSERT INTO voice_routes VALUES('route-id','conversation-id','app-id','12345678-1234-1234-1234-123456789012','12345678-1234-1234-1234-123456789013','["+14155550101"]',1);
            INSERT INTO voice_calls(id,conversation_id,route_id,credential_id,destination,state,provider_id,direction,end_requested,next_poll) VALUES('call-id','conversation-id','route-id','credential-id','+14155550101','ended','provider-id','outbound',0,0);
            "#,
        )
        .unwrap();
    drop(connection);

    let backup = dir.path().join("schema-six-backup.sqlite3");
    let expected = Store::backup_database(&database, &backup).unwrap();
    assert_eq!(expected.schema, 6);
    let upgraded = Store::migrate(&database, &backup, &expected).unwrap();
    assert_eq!(upgraded.schema, connect_store::SCHEMA_VERSION);
    assert_eq!(Store::inspect_snapshot(&backup).unwrap(), expected);

    let migrated = rusqlite::Connection::open(&database).unwrap();
    assert_eq!(
        migrated
            .query_row::<String, _, _>(
                "SELECT body FROM messages WHERE id='message-id'",
                [],
                |row| row.get(0),
            )
            .unwrap(),
        "schema six message"
    );
    assert_eq!(
        migrated
            .query_row::<String, _, _>(
                "SELECT result FROM requests WHERE id='request-id'",
                [],
                |row| row.get(0),
            )
            .unwrap(),
        "{\"Ok\":{\"kind\":\"accepted\",\"data\":{\"request_id\":\"request-id\",\"message_id\":\"message-id\",\"conversation_id\":\"conversation-id\"}}}"
    );
    assert_eq!(
        migrated
            .query_row::<String, _, _>(
                "SELECT state FROM voice_calls WHERE id='call-id'",
                [],
                |row| row.get(0),
            )
            .unwrap(),
        "ended"
    );
    assert_eq!(
        migrated
            .query_row::<i64, _, _>("SELECT COUNT(*) FROM journal", [], |row| row.get(0))
            .unwrap(),
        1
    );
    for table in [
        "sessions",
        "availability",
        "artifacts",
        "context_deliveries",
    ] {
        assert_eq!(
            migrated
                .query_row::<i64, _, _>(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap(),
            0
        );
    }
}

#[test]
fn schema_seven_upgrade_adds_expiry_without_changing_artifact_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("schema-seven.sqlite3");
    let connection = rusqlite::Connection::open(&database).unwrap();
    for migration in [
        include_str!("../migrations/001_identity.sql"),
        include_str!("../migrations/002_conversations.sql"),
        include_str!("../migrations/003_providers.sql"),
        include_str!("../migrations/004_voice.sql"),
        include_str!("../migrations/005_inbound_voice.sql"),
        include_str!("../migrations/006_voice_recovery.sql"),
        include_str!("../migrations/007_sessions_context_artifacts.sql"),
    ] {
        connection.execute_batch(migration).unwrap();
    }
    connection
        .execute_batch(
            r#"
            INSERT INTO installation VALUES(1,'schema-seven-installation','cursor-secret');
            INSERT INTO identities VALUES('owner-id','owner','["messaging"]');
            INSERT INTO conversations VALUES('conversation-id',0);
            INSERT INTO participants VALUES('conversation-id','owner-id');
            INSERT INTO artifacts(id,conversation_id,owner_id,name,media_type,expected_size,expected_sha256,content,blob_name,state,created_at,finalized_at)
            VALUES('artifact-id','conversation-id','owner-id','draft.txt','text/plain',5,'digest',X'6472616674',NULL,'uploading',1,NULL);
            "#,
        )
        .unwrap();
    drop(connection);

    let backup = dir.path().join("schema-seven-backup.sqlite3");
    let expected = Store::backup_database(&database, &backup).unwrap();
    assert_eq!(expected.schema, 7);
    let upgraded = Store::migrate(&database, &backup, &expected).unwrap();
    assert_eq!(upgraded.schema, connect_store::SCHEMA_VERSION);
    let migrated = rusqlite::Connection::open(&database).unwrap();
    assert_eq!(
        migrated
            .query_row::<(Vec<u8>, Option<i64>), _, _>(
                "SELECT content,expires_at FROM artifacts WHERE id='artifact-id'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap(),
        (b"draft".to_vec(), None)
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
