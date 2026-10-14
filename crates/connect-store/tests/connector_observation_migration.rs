//! Backup-gated schema 17 transition retains authority and connector contracts.
#![allow(clippy::unwrap_used)]
use connect_connector_api::{manifest::Manifest, Descriptor};
use connect_store::{Scope, Store};
#[test]
fn schema17_backup_migrates_connector_state_without_resetting_revocation_or_contracts() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("state.db");
    let store = Store::initialize(&path, Default::default()).unwrap();
    let owner = store.create_identity("owner").unwrap();
    let until = chrono::Utc::now().timestamp() + 600;
    let active = store.issue(&owner.id, Scope::operator(), until).unwrap();
    let revoked = store.issue(&owner.id, Scope::operator(), until).unwrap();
    let actor = store.authenticate(&active.token).unwrap();
    store.revoke(&actor, &revoked.id).unwrap();
    let descriptor = Descriptor::one_way("migration-fixture");
    let manifest = Manifest::messaging(descriptor);
    store
        .register_connector_manifest(descriptor, &manifest)
        .unwrap();
    let installation = store.installation_id().unwrap();
    drop(store);
    // Reconstruct the exact preceding schema by removing only new additive tables.
    let c = rusqlite::Connection::open(&path).unwrap();
    c.execute_batch("DROP TABLE connector_observations; DROP TABLE connector_receipt_events; DROP TABLE connector_incoming; DROP TABLE connector_voice_observations; PRAGMA user_version=17;").unwrap();
    drop(c);
    let backup = tmp.path().join("schema17-backup.db");
    let before = Store::backup_database(&path, &backup).unwrap();
    assert_eq!(before.schema, 17);
    let after = Store::migrate(&path, &backup, &before).unwrap();
    assert_eq!(after.schema, 18);
    assert_eq!(after.installation_id, installation);
    assert_eq!(Store::inspect_snapshot(&backup).unwrap(), before);
    let current = Store::open(&path, Default::default()).unwrap();
    assert!(current.authenticate(&active.token).is_ok());
    assert!(current.authenticate(&revoked.token).is_err());
    current
        .register_connector_manifest(descriptor, &manifest)
        .unwrap();
    let c = rusqlite::Connection::open(&path).unwrap();
    for table in [
        "connector_observations",
        "connector_receipt_events",
        "connector_incoming",
        "connector_voice_observations",
    ] {
        let count: u32 = c
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
}
