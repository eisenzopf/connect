#![allow(clippy::unwrap_used)] // Isolated deterministic test fixtures.
use connect_connector_api::{manifest::*, Descriptor};
use connect_store::Store;
#[test]
fn persisted_manifest_restart_rejects_drift_and_incompatible_capability() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("store.db");
    let descriptor = Descriptor::one_way("contributor");
    let manifest = Manifest::messaging(descriptor);
    {
        let store = Store::initialize(&path, connect_protocol::Limits::default()).unwrap();
        store
            .register_connector_manifest(descriptor, &manifest)
            .unwrap();
        store
            .register_connector_manifest(descriptor, &manifest)
            .unwrap();
    }
    let store = Store::open(&path, connect_protocol::Limits::default()).unwrap();
    store
        .register_connector_manifest(descriptor, &manifest)
        .unwrap();
    let mut changed = manifest.clone();
    changed.limits.message_bytes += 1;
    assert_eq!(
        store
            .register_connector_manifest(descriptor, &changed)
            .unwrap_err()
            .code,
        "conflict"
    );
    let mut invalid = manifest;
    invalid.video = true;
    assert_eq!(
        store
            .register_connector_manifest(descriptor, &invalid)
            .unwrap_err()
            .code,
        "invalid-request"
    );
}
#[test]
fn separate_voice_control_registration_persists_without_granting_message_route_contract() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("state.db");
    let mut manifest = Manifest::messaging(Descriptor::one_way("phone-fixture"));
    manifest.messaging = false;
    manifest.operations = [
        ConnectorOperation::VoiceStart,
        ConnectorOperation::VoiceStatus,
        ConnectorOperation::VoiceEnd,
    ]
    .into();
    {
        let store = Store::initialize(&path, connect_protocol::Limits::default()).unwrap();
        store.register_extension_manifest(&manifest).unwrap();
    }
    let store = Store::open(&path, connect_protocol::Limits::default()).unwrap();
    store.register_extension_manifest(&manifest).unwrap();
    let mut changed = manifest;
    changed.operations.insert(ConnectorOperation::VoiceDiscover);
    assert_eq!(
        store
            .register_extension_manifest(&changed)
            .unwrap_err()
            .code,
        "conflict"
    );
}
