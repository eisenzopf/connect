//! Authority checks run without reading or mutating any native Keychain.
#![allow(clippy::unwrap_used)]
use connect_client::ClientConfig;
use connect_connectors::setup::SetupSecretStore;
use connect_server::{
    cli::{first_run, init::private_replace},
    setup_vault::CheckpointVault,
};
use connect_store::Store;
#[test]
fn current_operator_metadata_installation_and_expiry_guard_vault_access() {
    let tmp = tempfile::tempdir().unwrap();
    let state = tmp.path().join("state");
    first_run::initialize(&state, "127.0.0.1:0".parse().unwrap()).unwrap();
    let operator_path = state.join("operator.toml");
    let server_path = state.join("server.toml");
    assert!(CheckpointVault::open(&state.join("agent.toml"), &server_path, None).is_err());
    let vault = CheckpointVault::open(&operator_path, &server_path, None).unwrap();
    let operation = "0123456789abcdef0123456789abcdef";
    let reference = vault.reference(operation).unwrap();
    assert!(vault.reference("malformed").is_err());
    let other = tmp.path().join("other");
    first_run::initialize(&other, "127.0.0.1:0".parse().unwrap()).unwrap();
    let foreign = CheckpointVault::open(
        &other.join("operator.toml"),
        &other.join("server.toml"),
        None,
    )
    .unwrap();
    assert!(foreign
        .read(&reference)
        .unwrap_err()
        .to_string()
        .contains("another installation"));
    let original = ClientConfig::load(&operator_path).unwrap();
    let mut wrong = original.clone();
    wrong.identity_id = "different-identity".into();
    private_replace(
        &operator_path,
        toml::to_string_pretty(&wrong).unwrap().as_bytes(),
    )
    .unwrap();
    assert!(CheckpointVault::open(&operator_path, &server_path, None).is_err());
    private_replace(
        &operator_path,
        toml::to_string_pretty(&original).unwrap().as_bytes(),
    )
    .unwrap();
    let database = state.join("state.sqlite3");
    let store = Store::open(&database, Default::default()).unwrap();
    let actor = store.authenticate(&original.credential).unwrap();
    store.revoke(&actor, &original.credential_id).unwrap();
    assert!(vault.reference(operation).is_err());
    assert!(vault.read(&reference).is_err());
    assert!(CheckpointVault::open(&operator_path, &server_path, None).is_err());
    let c = rusqlite::Connection::open(database).unwrap();
    c.execute(
        "UPDATE credentials SET revoked=0,expires_at=0 WHERE id=?1",
        [&original.credential_id],
    )
    .unwrap();
    assert!(vault.authorize().is_err());
}
