//! Gate 6 schema, audience epochs, backup fidelity and incompatible-state safety.
#![allow(clippy::unwrap_used)]
use connect_protocol::Limits;
use connect_store::{Store, SCHEMA_VERSION};
use rusqlite::Connection;

fn seed(c: &Connection) {
    c.execute_batch(
        "PRAGMA foreign_keys=ON;
         INSERT INTO identities(id,alias,capabilities) VALUES('id_alice','alice','[]'),('id_bob','bob','[]');
         INSERT INTO conversations(id) VALUES('conv_one');
         INSERT INTO participants(conversation_id,identity_id,participant_id,role)
          VALUES('conv_one','id_alice','part_alice','agent'),('conv_one','id_bob','part_bob','customer');
         INSERT INTO devices(id,identity_id,tenant_id,kind,platform) VALUES('dev_alice','id_alice','tnt_default','browser','fixture');
         INSERT INTO endpoints(id,identity_id,tenant_id,device_id,transport,address,capabilities_json,verification_state,verification_evidence,verified_at)
          VALUES('ep_alice','id_alice','tnt_default','dev_alice','webrtc','private-endpoint-address','{}','verified','proof_one',100);
         INSERT INTO sessions(id,conversation_id,initiator_id,participant_id,state,media,created_at,updated_at)
          VALUES('sess_one','conv_one','id_alice','id_bob','active','[\"audio\"]',100,100);
         INSERT INTO session_participants VALUES('sess_one','conv_one','tnt_default','part_alice',1,'agent','present');
         INSERT INTO connections(id,session_id,participant_id,membership_epoch,identity_id,tenant_id,endpoint_id,generation,state,observed_runtime_id,observed_at,readiness_json)
          VALUES('conn_retained','sess_one','part_alice',1,'id_alice','tnt_default','ep_alice',3,'active','old_runtime',100,'{}');
         INSERT INTO messages(id,conversation_id,sender,body,sender_participant_id,audience_mode) VALUES('msg_private','conv_one','id_alice','private content','part_alice','explicit');
         INSERT INTO message_audiences VALUES('msg_private','conv_one','part_alice',1,'sender'),('msg_private','conv_one','part_bob',1,'recipient');
         INSERT INTO operations(record_id,tenant_id,owner_id,operation_id,digest,command_type,conversation_id,session_id,connection_id,intent_json,state,created_at,updated_at)
          VALUES('opr_one','tnt_default','id_alice','op_one','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','connection.replace','conv_one','sess_one','conn_retained','{}','unknown',100,100);
         INSERT INTO operation_resources VALUES('opr_one','tnt_default','session','sess_one',1);
         INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,attempt,provider_ref,observation_json) VALUES('effect_one','opr_one','fixture','correlation_one','none','unknown',1,'provider_reference',NULL);
         INSERT INTO operation_observations VALUES('observation_one','opr_one','fixture','event_one','bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb','id_alice','{}',100);",
    ).unwrap();
}

#[test]
fn gate6_backup_preserves_ids_audiences_unknown_effects_and_fences_live_claims() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.sqlite3");
    let store = Store::initialize(&path, Limits::default()).unwrap();
    assert_eq!(
        Store::inspect_snapshot(&path).unwrap().schema,
        SCHEMA_VERSION
    );
    let c = Connection::open(&path).unwrap();
    seed(&c);
    let backup = dir.path().join("backup.sqlite3");
    let snapshot = store.backup(&backup).unwrap();
    assert_eq!(Store::inspect_snapshot(&backup).unwrap(), snapshot);
    let original_backup = std::fs::read(&backup).unwrap();
    let restored_path = dir.path().join("restored.sqlite3");
    std::fs::copy(&backup, &restored_path).unwrap();
    let restored = Store::open(&restored_path, Limits::default()).unwrap();
    assert_eq!(
        restored.installation_id().unwrap(),
        snapshot.installation_id
    );
    assert_ne!(restored.runtime_epoch(), store.runtime_epoch());
    let c = Connection::open(&restored_path).unwrap();
    let connection: (String, u64, Option<String>) = c
        .query_row(
            "SELECT state,generation,observed_runtime_id FROM connections WHERE id='conn_retained'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(connection, ("interrupted".into(), 4, None));
    assert_eq!(
        c.query_row(
            "SELECT state FROM operations WHERE operation_id='op_one'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "unknown"
    );
    assert_eq!(
        c.query_row(
            "SELECT state FROM operation_effects WHERE id='effect_one'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "unknown"
    );
    assert_eq!(
        c.query_row(
            "SELECT membership_epoch FROM message_audiences WHERE participant_id='part_bob'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(std::fs::read(&backup).unwrap(), original_backup);
    assert_eq!(Store::inspect_snapshot(&backup).unwrap(), snapshot);
}

#[test]
fn gate6_incompatible_state_refusal_preserves_files_and_requires_explicit_new_path() {
    for previous_version in 18..SCHEMA_VERSION {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("previous.sqlite3");
        let store = Store::initialize(&path, Limits::default()).unwrap();
        store.create_identity("preserve-me").unwrap();
        drop(store);
        let c = Connection::open(&path).unwrap();
        // This fixture changes the version header rather than running an old
        // binary. Restore its removed receipt table so historical backup
        // inspection has the shape belonging to that header.
        if previous_version < 32 {
            c.execute_batch("CREATE TABLE requests(actor TEXT,id TEXT,digest TEXT,operation TEXT,resources TEXT,result TEXT,PRIMARY KEY(actor,id))").unwrap();
        }
        c.execute_batch(&format!("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE; PRAGMA user_version={previous_version};"))
    .unwrap();
        drop(c);
        let original = std::fs::read(&path).unwrap();
        let error = Store::open(&path, Limits::default()).err().unwrap();
        assert_eq!(error.code, "unsupported-schema");
        assert!(
            error.message.contains("backup") && error.message.contains("new Gate 6 state path")
        );
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert!(Store::initialize(&path, Limits::default()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        let backup = dir.path().join("preserved.sqlite3");
        let before = Store::backup_database(&path, &backup).unwrap();
        assert_eq!(
            Store::migrate(&path, &backup, &before).unwrap_err().code,
            "unsupported-schema"
        );
        assert_eq!(Store::inspect_snapshot(&backup).unwrap(), before);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        let fresh =
            Store::initialize(&dir.path().join("new-gate6.sqlite3"), Limits::default()).unwrap();
        assert_ne!(fresh.installation_id().unwrap(), before.installation_id);
    }
}

#[test]
fn gate6_constraints_reject_rebinding_stale_epoch_cross_owner_and_false_completion() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.sqlite3");
    let _store = Store::initialize(&path, Limits::default()).unwrap();
    let c = Connection::open(&path).unwrap();
    seed(&c);
    for statement in [
        "UPDATE participants SET identity_id='id_bob' WHERE participant_id='part_alice'",
        "UPDATE participants SET membership_epoch=2 WHERE participant_id='part_alice'",
        "UPDATE conversations SET revision=9007199254740992",
        "UPDATE operations SET result_json='{\"completed\":true}' WHERE state='unknown'",
        "INSERT INTO connections(id,session_id,participant_id,membership_epoch,identity_id,tenant_id,endpoint_id,generation,state) VALUES('conn_stolen','sess_one','part_alice',1,'id_bob','tnt_default','ep_alice',1,'preparing')",
        "INSERT INTO message_receipts(message_id,participant_id,membership_epoch,channel,delivery_id,kind,provenance,observed_at) VALUES('msg_private','part_alice',2,'native','part_alice','read','client_reported',100)",
        "INSERT INTO connections(id,session_id,participant_id,membership_epoch,identity_id,tenant_id,endpoint_id,generation,state,replaces) VALUES('conn_stale','sess_one','part_alice',1,'id_alice','tnt_default','ep_alice',2,'preparing','conn_retained')",
    ] { assert!(c.execute_batch(statement).is_err(),"constraint accepted: {statement}"); }
    c.execute(
        "UPDATE participants SET active=0 WHERE participant_id='part_bob'",
        [],
    )
    .unwrap();
    assert!(c
        .execute(
            "UPDATE participants SET active=1 WHERE participant_id='part_bob'",
            []
        )
        .is_err());
    c.execute(
        "UPDATE participants SET active=1,membership_epoch=2 WHERE participant_id='part_bob'",
        [],
    )
    .unwrap();
    assert_eq!(
        c.query_row(
            "SELECT membership_epoch FROM message_audiences WHERE participant_id='part_bob'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        c.query_row(
            "SELECT membership_epoch FROM participants WHERE participant_id='part_bob'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        2
    );
    assert!(c
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .query([])
        .unwrap()
        .next()
        .unwrap()
        .is_none());
}

#[test]
fn gate6_snapshot_digest_covers_new_private_and_effect_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.sqlite3");
    let _store = Store::initialize(&path, Limits::default()).unwrap();
    let c = Connection::open(&path).unwrap();
    seed(&c);
    let mut before = Store::inspect_snapshot(&path).unwrap();
    for statement in [
        "UPDATE endpoints SET verification_evidence='changed-private-proof'",
        "UPDATE operation_effects SET provider_ref='changed-provider-reference'",
        "UPDATE operation_observations SET data_json='{\"changed\":true}'",
        "UPDATE message_audiences SET purpose='sender' WHERE participant_id='part_bob'",
        "UPDATE operation_resources SET fencing_generation=2",
        "UPDATE message_receipts SET observed_at=observed_at+1",
        "INSERT INTO receipt_preferences VALUES('part_bob',1,1,1)",
    ] {
        c.execute_batch(statement).unwrap();
        let after = Store::inspect_snapshot(&path).unwrap();
        assert_ne!(before.digest, after.digest);
        before = after;
    }
}
