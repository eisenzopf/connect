#![allow(clippy::unwrap_used)]
use connect_protocol::Limits;
use connect_store::Store;

#[test]
fn schema_nine_upgrade_is_additive_backup_gated_and_preserves_existing_rows() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("legacy.sqlite3");
    let c = rusqlite::Connection::open(&database).unwrap();
    for migration in [
        include_str!("../migrations/001_identity.sql"),
        include_str!("../migrations/002_conversations.sql"),
        include_str!("../migrations/003_providers.sql"),
        include_str!("../migrations/004_voice.sql"),
        include_str!("../migrations/005_inbound_voice.sql"),
        include_str!("../migrations/006_voice_recovery.sql"),
        include_str!("../migrations/007_sessions_context_artifacts.sql"),
        include_str!("../migrations/008_artifact_expiry.sql"),
        include_str!("../migrations/009_sip_legs.sql"),
    ] {
        c.execute_batch(migration).unwrap();
    }
    c.execute_batch("INSERT INTO installation VALUES(1,'preserved-installation','private-cursor-key'); INSERT INTO identities VALUES('human','human','[\"messaging\"]'); INSERT INTO conversations VALUES('conversation',0); INSERT INTO participants VALUES('conversation','human'); INSERT INTO messages(id,conversation_id,sender,body) VALUES('message','conversation','human','Preserve this conversation');").unwrap();
    drop(c);
    assert!(Store::open(&database, Limits::default()).is_err());
    let backup = dir.path().join("verified-backup.sqlite3");
    let snapshot = Store::backup_database(&database, &backup).unwrap();
    assert_eq!(snapshot.schema, 9);
    let upgraded = Store::migrate(&database, &backup, &snapshot).unwrap();
    assert_eq!(upgraded.schema, connect_store::SCHEMA_VERSION);
    assert_eq!(Store::inspect_snapshot(&backup).unwrap(), snapshot);
    let store = Store::open(&database, Limits::default()).unwrap();
    assert_eq!(store.installation_id().unwrap(), "preserved-installation");
    store.integrity_check().unwrap();
    let c = rusqlite::Connection::open(&database).unwrap();
    assert_eq!(
        c.query_row::<String, _, _>("SELECT body FROM messages WHERE id='message'", [], |r| r
            .get(0))
            .unwrap(),
        "Preserve this conversation"
    );
    assert_eq!(
        c.query_row::<i64, _, _>("SELECT COUNT(*) FROM sms_grants", [], |r| r.get(0))
            .unwrap(),
        0
    );
}
