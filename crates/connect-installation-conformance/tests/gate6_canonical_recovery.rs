#![recursion_limit = "256"]
//! Recovery must retain signed identities and reject incomplete/unsafe material.
#![cfg(unix)]
#![allow(clippy::unwrap_used)]
use base64::{engine::general_purpose::STANDARD, Engine as _};
use connect_client::canonical::{CanonicalClient, CanonicalClientConfig};
use connect_protocol::{Operation, Outcome};
use connect_server::{
    canonical::{CanonicalServerConfig, RunningCanonicalServer},
    cli::{canonical_init, canonical_recovery as recovery},
};
use connect_store::Store;
use sha2::{Digest, Sha256};
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
const ATTACHMENT: &[u8] = b"Canonical recovery preserves finalized file bytes outside SQLite.";
struct StoredAttachment {
    conversation: String,
    artifact: String,
}
fn seed_attachment(root: &Path, expires_at: Option<i64>) -> StoredAttachment {
    let server = CanonicalServerConfig::load(&root.join("server.toml")).unwrap();
    let operator = CanonicalClientConfig::load(&root.join("operator.toml")).unwrap();
    let store = Store::open(&server.database, server.limits).unwrap();
    let actor = store.authenticate(&operator.credential).unwrap();
    let mut request = connect_protocol::conversations::CreateRequest::persistent(vec![]);
    request.tenant_id = Some("tnt_default".into());
    let Outcome::Conversation(conversation) = store
        .execute(
            &actor.credential_id,
            &Operation::Create {
                request_id: "recovery-file-room".into(),
                request,
            },
        )
        .unwrap()
    else {
        panic!("Expected fixture conversation")
    };
    let Outcome::Artifact(artifact) = store
        .execute(
            &actor.credential_id,
            &Operation::ArtifactBegin {
                to: connect_protocol::messages::MessageAudience::all(),
                request_id: "recovery-file-begin".into(),
                conversation_id: conversation.id.clone(),
                name: "proof.txt".into(),
                media_type: "text/plain".into(),
                size: ATTACHMENT.len() as u64,
                sha256: hex::encode(Sha256::digest(ATTACHMENT)),
                expires_at,
            },
        )
        .unwrap()
    else {
        panic!("Expected fixture attachment")
    };
    store
        .execute(
            &actor.credential_id,
            &Operation::ArtifactAppend {
                request_id: "recovery-file-append".into(),
                conversation_id: conversation.id.clone(),
                artifact_id: artifact.id.clone(),
                offset: 0,
                data: STANDARD.encode(ATTACHMENT),
            },
        )
        .unwrap();
    store
        .execute(
            &actor.credential_id,
            &Operation::ArtifactFinish {
                request_id: "recovery-file-finish".into(),
                conversation_id: conversation.id.clone(),
                artifact_id: artifact.id.clone(),
            },
        )
        .unwrap();
    assert_eq!(
        std::fs::read(store.blob_directory().join(format!("{}.blob", artifact.id))).unwrap(),
        ATTACHMENT
    );
    StoredAttachment {
        conversation: conversation.id,
        artifact: artifact.id,
    }
}
#[tokio::test]
async fn stopped_backup_restore_preserves_enrolled_keys_and_authenticates_after_restart() {
    let temp = tempfile::tempdir().unwrap();
    let original = temp.path().join("original");
    init(&original);
    let attachment = seed_attachment(&original, None);
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
    assert_eq!(manifest.blobs.len(), 1);
    assert!(manifest
        .blobs
        .contains_key(&format!("{}.blob", attachment.artifact)));
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
        let rooms = client.list_conversations(Default::default()).await.unwrap();
        assert!(rooms
            .conversations
            .iter()
            .any(|room| room.id == attachment.conversation));
        let operator = CanonicalClientConfig::load(&restored.join("operator.toml")).unwrap();
        let actor = server.store.authenticate(&operator.credential).unwrap();
        let chunk = server
            .store
            .execute(
                &actor.credential_id,
                &Operation::ArtifactGet {
                    conversation_id: attachment.conversation.clone(),
                    artifact_id: attachment.artifact.clone(),
                    offset: 0,
                    limit: 1024,
                },
            )
            .unwrap();
        assert!(
            matches!(chunk, Outcome::ArtifactChunk(value) if STANDARD.decode(&value.data).unwrap() == ATTACHMENT)
        );
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
    let attachment = seed_attachment(&original, None);
    let saved = temp.path().join("backup");
    let source_blob = Store::blob_directory_for(&original.join("state.sqlite3"))
        .join(format!("{}.blob", attachment.artifact));
    std::fs::write(&source_blob, vec![b'x'; ATTACHMENT.len()]).unwrap();
    assert!(backup(&original, &saved).is_err());
    assert!(!saved.exists());
    std::fs::write(&source_blob, ATTACHMENT).unwrap();
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
    let blob = saved
        .join("blobs")
        .join(format!("{}.blob", attachment.artifact));
    std::fs::write(&blob, b"corrupted file content").unwrap();
    assert!(recovery::restore(&saved, &rejected).is_err());
    assert!(!rejected.exists());
    let manifest_path = saved.join("manifest.json");
    let manifest_original = std::fs::read(&manifest_path).unwrap();
    let changed = vec![b'x'; ATTACHMENT.len()];
    std::fs::write(&blob, &changed).unwrap();
    let mut forged: serde_json::Value = serde_json::from_slice(&manifest_original).unwrap();
    forged["blobs"][format!("{}.blob", attachment.artifact)] =
        serde_json::json!(hex::encode(Sha256::digest(&changed)));
    std::fs::write(&manifest_path, serde_json::to_vec(&forged).unwrap()).unwrap();
    assert!(recovery::restore(&saved, &rejected).is_err());
    assert!(!rejected.exists());
    std::fs::write(&manifest_path, manifest_original).unwrap();
    std::fs::remove_file(&blob).unwrap();
    assert!(recovery::restore(&saved, &rejected).is_err());
    assert!(!rejected.exists());
    std::fs::write(&blob, ATTACHMENT).unwrap();
    let manifest = saved.join("manifest.json");
    let mut json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    json["files"]["../outside"] = serde_json::json!("invalid");
    std::fs::write(&manifest, serde_json::to_vec(&json).unwrap()).unwrap();
    assert!(recovery::restore(&saved, &rejected).is_err());
    assert!(!rejected.exists());
    let expiring = temp.path().join("expiring");
    init(&expiring);
    let expiry = chrono::Utc::now().timestamp() + 10;
    let expiring_attachment = seed_attachment(&expiring, Some(expiry));
    let expiring_backup = temp.path().join("expiring-backup");
    backup(&expiring, &expiring_backup).unwrap();
    let before_expiry = recovery::verified_manifest(&expiring_backup).unwrap();
    assert_eq!(before_expiry.blobs.len(), 1);
    assert!(
        chrono::DateTime::parse_from_rfc3339(&before_expiry.created_at)
            .unwrap()
            .timestamp()
            < expiry
    );
    let remaining = (expiry - chrono::Utc::now().timestamp()).max(0) as u64;
    std::thread::sleep(std::time::Duration::from_secs(remaining + 1));
    assert!(chrono::Utc::now().timestamp() >= expiry);
    // Verification uses the recorded capture time, including after live expiry.
    recovery::verified_manifest(&expiring_backup).unwrap();
    let expired_restore = temp.path().join("expired-restored");
    recovery::restore(&expiring_backup, &expired_restore).unwrap();
    let server = CanonicalServerConfig::load(&expired_restore.join("server.toml")).unwrap();
    let operator = CanonicalClientConfig::load(&expired_restore.join("operator.toml")).unwrap();
    let store = Store::open(&server.database, server.limits).unwrap();
    let actor = store.authenticate(&operator.credential).unwrap();
    // Even if bytes remain physically present, the archived capture time is
    // never used to renew live read authorization or artifact expiry.
    std::fs::write(
        store
            .blob_directory()
            .join(format!("{}.blob", expiring_attachment.artifact)),
        ATTACHMENT,
    )
    .unwrap();
    let denied = store
        .execute(
            &actor.credential_id,
            &Operation::ArtifactGet {
                conversation_id: expiring_attachment.conversation,
                artifact_id: expiring_attachment.artifact,
                offset: 0,
                limit: 1024,
            },
        )
        .unwrap_err();
    assert_eq!(denied.code, "forbidden");
    drop(store);
    assert!(!std::fs::read_dir(temp.path()).unwrap().any(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".connect-recovery-")));
}
#[test]
fn canonical_backup_refuses_unrecognized_non_operator_and_revoked_credentials() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("original");
    canonical_init::initialize(&root, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let config = root.join("operator.toml");
    let original = std::fs::read(&config).unwrap();
    let mut c = CanonicalClientConfig::load(&config).unwrap();
    let credential = c.credential.clone();
    c.credential = "0".repeat(64);
    std::fs::write(&config, toml::to_string(&c).unwrap()).unwrap();
    let output = temp.path().join("backup");
    let denied = backup(&root, &output).unwrap_err();
    assert_eq!(
        denied
            .downcast_ref::<connect_protocol::Error>()
            .unwrap()
            .code,
        "unauthorized"
    );
    assert!(!output.exists());
    std::fs::write(&config, original).unwrap();
    let denied = recovery::backup(
        &root.join("server.toml"),
        &root.join("reviews.toml"),
        &output,
    )
    .unwrap_err();
    assert_eq!(
        denied
            .downcast_ref::<connect_protocol::Error>()
            .unwrap()
            .code,
        "forbidden"
    );
    assert!(!output.exists());
    {
        let server = CanonicalServerConfig::load(&root.join("server.toml")).unwrap();
        let store = Store::open(&server.database, server.limits).unwrap();
        let actor = store.authenticate(&credential).unwrap();
        store.revoke(&actor, &actor.credential_id).unwrap();
    }
    let denied = backup(&root, &output).unwrap_err();
    assert_eq!(
        denied
            .downcast_ref::<connect_protocol::Error>()
            .unwrap()
            .code,
        "unauthorized"
    );
    assert!(!output.exists());
}
