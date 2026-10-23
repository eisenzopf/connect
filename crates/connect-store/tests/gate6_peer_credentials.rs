//! Durable key ownership, administrative replay, immutable facts and snapshots.
#![allow(clippy::unwrap_used)]
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use connect_protocol::{provisioning::*, Limits};
use connect_store::{Scope, Store};
use ring::signature::{Ed25519KeyPair, KeyPair};
use rusqlite::Connection;

struct Fixture {
    dir: tempfile::TempDir,
    store: Store,
    admin: String,
    identity: String,
    credential: String,
    public: String,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store =
            Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap();
        let identity = store.create_identity("owner").unwrap().id;
        let now = chrono::Utc::now().timestamp();
        let admin = store
            .issue(&identity, Scope::operator(), now + 600)
            .unwrap()
            .id;
        let credential = store
            .issue(&identity, Scope::messaging(vec!["*".into()]), now + 600)
            .unwrap()
            .id;
        store
            .execute_provisioning(
                &admin,
                ProvisioningCommand::Endpoint {
                    operation_id: "device".into(),
                    identity_id: identity.clone(),
                    device: DeviceSpec {
                        id: "device".into(),
                        kind: "desktop".into(),
                        platform: "fixture".into(),
                    },
                    transport: EndpointTransport::Quic,
                    address: "quic://localhost:4444".into(),
                    capabilities: vec!["data".into()],
                    secret_ref: None,
                },
            )
            .unwrap();
        let bytes = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
        let public = URL_SAFE_NO_PAD.encode(
            Ed25519KeyPair::from_pkcs8(bytes.as_ref())
                .unwrap()
                .public_key()
                .as_ref(),
        );
        Self {
            dir,
            store,
            admin,
            identity,
            credential,
            public,
        }
    }
    fn command(&self, operation: &str, key: &str) -> ProvisioningCommand {
        let now = chrono::Utc::now().timestamp();
        ProvisioningCommand::SigningKey {
            operation_id: operation.into(),
            identity_id: self.identity.clone(),
            credential_id: self.credential.clone(),
            device_id: "device".into(),
            key_id: key.into(),
            algorithm: SigningAlgorithm::EdDSA,
            public_key: self.public.clone(),
            not_before: now - 1,
            expires_at: now + 300,
            expected_revision: None,
        }
    }
}
#[test]
fn key_ownership_validity_revisions_and_retirement_are_not_overridden_by_retry() {
    let f = Fixture::new();
    let command = f.command("enroll", "key");
    let original = f
        .store
        .execute_provisioning(&f.admin, command.clone())
        .unwrap();
    assert_eq!(
        f.store
            .execute_provisioning(&f.admin, command.clone())
            .unwrap(),
        original
    );
    let mut change = command.clone();
    if let ProvisioningCommand::SigningKey { public_key, .. } = &mut change {
        *public_key = URL_SAFE_NO_PAD.encode([1u8; 32]);
    }
    assert_eq!(
        f.store
            .execute_provisioning(&f.admin, change)
            .unwrap_err()
            .code,
        "conflict"
    );
    let foreign = f.store.create_identity("foreign").unwrap().id;
    let foreign_key = f
        .store
        .issue(
            &foreign,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let mut cross = f.command("cross", "cross-key");
    if let ProvisioningCommand::SigningKey { credential_id, .. } = &mut cross {
        *credential_id = foreign_key.id.clone();
    }
    assert_eq!(
        f.store
            .execute_provisioning(&f.admin, cross)
            .unwrap_err()
            .code,
        "forbidden"
    );
    let mut cross = f.command("cross-device", "cross-device-key");
    if let ProvisioningCommand::SigningKey {
        identity_id,
        credential_id,
        ..
    } = &mut cross
    {
        *identity_id = foreign;
        *credential_id = foreign_key.id;
    }
    assert_eq!(
        f.store
            .execute_provisioning(&f.admin, cross)
            .unwrap_err()
            .code,
        "forbidden"
    );
    let mut weak = f.command("weak", "weak-key");
    if let ProvisioningCommand::SigningKey { public_key, .. } = &mut weak {
        *public_key = URL_SAFE_NO_PAD.encode([0u8; 32]);
    }
    assert_eq!(
        f.store
            .execute_provisioning(&f.admin, weak)
            .unwrap_err()
            .code,
        "invalid-request"
    );
    let mut curve = f.command("curve", "bad-curve");
    if let ProvisioningCommand::SigningKey {
        public_key,
        algorithm,
        ..
    } = &mut curve
    {
        let mut b = [0u8; 65];
        b[0] = 4;
        *public_key = URL_SAFE_NO_PAD.encode(b);
        *algorithm = SigningAlgorithm::ES256;
    }
    assert_eq!(
        f.store
            .execute_provisioning(&f.admin, curve)
            .unwrap_err()
            .code,
        "invalid-request"
    );
    assert_eq!(
        f.store
            .execute_provisioning(
                &f.admin,
                ProvisioningCommand::RevokeSigningKey {
                    operation_id: "stale".into(),
                    key_id: "key".into(),
                    expected_revision: 1
                }
            )
            .unwrap_err()
            .code,
        "conflict"
    );
    f.store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::RevokeSigningKey {
                operation_id: "revoke".into(),
                key_id: "key".into(),
                expected_revision: 0,
            },
        )
        .unwrap();
    assert!(matches!(
        f.store
            .execute_provisioning(&f.admin, command.clone())
            .unwrap(),
        ProvisioningOutcome::SigningKey {
            revoked: true,
            revision: 1,
            ..
        }
    ));
    let mut revive = command;
    if let ProvisioningCommand::SigningKey {
        operation_id,
        expected_revision,
        ..
    } = &mut revive
    {
        *operation_id = "revive".into();
        *expected_revision = Some(1);
    }
    assert_eq!(
        f.store
            .execute_provisioning(&f.admin, revive)
            .unwrap_err()
            .code,
        "conflict"
    );
    let c = Connection::open(f.store.path()).unwrap();
    c.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    assert!(c
        .execute(
            "UPDATE peer_signing_keys SET revoked=0 WHERE key_id='key'",
            []
        )
        .is_err());
    assert!(c
        .execute(
            "UPDATE credentials SET issued_at=issued_at+1 WHERE id=?1",
            [&f.credential]
        )
        .is_err());
    assert!(c
        .execute(
            "INSERT INTO credential_signing_keys VALUES(?1,'key')",
            [&f.admin]
        )
        .is_err());
    // Neither a fresh binding nor a lookup can revive a retired key.
    let tenant = f.store.credential_metadata(&f.admin).unwrap().tenant_id;
    assert!(f
        .store
        .resolve_peer_signing_key(&f.admin, &tenant, "key", "EdDSA")
        .is_err());
}

#[test]
fn fresh_schema_snapshot_digest_includes_keys_bindings_and_real_issuance() {
    let f = Fixture::new();
    let before = f
        .store
        .credential_metadata(&f.credential)
        .unwrap()
        .issued_at;
    f.store
        .execute_provisioning(&f.admin, f.command("enroll", "key"))
        .unwrap();
    let path = f.dir.path().join("backup.sqlite3");
    let snapshot = f.store.backup(&path).unwrap();
    assert_eq!(snapshot.schema, connect_store::SCHEMA_VERSION);
    let copy = f.dir.path().join("restore.sqlite3");
    std::fs::copy(&path, &copy).unwrap();
    let restored = Store::open(&copy, Limits::default()).unwrap();
    assert_eq!(
        restored
            .credential_metadata(&f.credential)
            .unwrap()
            .issued_at,
        before
    );
    let tenant = restored
        .credential_metadata(&f.credential)
        .unwrap()
        .tenant_id;
    assert!(restored
        .resolve_peer_signing_key(&f.credential, &tenant, "key", "EdDSA")
        .is_ok());
    assert_eq!(Store::inspect_snapshot(&path).unwrap(), snapshot);
    let c = Connection::open(&path).unwrap();
    c.execute(
        "UPDATE peer_signing_keys SET revoked=1,revision=1 WHERE key_id='key'",
        [],
    )
    .unwrap();
    drop(c);
    assert_ne!(
        Store::inspect_snapshot(&path).unwrap().digest,
        snapshot.digest
    );
}

#[test]
fn key_and_credential_history_bounds_do_not_evict_old_bindings_or_grant_the_overflow() {
    let f = Fixture::new();
    let base = f.command("base", "base");
    f.store
        .execute_provisioning(&f.admin, base.clone())
        .unwrap();
    for n in 1..128 {
        let cred = f
            .store
            .issue(
                &f.identity,
                Scope::messaging(vec![]),
                chrono::Utc::now().timestamp() + 600,
            )
            .unwrap();
        let mut command = base.clone();
        if let ProvisioningCommand::SigningKey {
            operation_id,
            credential_id,
            expected_revision,
            ..
        } = &mut command
        {
            *operation_id = format!("bind-{n}");
            *credential_id = cred.id;
            *expected_revision = Some(n - 1);
        }
        f.store.execute_provisioning(&f.admin, command).unwrap();
    }
    let overflow = f
        .store
        .issue(
            &f.identity,
            Scope::messaging(vec![]),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let mut command = base;
    if let ProvisioningCommand::SigningKey {
        operation_id,
        credential_id,
        expected_revision,
        ..
    } = &mut command
    {
        *operation_id = "overflow".into();
        *credential_id = overflow.id.clone();
        *expected_revision = Some(127);
    }
    assert_eq!(
        f.store
            .execute_provisioning(&f.admin, command)
            .unwrap_err()
            .code,
        "capacity-exceeded"
    );
    let tenant = f
        .store
        .credential_metadata(&f.credential)
        .unwrap()
        .tenant_id;
    assert!(f
        .store
        .resolve_peer_signing_key(&f.credential, &tenant, "base", "EdDSA")
        .is_ok());
    assert!(f
        .store
        .resolve_peer_signing_key(&overflow.id, &tenant, "base", "EdDSA")
        .is_err());
    for n in 1..64 {
        f.store
            .execute_provisioning(
                &f.admin,
                f.command(&format!("key-{n}"), &format!("key-{n}")),
            )
            .unwrap();
    }
    assert_eq!(
        f.store
            .execute_provisioning(&f.admin, f.command("key-overflow", "key-overflow"))
            .unwrap_err()
            .code,
        "capacity-exceeded"
    );
}
