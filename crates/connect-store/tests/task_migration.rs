//! Populated pre-Gate 4 state is preserved by a backup-gated, additive migration.
#![allow(clippy::unwrap_used)]
use connect_protocol::{Limits, Operation, Outcome};
use connect_store::{Store, SCHEMA_VERSION};
use rusqlite::{types::Value, Connection};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};

fn schema_ten(path: &Path) {
    let c = Connection::open(path).unwrap();
    for sql in [
        include_str!("../migrations/001_identity.sql"),
        include_str!("../migrations/002_conversations.sql"),
        include_str!("../migrations/003_providers.sql"),
        include_str!("../migrations/004_voice.sql"),
        include_str!("../migrations/005_inbound_voice.sql"),
        include_str!("../migrations/006_voice_recovery.sql"),
        include_str!("../migrations/007_sessions_context_artifacts.sql"),
        include_str!("../migrations/008_artifact_expiry.sql"),
        include_str!("../migrations/009_sip_legs.sql"),
        include_str!("../migrations/010_sms_eligibility.sql"),
    ] {
        c.execute_batch(sql).unwrap();
    }
    c.execute_batch(r#"
        INSERT INTO installation VALUES(1,'installation','private-cursor-key');
        INSERT INTO identities VALUES('owner','owner','["messaging"]');
        INSERT INTO credentials(id,identity_id,verifier,operations,recipients,expires_at) VALUES('credential','owner','verifier','["*"]','["*"]',2147483647);
        INSERT INTO conversations VALUES('conversation',0);
        INSERT INTO participants VALUES('conversation','owner');
        INSERT INTO messages(id,conversation_id,sender,body) VALUES('message','conversation','owner','A retained decision');
        INSERT INTO requests VALUES('owner','request','digest','send','["owner"]','{"Ok":{"kind":"accepted","data":{"request_id":"request","message_id":"message","conversation_id":"conversation"}}}');
        INSERT INTO journal(id,recipient,conversation_id,kind,message_id) VALUES('event','owner','conversation','message','message');
        INSERT INTO sessions VALUES('session','conversation','owner','owner','ended','["audio"]',100,101,'completed');
        INSERT INTO session_bindings VALUES('session','owner','credential','connection',NULL,'ended',101);
        INSERT INTO availability VALUES('owner','available',NULL,2147483647,100);
        INSERT INTO artifacts(id,conversation_id,owner_id,name,media_type,expected_size,expected_sha256,content,state,created_at,finalized_at) VALUES('artifact','conversation','owner','decision.txt','text/plain',3,'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad',X'616263','ready',100,101);
        INSERT INTO context_deliveries(id,conversation_id,session_id,provider,watermark,selection_json,state,created_at,updated_at,sequence) VALUES('delivery','conversation','session','vapi',1,'{}','delivered',100,101,1);
        INSERT INTO voice_routes VALUES('route','conversation','owner','assistant','phone','["+14155550101"]',1);
        INSERT INTO voice_calls(id,conversation_id,route_id,credential_id,destination,state,provider_id,direction) VALUES('call','conversation','route','credential','+14155550101','ended','provider-call','outbound');
        INSERT INTO sip_legs(id,conversation_id,session_id,credential_id,actor_id,destination,state,created_at,updated_at) VALUES('leg','conversation','session','credential','owner','sip:example@local','ended',100,101);
        INSERT INTO sms_authorities VALUES('rudeless',1,'authority-digest',2147483647);
        INSERT INTO sms_grants VALUES('rudeless','enrollment','+14155550101','+12702628454','conversation','reviewed-form-evidence',2147483647,1);
        INSERT INTO sms_controls VALUES('+14155550101',1,102,'stop-event');
        INSERT INTO sms_control_events VALUES('stop-event','+14155550101','+12702628454','STOP',102,1);
    "#).unwrap();
    // Ready schema-10 artifacts already use managed external blobs. Keep the
    // fixture valid, and prove both the database reference and actual bytes.
    let blobs = Store::blob_directory_for(path);
    std::fs::create_dir(&blobs).unwrap();
    let blob = "11111111-1111-4111-8111-111111111111.blob";
    std::fs::write(blobs.join(blob), b"abc").unwrap();
    c.execute("UPDATE artifacts SET blob_name=?1,content=X''", [blob])
        .unwrap();
    let canonical = serde_json::to_value(old_send()).unwrap();
    let digest = format!(
        "v1:{}",
        hex::encode(Sha256::digest(serde_json::to_vec(&canonical).unwrap()))
    );
    c.execute("UPDATE requests SET digest=?1", [digest])
        .unwrap();
    c.execute(
        "UPDATE credentials SET verifier=?1,expires_at=?2",
        rusqlite::params![
            hex::encode(Sha256::digest("a".repeat(64).as_bytes())),
            chrono::Utc::now().timestamp() + 3_600
        ],
    )
    .unwrap();
}
fn old_send() -> Operation {
    Operation::Send {
        request_id: "request".into(),
        conversation_id: "conversation".into(),
        body: "A retained decision".into(),
    }
}
fn rows(path: &Path) -> BTreeMap<String, Vec<Vec<Value>>> {
    let c = Connection::open(path).unwrap();
    let mut table_query=c.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE 'task%' AND name NOT IN ('media_end_intents','media_start_admissions','vapi_session_setups') ORDER BY name").unwrap();
    let names = table_query
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    names
        .into_iter()
        .map(|name| {
            // Schema 15 adds one nullable grant reference. Compare every original
            // field, and verify the new reference separately after migration.
            let projection = if name == "session_bindings" {
                "session_id,participant_id,credential_id,connection_id,stream_id,state,updated_at"
            } else {
                "*"
            };
            let mut query = c
                .prepare(&format!("SELECT {projection} FROM {name} ORDER BY 1,2"))
                .unwrap();
            let width = query.column_count();
            let data = query
                .query_map([], |r| {
                    (0..width)
                        .map(|i| r.get::<_, Value>(i))
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            (name, data)
        })
        .collect()
}
#[test]
fn upgrade_populated_ten_preserves_all_existing_rows_and_verified_backup() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("ten.sqlite3");
    schema_ten(&db);
    let before = rows(&db);
    let backup = dir.path().join("backup.sqlite3");
    let snapshot = Store::backup_database(&db, &backup).unwrap();
    assert_eq!(snapshot.schema, 10);
    assert_eq!(
        Store::open(&db, Limits::default()).err().unwrap().code,
        "unsupported-schema"
    );
    let upgraded = Store::migrate(&db, &backup, &snapshot).unwrap();
    assert_eq!(upgraded.schema, SCHEMA_VERSION);
    assert_eq!(rows(&db), before);
    assert_eq!(Store::inspect_snapshot(&backup).unwrap(), snapshot);
    let store = Store::open(&db, Limits::default()).unwrap();
    store.integrity_check().unwrap();
    assert_eq!(
        store.authenticate(&"a".repeat(64)).unwrap().identity_id,
        "owner"
    );
    assert!(
        matches!(store.execute("credential",&old_send()).unwrap(),Outcome::Accepted{message_id,..} if message_id=="message")
    );
    assert!(
        matches!(store.execute("credential",&Operation::History {conversation_id:"conversation".into(),after:None,limit:100}).unwrap(),Outcome::History {messages,..} if messages.len()==1 && messages[0].body=="A retained decision")
    );
    assert!(
        matches!(store.execute("credential",&Operation::SessionGet {conversation_id:"conversation".into(),session_id:"session".into()}).unwrap(),Outcome::Session(session) if session.state=="ended")
    );
    assert!(
        matches!(store.execute("credential",&Operation::ArtifactGet {conversation_id:"conversation".into(),artifact_id:"artifact".into(),offset:0,limit:3}).unwrap(),Outcome::ArtifactChunk(chunk) if chunk.data=="YWJj" && chunk.eof)
    );
    let c = Connection::open(&db).unwrap();
    assert_eq!(
        c.query_row::<u64, _, _>(
            "SELECT COUNT(*) FROM session_bindings WHERE task_grant_id IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap(),
        0,
        "Existing media bindings must not acquire invocation authority"
    );
    assert_eq!(
        c.query_row::<u64, _, _>("SELECT COUNT(*) FROM vapi_session_setups", [], |r| r.get(0))
            .unwrap(),
        0,
        "Schema-10 sessions have no task binding to invent"
    );
    let mut query = c.prepare("PRAGMA foreign_key_check").unwrap();
    assert!(!query.exists([]).unwrap());
    assert_eq!(
        c.query_row::<u64, _, _>("SELECT COUNT(*) FROM tasks", [], |r| r.get(0))
            .unwrap(),
        0
    );
}
#[test]
fn migration_failure_rolls_back_new_tables_and_version_without_touching_old_rows() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("ten.sqlite3");
    schema_ten(&db);
    // Deliberate schema conflict occurs after CREATE tasks, inside the migration transaction.
    Connection::open(&db)
        .unwrap()
        .execute_batch("CREATE TABLE task_events(existing TEXT)")
        .unwrap();
    let before = rows(&db);
    let backup = dir.path().join("backup.sqlite3");
    let snapshot = Store::backup_database(&db, &backup).unwrap();
    assert_eq!(
        Store::migrate(&db, &backup, &snapshot).unwrap_err().code,
        "storage-unavailable"
    );
    assert_eq!(Store::inspect_snapshot(&db).unwrap(), snapshot);
    assert_eq!(rows(&db), before);
    let c = Connection::open(&db).unwrap();
    assert_eq!(
        c.query_row::<u64, _, _>(
            "SELECT COUNT(*) FROM sqlite_master WHERE name='tasks'",
            [],
            |r| r.get(0)
        )
        .unwrap(),
        0
    );
}
#[test]
fn migration_rejects_stale_backup_before_task_schema_admission() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("ten.sqlite3");
    schema_ten(&db);
    let backup = dir.path().join("backup.sqlite3");
    let snapshot = Store::backup_database(&db, &backup).unwrap();
    Connection::open(&db)
        .unwrap()
        .execute("UPDATE messages SET body='Later committed decision'", [])
        .unwrap();
    assert_eq!(
        Store::migrate(&db, &backup, &snapshot).unwrap_err().code,
        "invalid-backup"
    );
    assert_eq!(Store::inspect_snapshot(&db).unwrap().schema, 10);
}
