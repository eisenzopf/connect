#![allow(clippy::unwrap_used)] // Integration assertions deliberately panic.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use connect_client::ClientConfig;
use connect_protocol::{Operation, Outcome};
use connect_server::{
    cli::{init, recovery},
    config::ServerConfig,
};
use connect_store::Store;
use sha2::{Digest, Sha256};

#[test]
fn finalized_artifact_blob_is_private_verified_and_restored_with_database() {
    let temporary = tempfile::tempdir().unwrap();
    let installation = temporary.path().join("installation");
    init::initialize(&installation, false, "127.0.0.1:4743".parse().unwrap()).unwrap();
    let server = ServerConfig::load(&installation.join("server.toml")).unwrap();
    let operator = ClientConfig::load(&installation.join("operator.toml")).unwrap();
    let store = Store::open(&server.database, server.limits.clone()).unwrap();
    let conversation = match store
        .execute(
            &operator.credential_id,
            &Operation::Create {
                request_id: "blob-create".into(),
                request: connect_protocol::conversations::CreateRequest::persistent(vec![]),
            },
        )
        .unwrap()
    {
        Outcome::Conversation(value) => value,
        other => panic!("expected conversation, got {other:?}"),
    };
    let bytes = b"artifact bytes live outside SQLite after finalization";
    let artifact = match store
        .execute(
            &operator.credential_id,
            &Operation::ArtifactBegin {
                to: connect_protocol::messages::MessageAudience::all(),
                request_id: "blob-begin".into(),
                conversation_id: conversation.id.clone(),
                name: "proof.txt".into(),
                media_type: "text/plain".into(),
                size: bytes.len() as u64,
                sha256: hex::encode(Sha256::digest(bytes)),
                expires_at: None,
            },
        )
        .unwrap()
    {
        Outcome::Artifact(value) => value,
        other => panic!("expected artifact, got {other:?}"),
    };
    store
        .execute(
            &operator.credential_id,
            &Operation::ArtifactAppend {
                request_id: "blob-append".into(),
                conversation_id: conversation.id.clone(),
                artifact_id: artifact.id.clone(),
                offset: 0,
                data: STANDARD.encode(bytes),
            },
        )
        .unwrap();
    store
        .execute(
            &operator.credential_id,
            &Operation::ArtifactFinish {
                request_id: "blob-finish".into(),
                conversation_id: conversation.id.clone(),
                artifact_id: artifact.id.clone(),
            },
        )
        .unwrap();
    let blob_name = format!("{}.blob", artifact.id);
    assert_eq!(
        std::fs::read(store.blob_directory().join(&blob_name)).unwrap(),
        bytes
    );
    let connection = rusqlite::Connection::open(&server.database).unwrap();
    assert_eq!(
        connection
            .query_row::<i64, _, _>(
                "SELECT length(content) FROM artifacts WHERE id=?1",
                [&artifact.id],
                |row| row.get(0),
            )
            .unwrap(),
        0
    );
    drop(connection);
    drop(store);

    let backup = temporary.path().join("backup");
    recovery::backup(
        &installation.join("server.toml"),
        &installation.join("operator.toml"),
        &backup,
    )
    .unwrap();
    let manifest = recovery::verified_manifest(&backup).unwrap();
    assert_eq!(manifest.version, 2);
    assert_eq!(manifest.blobs.len(), 1);

    let restored = temporary.path().join("restored");
    recovery::restore(&backup, &restored, "127.0.0.1:4744".parse().unwrap(), None).unwrap();
    let restored_server = ServerConfig::load(&restored.join("server.toml")).unwrap();
    let restored_operator = ClientConfig::load(&restored.join("operator.toml")).unwrap();
    let restored_store =
        Store::open(&restored_server.database, restored_server.limits.clone()).unwrap();
    let chunk = restored_store
        .execute(
            &restored_operator.credential_id,
            &Operation::ArtifactGet {
                conversation_id: conversation.id.clone(),
                artifact_id: artifact.id.clone(),
                offset: 0,
                limit: 1024,
            },
        )
        .unwrap();
    assert!(
        matches!(chunk, Outcome::ArtifactChunk(value) if STANDARD.decode(&value.data).unwrap() == bytes)
    );

    let orphan_id = uuid::Uuid::new_v4();
    let temporary_id = uuid::Uuid::new_v4();
    let orphan = restored_store
        .blob_directory()
        .join(format!("{orphan_id}.blob"));
    let stale_temporary = restored_store
        .blob_directory()
        .join(format!(".{orphan_id}.{temporary_id}.tmp"));
    std::fs::write(&orphan, b"orphan").unwrap();
    std::fs::write(&stale_temporary, b"partial").unwrap();
    assert_eq!(restored_store.purge_orphan_artifact_blobs(1).unwrap(), 1);
    assert_eq!(restored_store.purge_orphan_artifact_blobs(10).unwrap(), 1);
    assert!(restored_store.blob_directory().join(&blob_name).is_file());

    restored_store
        .execute(
            &restored_operator.credential_id,
            &Operation::ArtifactDelete {
                request_id: "blob-delete".into(),
                conversation_id: conversation.id,
                artifact_id: artifact.id,
            },
        )
        .unwrap();
    assert_eq!(restored_store.purge_orphan_artifact_blobs(10).unwrap(), 1);
    assert!(!restored_store.blob_directory().join(&blob_name).exists());

    std::fs::write(backup.join("blobs").join(blob_name), b"tampered").unwrap();
    assert!(recovery::verified_manifest(&backup).is_err());
}
