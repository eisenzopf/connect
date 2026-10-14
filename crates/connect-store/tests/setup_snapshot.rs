//! Setup metadata participates in backup freshness; it never escapes current authority.
#![allow(clippy::unwrap_used)]
use connect_connector_api::{manifest::Manifest, Descriptor};
use connect_protocol::{Operation, Outcome};
use connect_store::{Scope, Store};
#[test]
fn schema17_snapshot_detects_contact_and_manifest_drift_and_restores_current_revocation() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("state.db");
    let store = Store::initialize(&path, Default::default()).unwrap();
    let owner = store.create_identity("owner").unwrap();
    let peer = store.create_identity("peer").unwrap();
    let until = chrono::Utc::now().timestamp() + 600;
    let owner_key = store.issue(&owner.id, Scope::operator(), until).unwrap();
    let peer_key = store.issue(&peer.id, Scope::operator(), until).unwrap();
    let pairing = match store
        .execute(
            &owner_key.id,
            &Operation::ContactPairingStart {
                request_id: "pair".into(),
                peer_identity_id: peer.id.clone(),
                expires_at: until,
            },
        )
        .unwrap()
    {
        Outcome::ContactPairing(p) => p,
        _ => panic!("pairing"),
    };
    let descriptor = Descriptor::one_way("snapshot-fixture");
    let manifest = Manifest::messaging(descriptor);
    store
        .register_connector_manifest(descriptor, &manifest)
        .unwrap();
    let saved_path = tmp.path().join("snapshot.db");
    let saved = store.backup(&saved_path).unwrap();
    let connection = rusqlite::Connection::open(&path).unwrap();
    // No journal or request row changes: detect each setup table independently.
    connection
        .execute("UPDATE contact_pairings SET attempts=attempts+1", [])
        .unwrap();
    assert_ne!(Store::inspect_snapshot(&path).unwrap().digest, saved.digest);
    assert_eq!(
        Store::migrate(&path, &saved_path, &saved).unwrap_err().code,
        "invalid-backup"
    );
    connection
        .execute("UPDATE contact_pairings SET attempts=attempts-1", [])
        .unwrap();
    assert_eq!(Store::inspect_snapshot(&path).unwrap(), saved);
    connection
        .execute("UPDATE connector_manifests SET manifest='{}'", [])
        .unwrap();
    assert_ne!(Store::inspect_snapshot(&path).unwrap().digest, saved.digest);
    assert_eq!(
        Store::migrate(&path, &saved_path, &saved).unwrap_err().code,
        "invalid-backup"
    );
    assert_eq!(Store::inspect_snapshot(&saved_path).unwrap(), saved);
    let restored = Store::open(&saved_path, Default::default()).unwrap();
    restored
        .register_connector_manifest(descriptor, &manifest)
        .unwrap();
    let complete = Operation::ContactPairingComplete {
        request_id: "complete".into(),
        pairing_id: pairing.id,
        challenge: pairing.challenge.unwrap(),
    };
    restored.execute(&peer_key.id, &complete).unwrap();
    let actor = restored.authenticate(&peer_key.token).unwrap();
    restored.revoke(&actor, &owner_key.id).unwrap();
    assert!(restored.execute(&peer_key.id, &complete).is_err());
}
