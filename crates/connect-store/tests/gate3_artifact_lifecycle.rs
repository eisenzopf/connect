#![allow(clippy::unwrap_used)] // Integration assertions deliberately panic.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use connect_protocol::{Limits, Operation, Outcome};
use connect_store::{Actor, Scope, Store};
use sha2::{Digest, Sha256};

struct Fixture {
    _directory: tempfile::TempDir,
    database: std::path::PathBuf,
    limits: Limits,
    store: Store,
    operator: Actor,
    owner_id: String,
    recipient_id: String,
    owner_key: String,
    recipient_key: String,
    foreign_key: String,
    conversation_id: String,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("state.sqlite3");
        let limits = Limits {
            artifact_file_bytes: 16,
            artifact_bytes: 24,
            artifact_chunk_bytes: 8,
            ..Limits::default()
        };
        let store = Store::initialize(&database, limits.clone()).unwrap();
        let owner = store.create_identity("owner").unwrap();
        let recipient = store.create_identity("recipient").unwrap();
        let foreign = store.create_identity("foreign").unwrap();
        let operator_identity = store.create_identity("operator").unwrap();
        let expiry = chrono::Utc::now().timestamp() + 3_600;
        let operator_key = store
            .issue(&operator_identity.id, Scope::operator(), expiry)
            .unwrap();
        let operator = store.authenticate(&operator_key.token).unwrap();
        let owner_key = store
            .issue(
                &owner.id,
                Scope::communications(vec![recipient.id.clone()]),
                expiry,
            )
            .unwrap();
        let recipient_key = store
            .issue(
                &recipient.id,
                Scope::communications(vec![owner.id.clone()]),
                expiry,
            )
            .unwrap();
        let foreign_key = store
            .issue(&foreign.id, Scope::communications(vec![]), expiry)
            .unwrap();
        let conversation_id = match store
            .execute(
                &owner_key.id,
                &Operation::Create {
                    request_id: "artifact-create".into(),
                    participants: vec![recipient.id.clone()],
                },
            )
            .unwrap()
        {
            Outcome::Conversation(value) => value.id,
            other => panic!("expected conversation, got {other:?}"),
        };
        Self {
            _directory: directory,
            database,
            limits,
            store,
            operator,
            owner_id: owner.id,
            recipient_id: recipient.id,
            owner_key: owner_key.id,
            recipient_key: recipient_key.id,
            foreign_key: foreign_key.id,
            conversation_id,
        }
    }

    fn begin(&self, request_id: &str, bytes: &[u8], digest: String) -> connect_protocol::Artifact {
        match self
            .store
            .execute(
                &self.owner_key,
                &Operation::ArtifactBegin {
                    request_id: request_id.into(),
                    conversation_id: self.conversation_id.clone(),
                    name: format!("{request_id}.bin"),
                    media_type: "application/octet-stream".into(),
                    size: bytes.len() as u64,
                    sha256: digest,
                    expires_at: None,
                },
            )
            .unwrap()
        {
            Outcome::Artifact(value) => value,
            other => panic!("expected artifact, got {other:?}"),
        }
    }
}

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[test]
fn upload_resume_conflicts_digest_authority_quota_and_delete_are_bounded() {
    let mut fixture = Fixture::new();
    let bytes = b"0123456789abcdef";
    let artifact = fixture.begin("exact", bytes, digest(bytes));

    let first = fixture
        .store
        .execute(
            &fixture.owner_key,
            &Operation::ArtifactAppend {
                request_id: "append-first".into(),
                conversation_id: fixture.conversation_id.clone(),
                artifact_id: artifact.id.clone(),
                offset: 0,
                data: STANDARD.encode(&bytes[..8]),
            },
        )
        .unwrap();
    assert!(matches!(first, Outcome::Artifact(value) if value.received == 8));
    assert_eq!(
        fixture
            .store
            .execute(
                &fixture.owner_key,
                &Operation::ArtifactAppend {
                    request_id: "wrong-offset".into(),
                    conversation_id: fixture.conversation_id.clone(),
                    artifact_id: artifact.id.clone(),
                    offset: 0,
                    data: STANDARD.encode(&bytes[8..]),
                },
            )
            .unwrap_err()
            .code,
        "conflict"
    );

    drop(fixture.store);
    fixture.store = Store::open(&fixture.database, fixture.limits.clone()).unwrap();
    fixture
        .store
        .execute(
            &fixture.owner_key,
            &Operation::ArtifactAppend {
                request_id: "append-resumed".into(),
                conversation_id: fixture.conversation_id.clone(),
                artifact_id: artifact.id.clone(),
                offset: 8,
                data: STANDARD.encode(&bytes[8..]),
            },
        )
        .unwrap();
    fixture
        .store
        .execute(
            &fixture.owner_key,
            &Operation::ArtifactFinish {
                request_id: "finish-exact".into(),
                conversation_id: fixture.conversation_id.clone(),
                artifact_id: artifact.id.clone(),
            },
        )
        .unwrap();
    let duplicate = fixture
        .store
        .execute(
            &fixture.owner_key,
            &Operation::ArtifactFinish {
                request_id: "finish-exact-again".into(),
                conversation_id: fixture.conversation_id.clone(),
                artifact_id: artifact.id.clone(),
            },
        )
        .unwrap();
    assert!(
        matches!(duplicate, Outcome::Artifact(value) if value.id == artifact.id && value.state == "ready")
    );

    let recipient = fixture
        .store
        .execute(
            &fixture.recipient_key,
            &Operation::ArtifactGet {
                conversation_id: fixture.conversation_id.clone(),
                artifact_id: artifact.id.clone(),
                offset: 0,
                limit: 8,
            },
        )
        .unwrap();
    assert!(
        matches!(recipient, Outcome::ArtifactChunk(value) if STANDARD.decode(value.data.as_bytes()).unwrap() == bytes[..8])
    );
    std::thread::scope(|scope| {
        for offset in [0, 4, 8, 12] {
            let store = &fixture.store;
            let recipient_key = &fixture.recipient_key;
            let conversation_id = fixture.conversation_id.clone();
            let artifact_id = artifact.id.clone();
            scope.spawn(move || {
                let outcome = store
                    .execute(
                        recipient_key,
                        &Operation::ArtifactGet {
                            conversation_id,
                            artifact_id,
                            offset,
                            limit: 4,
                        },
                    )
                    .unwrap();
                assert!(matches!(outcome, Outcome::ArtifactChunk(value) if STANDARD.decode(&value.data).unwrap() == bytes[offset as usize..offset as usize + 4]));
            });
        }
    });
    assert_eq!(
        fixture
            .store
            .execute(
                &fixture.foreign_key,
                &Operation::ArtifactGet {
                    conversation_id: fixture.conversation_id.clone(),
                    artifact_id: artifact.id.clone(),
                    offset: 0,
                    limit: 8,
                },
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        fixture
            .store
            .execute(
                &fixture.owner_key,
                &Operation::ArtifactBegin {
                    request_id: "over-limit".into(),
                    conversation_id: fixture.conversation_id.clone(),
                    name: "too-large.bin".into(),
                    media_type: "application/octet-stream".into(),
                    size: 17,
                    sha256: digest(&[0; 17]),
                    expires_at: None,
                },
            )
            .unwrap_err()
            .code,
        "invalid-request"
    );

    let mismatched = fixture.begin("mismatch", b"bad", digest(b"good"));
    fixture
        .store
        .execute(
            &fixture.owner_key,
            &Operation::ArtifactAppend {
                request_id: "append-mismatch".into(),
                conversation_id: fixture.conversation_id.clone(),
                artifact_id: mismatched.id.clone(),
                offset: 0,
                data: STANDARD.encode(b"bad"),
            },
        )
        .unwrap();
    assert_eq!(
        fixture
            .store
            .execute(
                &fixture.owner_key,
                &Operation::ArtifactFinish {
                    request_id: "finish-mismatch".into(),
                    conversation_id: fixture.conversation_id.clone(),
                    artifact_id: mismatched.id,
                },
            )
            .unwrap_err()
            .code,
        "digest-mismatch"
    );

    let unwritable = fixture.begin("unwritable", b"disk", digest(b"disk"));
    fixture
        .store
        .execute(
            &fixture.owner_key,
            &Operation::ArtifactAppend {
                request_id: "append-unwritable".into(),
                conversation_id: fixture.conversation_id.clone(),
                artifact_id: unwritable.id.clone(),
                offset: 0,
                data: STANDARD.encode(b"disk"),
            },
        )
        .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let directory = fixture.store.blob_directory();
        let original = std::fs::metadata(directory).unwrap().permissions();
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o500)).unwrap();
        let result = fixture.store.execute(
            &fixture.owner_key,
            &Operation::ArtifactFinish {
                request_id: "finish-unwritable".into(),
                conversation_id: fixture.conversation_id.clone(),
                artifact_id: unwritable.id.clone(),
            },
        );
        std::fs::set_permissions(directory, original).unwrap();
        assert_eq!(result.unwrap_err().code, "storage-unavailable");
        assert!(!directory.join(format!("{}.blob", unwritable.id)).exists());
    }

    let blob = fixture
        .store
        .blob_directory()
        .join(format!("{}.blob", artifact.id));
    fixture
        .store
        .execute(
            &fixture.owner_key,
            &Operation::ArtifactDelete {
                request_id: "delete-exact".into(),
                conversation_id: fixture.conversation_id.clone(),
                artifact_id: artifact.id.clone(),
            },
        )
        .unwrap();
    assert_eq!(
        fixture
            .store
            .execute(
                &fixture.recipient_key,
                &Operation::ArtifactGet {
                    conversation_id: fixture.conversation_id.clone(),
                    artifact_id: artifact.id,
                    offset: 0,
                    limit: 8,
                },
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    drop(fixture.store);
    fixture.store = Store::open(&fixture.database, fixture.limits).unwrap();
    assert!(!blob.exists(), "startup cleanup must erase deleted bytes");

    let expiring_bytes = b"soon";
    let expiring = match fixture
        .store
        .execute(
            &fixture.owner_key,
            &Operation::ArtifactBegin {
                request_id: "expiring".into(),
                conversation_id: fixture.conversation_id.clone(),
                name: "expiring.bin".into(),
                media_type: "application/octet-stream".into(),
                size: expiring_bytes.len() as u64,
                sha256: digest(expiring_bytes),
                expires_at: Some(chrono::Utc::now().timestamp() + 60),
            },
        )
        .unwrap()
    {
        Outcome::Artifact(value) => value,
        other => panic!("expected artifact, got {other:?}"),
    };
    fixture
        .store
        .execute(
            &fixture.owner_key,
            &Operation::ArtifactAppend {
                request_id: "append-expiring".into(),
                conversation_id: fixture.conversation_id.clone(),
                artifact_id: expiring.id.clone(),
                offset: 0,
                data: STANDARD.encode(expiring_bytes),
            },
        )
        .unwrap();
    fixture
        .store
        .execute(
            &fixture.owner_key,
            &Operation::ArtifactFinish {
                request_id: "finish-expiring".into(),
                conversation_id: fixture.conversation_id.clone(),
                artifact_id: expiring.id.clone(),
            },
        )
        .unwrap();
    let expiring_blob = fixture
        .store
        .blob_directory()
        .join(format!("{}.blob", expiring.id));
    assert!(expiring_blob.is_file());
    // Advance this isolated fixture's retained expiry after the upload commits;
    // a one-second wall-clock deadline can race compilation and CPU contention.
    rusqlite::Connection::open(&fixture.database)
        .unwrap()
        .execute(
            "UPDATE artifacts SET expires_at=?1 WHERE id=?2",
            rusqlite::params![chrono::Utc::now().timestamp() - 1, expiring.id],
        )
        .unwrap();
    assert_eq!(
        fixture
            .store
            .execute(
                &fixture.recipient_key,
                &Operation::ArtifactGet {
                    conversation_id: fixture.conversation_id.clone(),
                    artifact_id: expiring.id,
                    offset: 0,
                    limit: 8,
                },
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    drop(fixture.store);
    fixture.store = Store::open(
        &fixture.database,
        Limits {
            artifact_file_bytes: 16,
            artifact_bytes: 24,
            artifact_chunk_bytes: 8,
            ..Limits::default()
        },
    )
    .unwrap();
    assert!(
        !expiring_blob.exists(),
        "startup cleanup must erase expired bytes"
    );
}

#[test]
fn unsafe_names_and_revoked_upload_authority_are_rejected() {
    let fixture = Fixture::new();
    for (index, name) in ["../escape", "nested/file", r"nested\file", "nul\0name"]
        .into_iter()
        .enumerate()
    {
        let error = fixture
            .store
            .execute(
                &fixture.owner_key,
                &Operation::ArtifactBegin {
                    request_id: format!("unsafe-name-{index}"),
                    conversation_id: fixture.conversation_id.clone(),
                    name: name.into(),
                    media_type: "application/octet-stream".into(),
                    size: 1,
                    sha256: digest(b"x"),
                    expires_at: None,
                },
            )
            .unwrap_err();
        assert_eq!(error.code, "invalid-request");
    }

    let delegated = fixture
        .store
        .issue(
            &fixture.owner_id,
            Scope::communications(vec![fixture.recipient_id.clone()]),
            chrono::Utc::now().timestamp() + 3_600,
        )
        .unwrap();
    let artifact = match fixture
        .store
        .execute(
            &delegated.id,
            &Operation::ArtifactBegin {
                request_id: "revoked-begin".into(),
                conversation_id: fixture.conversation_id.clone(),
                name: "revoked.bin".into(),
                media_type: "application/octet-stream".into(),
                size: 1,
                sha256: digest(b"x"),
                expires_at: None,
            },
        )
        .unwrap()
    {
        Outcome::Artifact(value) => value,
        other => panic!("expected artifact, got {other:?}"),
    };
    fixture
        .store
        .revoke(&fixture.operator, &delegated.id)
        .unwrap();
    let error = fixture
        .store
        .execute(
            &delegated.id,
            &Operation::ArtifactAppend {
                request_id: "revoked-append".into(),
                conversation_id: fixture.conversation_id.clone(),
                artifact_id: artifact.id,
                offset: 0,
                data: STANDARD.encode(b"x"),
            },
        )
        .unwrap_err();
    assert_eq!(error.code, "unauthorized");
}
