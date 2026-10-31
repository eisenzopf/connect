//! Recovery must retain signed identities and reject incomplete/unsafe material.
#![cfg(unix)]
#![allow(clippy::unwrap_used)]
use connect_client::canonical::{CanonicalClient, CanonicalClientConfig};
use connect_server::{
    canonical::{CanonicalServerConfig, RunningCanonicalServer},
    cli::{canonical_init, canonical_recovery as recovery},
};
use std::path::Path;

fn init(root: &Path) {
    canonical_init::initialize(root, false, "127.0.0.1:0".parse().unwrap()).unwrap();
}
fn backup(root: &Path, target: &Path) -> anyhow::Result<()> {
    recovery::backup(
        &root.join("server.toml"),
        &root.join("operator.toml"),
        target,
    )
}
#[tokio::test]
async fn stopped_backup_restore_preserves_enrolled_keys_and_authenticates_after_restart() {
    let temp = tempfile::tempdir().unwrap();
    let original = temp.path().join("original");
    init(&original);
    let online = RunningCanonicalServer::start(
        CanonicalServerConfig::load(&original.join("server.toml")).unwrap(),
    )
    .await
    .unwrap();
    let refused = temp.path().join("online-backup");
    assert!(backup(&original, &refused).is_err());
    assert!(!refused.exists());
    online.shutdown().await.unwrap();
    let saved = temp.path().join("backup");
    backup(&original, &saved).unwrap();
    let manifest = recovery::verified_manifest(&saved).unwrap();
    let restored = temp.path().join("restored");
    recovery::restore(&saved, &restored).unwrap();
    assert_eq!(
        connect_store::Store::inspect_snapshot(&restored.join("state.sqlite3")).unwrap(),
        manifest.snapshot
    );
    // Removing the original proves every trust/signing reference is portable.
    std::fs::remove_dir_all(original).unwrap();
    std::fs::remove_dir_all(saved).unwrap();
    for _ in 0..2 {
        let server = RunningCanonicalServer::start(
            CanonicalServerConfig::load(&restored.join("server.toml")).unwrap(),
        )
        .await
        .unwrap();
        let mut config = CanonicalClientConfig::load(&restored.join("operator.toml")).unwrap();
        config.endpoint = server.address;
        let mut client = CanonicalClient::open(config).await.unwrap();
        client.list_conversations(Default::default()).await.unwrap();
        client.close();
        server.shutdown().await.unwrap();
    }
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&restored).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(restored.join("operator.toml"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}
#[test]
fn refusal_never_publishes_partial_state_or_replaces_existing_destinations() {
    let temp = tempfile::tempdir().unwrap();
    let original = temp.path().join("original");
    init(&original);
    let saved = temp.path().join("backup");
    backup(&original, &saved).unwrap();
    assert!(backup(&original, &saved).is_err());
    let occupied = temp.path().join("occupied");
    std::fs::create_dir(&occupied).unwrap();
    std::fs::write(occupied.join("sentinel"), b"keep").unwrap();
    assert!(recovery::restore(&saved, &occupied).is_err());
    assert_eq!(std::fs::read(occupied.join("sentinel")).unwrap(), b"keep");
    let asset = recovery::verified_manifest(&saved)
        .unwrap()
        .files
        .keys()
        .find(|p| p.starts_with("asset-"))
        .unwrap()
        .clone();
    let bytes = std::fs::read(saved.join(&asset)).unwrap();
    std::fs::write(saved.join(&asset), b"corrupt").unwrap();
    let rejected = temp.path().join("rejected");
    assert!(recovery::restore(&saved, &rejected).is_err());
    assert!(!rejected.exists());
    std::fs::write(saved.join(&asset), &bytes).unwrap();
    std::fs::remove_file(saved.join(&asset)).unwrap();
    std::os::unix::fs::symlink(original.join("certificate.der"), saved.join(&asset)).unwrap();
    assert!(recovery::restore(&saved, &rejected).is_err());
    assert!(!rejected.exists());
    std::fs::remove_file(saved.join(&asset)).unwrap();
    std::fs::write(saved.join(&asset), bytes).unwrap();
    let manifest = saved.join("manifest.json");
    let mut json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    json["files"]["../outside"] = serde_json::json!("invalid");
    std::fs::write(&manifest, serde_json::to_vec(&json).unwrap()).unwrap();
    assert!(recovery::restore(&saved, &rejected).is_err());
    assert!(!rejected.exists());
    assert!(!std::fs::read_dir(temp.path()).unwrap().any(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".connect-recovery-")));
}
#[test]
fn canonical_backup_refuses_unrecognized_operator_credentials() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("original");
    init(&root);
    let config = root.join("operator.toml");
    let mut c = CanonicalClientConfig::load(&config).unwrap();
    c.credential = "0".repeat(64);
    std::fs::write(&config, toml::to_string(&c).unwrap()).unwrap();
    let output = temp.path().join("backup");
    assert!(backup(&root, &output).is_err());
    assert!(!output.exists());
}
