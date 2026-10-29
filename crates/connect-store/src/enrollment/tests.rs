#![allow(clippy::unwrap_used)]
use super::*;
use crate::{Scope, SCHEMA_VERSION};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use p256::ecdsa::signature::{RandomizedSigner, Signer};

#[derive(Clone)]
enum Key {
    Ed(ed25519_dalek::SigningKey),
    P256(p256::ecdsa::SigningKey),
}
impl Key {
    fn new(algorithm: SigningAlgorithm) -> Self {
        match algorithm {
            SigningAlgorithm::EdDSA => Self::Ed(ed25519_dalek::SigningKey::from_bytes(&[7; 32])),
            SigningAlgorithm::ES256 => {
                Self::P256(p256::ecdsa::SigningKey::from_slice(&[8; 32]).unwrap())
            }
        }
    }
    fn public(&self) -> String {
        URL_SAFE_NO_PAD.encode(match self {
            Self::Ed(k) => k.verifying_key().to_bytes().to_vec(),
            Self::P256(k) => k
                .verifying_key()
                .to_encoded_point(false)
                .as_bytes()
                .to_vec(),
        })
    }
    fn sign(&self, message: &[u8]) -> String {
        URL_SAFE_NO_PAD.encode(match self {
            Self::Ed(k) => k.sign(message).to_bytes().to_vec(),
            Self::P256(k) => {
                let s: p256::ecdsa::Signature = k.sign_with_rng(&mut rand::rngs::OsRng, message);
                s.to_bytes().to_vec()
            }
        })
    }
}
struct Fixture {
    dir: tempfile::TempDir,
    store: Arc<Store>,
    admin: Actor,
    key: Key,
    request: EnrollmentRequest,
}
impl Fixture {
    fn new(algorithm: SigningAlgorithm, transport: EnrollmentTransport) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(
            Store::initialize(&dir.path().join("state.sqlite3"), Default::default()).unwrap(),
        );
        let owner = store.create_identity("backend").unwrap().id;
        let target = store.create_identity("client").unwrap().id;
        let peer = store.create_identity("peer").unwrap().id;
        let admin = store
            .authenticate(
                &store
                    .issue(&owner, Scope::operator(), crate::now() + 7200)
                    .unwrap()
                    .token,
            )
            .unwrap();
        let key = Key::new(algorithm);
        let browser = transport != EnrollmentTransport::Quic;
        let request = EnrollmentRequest {
            operation_id: "approve-original".into(),
            completion_operation_id: "claim-original".into(),
            identity_id: target,
            device: DeviceSpec {
                id: "device-original".into(),
                kind: if browser { "browser" } else { "desktop" }.into(),
                platform: "synthetic".into(),
            },
            key_id: "key-original".into(),
            algorithm,
            public_key: key.public(),
            transport,
            endpoint_address: if browser {
                "https://device.fixture.invalid/client"
            } else {
                "quic://device.fixture.invalid:4443"
            }
            .into(),
            capabilities: vec!["control".into(), "data".into()],
            browser_origin: browser.then(|| "https://app.fixture.invalid".into()),
            operations: vec!["send".into(), "operation.get".into()],
            recipients: vec![peer],
            credential_expires_at: crate::now() + 600,
            challenge_expires_at: crate::now() + 300,
        };
        Self {
            dir,
            store,
            admin,
            key,
            request,
        }
    }
    fn offer(&self) -> EnrollmentOffer {
        self.store
            .approve_enrollment(&self.admin.credential_id, self.request.clone())
            .unwrap()
            .outcome()
            .unwrap()
            .clone()
    }
    fn claim(&self, offer: &EnrollmentOffer) -> EnrollmentClaim {
        EnrollmentClaim {
            operation_id: offer.request.completion_operation_id.clone(),
            enrollment_id: offer.enrollment_id.clone(),
            identity_id: offer.request.identity_id.clone(),
            device_id: offer.request.device.id.clone(),
            endpoint_id: offer.endpoint_id.clone(),
            key_id: offer.request.key_id.clone(),
            transport: offer.request.transport,
            browser_origin: offer.request.browser_origin.clone(),
            challenge: offer.challenge.clone(),
            signature: self.key.sign(&offer.proof_message().unwrap()),
        }
    }
    fn complete(&self, offer: &EnrollmentOffer) -> EnrollmentCredential {
        self.store
            .claim_enrollment(self.claim(offer), offer.request.browser_origin.as_deref())
            .unwrap()
            .outcome()
            .unwrap()
            .clone()
    }
    fn counts(&self) -> (u64, u64, u64, u64, u64, u64) {
        self.store.connection().unwrap().query_row("SELECT (SELECT COUNT(*) FROM credentials),(SELECT COUNT(*) FROM devices),(SELECT COUNT(*) FROM endpoints),(SELECT COUNT(*) FROM peer_signing_keys),(SELECT COUNT(*) FROM credential_signing_keys),(SELECT COUNT(*) FROM operations)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).unwrap()
    }
}

#[test]
fn both_algorithms_and_all_transports_issue_one_bound_credential_and_recover_original() {
    for algorithm in [SigningAlgorithm::EdDSA, SigningAlgorithm::ES256] {
        for transport in [
            EnrollmentTransport::Quic,
            EnrollmentTransport::Webtransport,
            EnrollmentTransport::WssWebrtc,
        ] {
            let f = Fixture::new(algorithm, transport);
            let before = f.counts();
            let offer = f.offer();
            let approved = f.counts();
            assert_eq!(
                (approved.0, approved.1, approved.2, approved.3, approved.4),
                (before.0, before.1, before.2, before.3, before.4)
            );
            assert_eq!(approved.5, before.5 + 2);
            assert_eq!(f.offer(), offer);
            let value = f.complete(&offer);
            let count = f.counts();
            assert_eq!(
                count,
                (
                    before.0 + 1,
                    before.1 + 1,
                    before.2 + 1,
                    before.3 + 1,
                    before.4 + 1,
                    before.5 + 2
                )
            );
            let actor = f.store.authenticate(&value.credential.credential).unwrap();
            assert_eq!(actor.identity_id, f.request.identity_id);
            assert!(actor.permits("send"));
            assert!(!actor.permits("admin"));
            assert!(!actor.permits("credential.delegate"));
            assert_eq!(actor.expires_at, f.request.credential_expires_at);
            let key = f
                .store
                .resolve_peer_signing_key(
                    &actor.credential_id,
                    "tnt_default",
                    &f.request.key_id,
                    algorithm.as_str(),
                )
                .unwrap();
            assert_eq!(key.device_id, f.request.device.id);
            let binding = f
                .store
                .enrollment_credential_binding(&actor.credential_id)
                .unwrap()
                .unwrap();
            assert_eq!(binding.transport, transport);
            assert_eq!(binding.browser_origin, f.request.browser_origin);
            let fresh = Store::open_administration(f.store.path(), Default::default()).unwrap();
            let resigned = f.claim(&offer);
            let recovered = fresh
                .claim_enrollment(resigned, offer.request.browser_origin.as_deref())
                .unwrap();
            assert_eq!(recovered.outcome().unwrap(), &value);
            drop(recovered);
            assert_eq!(f.counts(), count);
            let (intent,receipt,request,hash):(String,String,String,String)=f.store.connection().unwrap().query_row("SELECT o.intent_json,o.result_json,e.request_json,e.challenge_hash FROM client_enrollments e JOIN operations o ON o.operation_id=e.complete_operation_id AND o.owner_id=e.issuer_identity_id WHERE e.id=?1",[&offer.enrollment_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
            for text in [intent, receipt, request, hash] {
                assert!(!text.contains(&offer.challenge));
                assert!(!text.contains(&value.credential.credential));
            }
            assert_eq!(format!("{value:?}"), "EnrollmentCredential(redacted)");
        }
    }
}

#[test]
fn stolen_challenge_without_device_key_exhausts_bounded_attempts_without_grant() {
    for algorithm in [SigningAlgorithm::EdDSA, SigningAlgorithm::ES256] {
        let f = Fixture::new(algorithm, EnrollmentTransport::Quic);
        let offer = f.offer();
        let before = f.counts();
        let mut stolen = f.claim(&offer);
        stolen.signature = URL_SAFE_NO_PAD.encode([0; 64]);
        for attempt in 1..=5 {
            assert!(f.store.claim_enrollment(stolen.clone(), None).is_err());
            let (state, attempts): (String, u32) = f
                .store
                .connection()
                .unwrap()
                .query_row(
                    "SELECT state,attempts FROM client_enrollments WHERE id=?1",
                    [&offer.enrollment_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!(attempts, attempt);
            assert_eq!(state, if attempt == 5 { "exhausted" } else { "pending" });
            assert_eq!(f.counts(), before);
        }
        assert!(f.store.claim_enrollment(f.claim(&offer), None).is_err());
        assert_eq!(f.counts(), before);
    }
}

#[test]
fn changed_targets_origin_or_logical_ids_cannot_claim_or_mutate_attempts() {
    let f = Fixture::new(SigningAlgorithm::EdDSA, EnrollmentTransport::WssWebrtc);
    let offer = f.offer();
    let good = f.claim(&offer);
    let before = f.counts();
    for field in 0..8 {
        let mut bad = good.clone();
        match field {
            0 => bad.operation_id = "replacement".into(),
            1 => bad.endpoint_id = "foreign".into(),
            2 => bad.device_id = "foreign".into(),
            3 => bad.identity_id = "foreign".into(),
            4 => bad.key_id = "foreign".into(),
            5 => bad.transport = EnrollmentTransport::Webtransport,
            6 => bad.challenge = "b".repeat(64),
            _ => bad.browser_origin = Some("https://other.fixture.invalid".into()),
        };
        assert!(f
            .store
            .claim_enrollment(bad, offer.request.browser_origin.as_deref())
            .is_err());
        assert_eq!(f.counts(), before);
    }
    for origin in [None, Some("null"), Some("https://other.fixture.invalid")] {
        assert!(f.store.claim_enrollment(good.clone(), origin).is_err());
    }
    let attempts: u32 = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT attempts FROM client_enrollments WHERE id=?1",
            [&offer.enrollment_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(attempts, 0);
    f.complete(&offer);
}

#[test]
fn foreign_device_and_resource_collisions_roll_back_every_grant_row() {
    let f = Fixture::new(SigningAlgorithm::EdDSA, EnrollmentTransport::Quic);
    let foreign = f.store.create_identity("foreign").unwrap();
    f.store
        .execute_provisioning(
            &f.admin.credential_id,
            ProvisioningCommand::Endpoint {
                operation_id: "foreign-device".into(),
                identity_id: foreign.id,
                device: f.request.device.clone(),
                transport: EndpointTransport::Quic,
                address: "quic://foreign.fixture.invalid:4444".into(),
                capabilities: vec!["control".into()],
                secret_ref: None,
            },
        )
        .unwrap();
    let offer = f.offer();
    let before = f.counts();
    assert!(f.store.claim_enrollment(f.claim(&offer), None).is_err());
    assert_eq!(f.counts(), before);
    let state: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT state FROM client_enrollments WHERE id=?1",
            [&offer.enrollment_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "pending");
}

#[test]
fn approval_rechecks_issuer_scope_tenant_key_validity_and_original_namespace() {
    let f = Fixture::new(SigningAlgorithm::EdDSA, EnrollmentTransport::Quic);
    let before = f.counts();
    for algorithm in [SigningAlgorithm::EdDSA, SigningAlgorithm::ES256] {
        let mut request = f.request.clone();
        request.algorithm = algorithm;
        request.public_key = URL_SAFE_NO_PAD.encode(if algorithm == SigningAlgorithm::EdDSA {
            vec![0; 32]
        } else {
            vec![4; 65]
        });
        assert!(f
            .store
            .approve_enrollment(&f.admin.credential_id, request)
            .is_err());
        assert_eq!(f.counts(), before);
    }
    let limited = f
        .store
        .issue(
            &f.admin.identity_id,
            Scope {
                operations: vec!["admin".into(), "send".into()],
                recipients: vec![f.request.identity_id.clone()],
            },
            crate::now() + 3600,
        )
        .unwrap();
    assert!(f
        .store
        .approve_enrollment(&limited.id, f.request.clone())
        .is_err());
    {
        let c = f.store.connection().unwrap();
        c.execute("INSERT INTO tenants(id) VALUES('tnt_other')", [])
            .unwrap();
        c.execute("INSERT INTO identities(id,alias,tenant_id,capabilities) SELECT 'foreign-tenant-client','foreign-tenant-client','tnt_other',capabilities FROM identities WHERE id=?1",[&f.request.identity_id]).unwrap();
    }
    let mut cross_tenant = f.request.clone();
    cross_tenant.identity_id = "foreign-tenant-client".into();
    let count = f.counts();
    assert!(f
        .store
        .approve_enrollment(&f.admin.credential_id, cross_tenant)
        .is_err());
    assert_eq!(f.counts(), count);
    let offer = f.offer();
    let after = f.counts();
    let other = f
        .store
        .issue(&f.admin.identity_id, Scope::operator(), crate::now() + 3600)
        .unwrap();
    let count = f.counts();
    assert_eq!(
        f.store
            .approve_enrollment(&other.id, f.request.clone())
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(f.counts(), count);
    let mut changed = f.request.clone();
    changed.credential_expires_at += 1;
    assert_eq!(
        f.store
            .approve_enrollment(&f.admin.credential_id, changed)
            .unwrap_err()
            .code,
        "conflict"
    );
    let ordinary = issue_request(&f.request);
    assert_eq!(
        f.store
            .issue_scoped_credential(&f.admin.credential_id, ordinary)
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(f.counts().5, after.5);
    f.complete(&offer);
}

#[test]
fn expiry_refuses_new_claim_but_completed_original_can_recover_without_extending_expiry() {
    let mut f = Fixture::new(SigningAlgorithm::EdDSA, EnrollmentTransport::Quic);
    f.request.challenge_expires_at = crate::now() + 1;
    let offer = f.offer();
    let before = f.counts();
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(
        f.store
            .claim_enrollment(f.claim(&offer), None)
            .unwrap_err()
            .code,
        "gone"
    );
    assert_eq!(f.counts(), before);
    let mut f = Fixture::new(SigningAlgorithm::ES256, EnrollmentTransport::Quic);
    f.request.challenge_expires_at = crate::now() + 2;
    let offer = f.offer();
    let value = f.complete(&offer);
    let count = f.counts();
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(f.complete(&offer), value);
    assert_eq!(f.counts(), count);
}

#[test]
fn pending_and_completed_revocation_are_terminal_and_replay_is_current() {
    let f = Fixture::new(SigningAlgorithm::EdDSA, EnrollmentTransport::Quic);
    let offer = f.offer();
    let result = f
        .store
        .revoke_enrollment(
            &f.admin.credential_id,
            &offer.enrollment_id,
            "revoke-pending",
        )
        .unwrap()
        .outcome()
        .unwrap()
        .clone();
    assert!(result.credential_id.is_none());
    assert!(f.store.claim_enrollment(f.claim(&offer), None).is_err());
    assert_eq!(
        f.store
            .revoke_enrollment(
                &f.admin.credential_id,
                &offer.enrollment_id,
                "revoke-pending"
            )
            .unwrap()
            .outcome()
            .unwrap(),
        &result
    );
    let f = Fixture::new(SigningAlgorithm::ES256, EnrollmentTransport::WssWebrtc);
    let offer = f.offer();
    let value = f.complete(&offer);
    let revoked = f
        .store
        .revoke_enrollment(
            &f.admin.credential_id,
            &offer.enrollment_id,
            "revoke-completed",
        )
        .unwrap()
        .outcome()
        .unwrap()
        .clone();
    assert_eq!(
        revoked.credential_id.as_ref(),
        Some(&value.credential.credential_id)
    );
    assert!(f.store.authenticate(&value.credential.credential).is_err());
    assert!(f
        .store
        .claim_enrollment(f.claim(&offer), offer.request.browser_origin.as_deref())
        .is_err());
    assert!(f
        .store
        .enrollment_credential_binding(&value.credential.credential_id)
        .is_err());
    assert_eq!(
        f.store
            .revoke_enrollment(
                &f.admin.credential_id,
                &offer.enrollment_id,
                "revoke-completed"
            )
            .unwrap()
            .outcome()
            .unwrap(),
        &revoked
    );
}

#[test]
fn rotation_reuses_exact_endpoint_and_old_retirement_preserves_the_new_binding() {
    let mut f = Fixture::new(SigningAlgorithm::EdDSA, EnrollmentTransport::Quic);
    let first = f.offer();
    let old = f.complete(&first);
    f.request.operation_id = "approve-rotation".into();
    f.request.completion_operation_id = "claim-rotation".into();
    f.request.key_id = "key-rotation".into();
    f.key = Key::P256(p256::ecdsa::SigningKey::from_slice(&[11; 32]).unwrap());
    f.request.algorithm = SigningAlgorithm::ES256;
    f.request.public_key = f.key.public();
    let next = f.offer();
    assert_eq!(first.endpoint_id, next.endpoint_id);
    let new = f.complete(&next);
    assert_ne!(old.credential.credential, new.credential.credential);
    f.store
        .revoke_enrollment(
            &f.admin.credential_id,
            &first.enrollment_id,
            "retire-original",
        )
        .unwrap();
    assert!(f.store.authenticate(&old.credential.credential).is_err());
    assert!(f.store.authenticate(&new.credential.credential).is_ok());
    assert!(f
        .store
        .enrollment_credential_binding(&new.credential.credential_id)
        .unwrap()
        .is_some());
    assert_eq!(f.complete(&next), new);
    assert_eq!(f.counts().2, 1);
}

#[test]
fn original_recovery_refuses_revoked_issuer_child_key_endpoint_and_tombstone() {
    for resource in ["issuer", "credential", "key", "endpoint", "tombstone"] {
        let f = Fixture::new(SigningAlgorithm::EdDSA, EnrollmentTransport::Quic);
        let offer = f.offer();
        let value = f.complete(&offer);
        match resource {
            "issuer" => {
                f.store.revoke(&f.admin, &f.admin.credential_id).unwrap();
            }
            "credential" => {
                f.store
                    .revoke(&f.admin, &value.credential.credential_id)
                    .unwrap();
            }
            "key" => {
                f.store
                    .execute_provisioning(
                        &f.admin.credential_id,
                        ProvisioningCommand::RevokeSigningKey {
                            operation_id: "revoke-key".into(),
                            key_id: f.request.key_id.clone(),
                            expected_revision: 0,
                        },
                    )
                    .unwrap();
            }
            "endpoint" => {
                f.store
                    .execute_provisioning(
                        &f.admin.credential_id,
                        ProvisioningCommand::RevokeEndpoint {
                            operation_id: "revoke-endpoint".into(),
                            endpoint_id: offer.endpoint_id.clone(),
                            expected_revision: 0,
                        },
                    )
                    .unwrap();
            }
            _ => {
                f.store.connection().unwrap().execute("UPDATE operations SET tombstoned=1,intent_json=NULL,result_json=NULL WHERE operation_id=?1",[&f.request.completion_operation_id]).unwrap();
            }
        }
        let before = f.counts();
        assert!(
            f.store.claim_enrollment(f.claim(&offer), None).is_err(),
            "{resource}"
        );
        assert!(
            f.store
                .approve_enrollment(&f.admin.credential_id, f.request.clone())
                .is_err(),
            "{resource}"
        );
        assert_eq!(f.counts(), before);
    }
}

#[test]
fn protected_delivery_fence_excludes_revocation_until_owner_drops() {
    let f = Fixture::new(SigningAlgorithm::EdDSA, EnrollmentTransport::Quic);
    let offer = f.offer();
    let owned = f.store.claim_enrollment(f.claim(&offer), None).unwrap();
    let credential = owned.outcome().unwrap().credential.clone();
    let store = f.store.clone();
    let admin = f.admin.clone();
    let id = credential.credential_id.clone();
    let (sent, received) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        sent.send(store.revoke(&admin, &id)).unwrap();
    });
    assert!(received.recv_timeout(Duration::from_millis(150)).is_err());
    assert!(owned.current());
    assert!(f.store.authenticate(&credential.credential).is_ok());
    drop(owned);
    received
        .recv_timeout(Duration::from_secs(3))
        .unwrap()
        .unwrap();
    worker.join().unwrap();
    assert!(f.store.authenticate(&credential.credential).is_err());
}

#[test]
fn snapshot_includes_enrollment_and_unsupported_schema_is_never_implicitly_upgraded() {
    let f = Fixture::new(SigningAlgorithm::EdDSA, EnrollmentTransport::Quic);
    let before = f
        .store
        .backup(&f.dir.path().join("before.sqlite3"))
        .unwrap();
    let offer = f.offer();
    let approved = f
        .store
        .backup(&f.dir.path().join("approved.sqlite3"))
        .unwrap();
    assert_eq!(approved.schema, 115);
    assert_eq!(SCHEMA_VERSION, 115);
    assert_ne!(before.digest, approved.digest);
    let value = f.complete(&offer);
    let completed = f
        .store
        .backup(&f.dir.path().join("completed.sqlite3"))
        .unwrap();
    assert_ne!(completed.digest, approved.digest);
    assert_eq!(
        Store::inspect_snapshot(&f.dir.path().join("completed.sqlite3")).unwrap(),
        completed
    );
    assert!(f
        .store
        .connection()
        .unwrap()
        .execute(
            "UPDATE client_enrollments SET request_json='{}' WHERE id=?1",
            [&offer.enrollment_id]
        )
        .is_err());
    assert!(f
        .store
        .connection()
        .unwrap()
        .execute(
            "UPDATE client_enrollments SET state='pending' WHERE id=?1",
            [&offer.enrollment_id]
        )
        .is_err());
    assert_eq!(f.complete(&offer), value);
    let copy = f.dir.path().join("old.sqlite3");
    f.store.backup(&copy).unwrap();
    let c = Connection::open(&copy).unwrap();
    c.pragma_update(None, "user_version", 114).unwrap();
    drop(c);
    assert!(Store::open_administration(&copy, Default::default()).is_err());
    assert_eq!(
        Connection::open(&copy)
            .unwrap()
            .pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0))
            .unwrap(),
        114
    );
}

#[test]
fn bounded_pending_and_receipt_capacity_refuse_without_partial_approval() {
    let mut f = Fixture::new(SigningAlgorithm::EdDSA, EnrollmentTransport::Quic);
    for i in 0..32 {
        f.request.operation_id = format!("approve-{i}");
        f.request.completion_operation_id = format!("claim-{i}");
        f.request.key_id = format!("key-{i}");
        f.offer();
    }
    let before = f.counts();
    f.request.operation_id = "approve-overflow".into();
    f.request.completion_operation_id = "claim-overflow".into();
    f.request.key_id = "key-overflow".into();
    assert_eq!(
        f.store
            .approve_enrollment(&f.admin.credential_id, f.request.clone())
            .unwrap_err()
            .code,
        "capacity-exceeded"
    );
    assert_eq!(f.counts(), before);
    let mut limited = Fixture::new(SigningAlgorithm::EdDSA, EnrollmentTransport::Quic);
    Arc::get_mut(&mut limited.store)
        .unwrap()
        .limits
        .operation_receipts = 1;
    let before = limited.counts();
    assert_eq!(
        limited
            .store
            .approve_enrollment(&limited.admin.credential_id, limited.request.clone())
            .unwrap_err()
            .code,
        "capacity-exceeded"
    );
    assert_eq!(limited.counts(), before);
    let rows: u64 = limited
        .store
        .connection()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM client_enrollments", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 0);
}

#[test]
fn late_key_collision_rolls_back_the_new_credential_after_endpoint_reuse() {
    let f = Fixture::new(SigningAlgorithm::EdDSA, EnrollmentTransport::Quic);
    f.store
        .execute_provisioning(
            &f.admin.credential_id,
            ProvisioningCommand::Endpoint {
                operation_id: "prepare-device".into(),
                identity_id: f.request.identity_id.clone(),
                device: f.request.device.clone(),
                transport: EndpointTransport::Quic,
                address: f.request.endpoint_address.clone(),
                capabilities: f.request.capabilities.clone(),
                secret_ref: None,
            },
        )
        .unwrap();
    let offer = f.offer();
    let other = f
        .store
        .issue(
            &f.request.identity_id,
            Scope::messaging(f.request.recipients.clone()),
            crate::now() + 600,
        )
        .unwrap();
    f.store
        .execute_provisioning(
            &f.admin.credential_id,
            ProvisioningCommand::SigningKey {
                operation_id: "late-key".into(),
                identity_id: f.request.identity_id.clone(),
                credential_id: other.id,
                device_id: f.request.device.id.clone(),
                key_id: f.request.key_id.clone(),
                algorithm: f.request.algorithm,
                public_key: f.request.public_key.clone(),
                not_before: crate::now(),
                expires_at: f.request.credential_expires_at,
                expected_revision: None,
            },
        )
        .unwrap();
    let before = f.counts();
    assert!(f.store.claim_enrollment(f.claim(&offer), None).is_err());
    assert_eq!(f.counts(), before);
    let credential: Option<String> = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT credential_id FROM client_enrollments WHERE id=?1",
            [&offer.enrollment_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(credential.is_none());
}

#[test]
fn enrolled_metadata_origin_and_exact_key_remain_current_after_endpoint_revocation() {
    for algorithm in [SigningAlgorithm::EdDSA, SigningAlgorithm::ES256] {
        let f = Fixture::new(algorithm, EnrollmentTransport::Webtransport);
        let issued = f.complete(&f.offer());
        let id = &issued.credential.credential_id;
        let origin = f.request.browser_origin.as_deref();
        f.store.credential_metadata(id).unwrap();
        f.store
            .check_enrollment_http_bearer(&issued.credential.credential, origin)
            .unwrap();
        for wrong in [None, Some("https://other.fixture.invalid")] {
            assert!(f
                .store
                .check_enrollment_http_bearer(&issued.credential.credential, wrong)
                .is_err());
        }
        assert!(f
            .store
            .check_unbound_enrollment_bearer(&issued.credential.credential)
            .is_err());
        let control = f
            .store
            .issue(&f.admin.identity_id, Scope::operator(), crate::now() + 600)
            .unwrap();
        f.store
            .check_unbound_enrollment_bearer(&control.token)
            .unwrap();
        f.store
            .execute_provisioning(
                &f.admin.credential_id,
                ProvisioningCommand::SigningKey {
                    operation_id: "attach-other-key".into(),
                    identity_id: f.request.identity_id.clone(),
                    credential_id: id.clone(),
                    device_id: f.request.device.id.clone(),
                    key_id: "other-active-key".into(),
                    algorithm,
                    public_key: f.key.public(),
                    not_before: crate::now() - 1,
                    expires_at: issued.credential.expires_at,
                    expected_revision: None,
                },
            )
            .unwrap();
        let tenant = f.store.credential_metadata(id).unwrap().tenant_id;
        f.store
            .resolve_peer_signing_key(id, &tenant, &f.request.key_id, algorithm.as_str())
            .unwrap();
        assert!(f
            .store
            .resolve_peer_signing_key(id, &tenant, "other-active-key", algorithm.as_str())
            .is_err());
        let mut peer = crate::PeerAuthorityRequest {
            credentials: vec![id.clone()],
            device: crate::PeerDeviceClaim {
                id: f.request.device.id.clone(),
                kind: f.request.device.kind.clone(),
                platform: f.request.device.platform.clone(),
            },
            key_id: f.request.key_id.clone(),
            algorithm: algorithm.as_str().into(),
            command: crate::PeerCommand::Binding,
        };
        drop(
            f.store
                .admit_peer_authority(&peer, &crate::AuthorityWait::new(Duration::from_secs(2)))
                .unwrap(),
        );
        peer.key_id = "other-active-key".into();
        assert!(f
            .store
            .admit_peer_authority(&peer, &crate::AuthorityWait::new(Duration::from_secs(2)))
            .is_err());
        f.store.credential_metadata(id).unwrap();
        f.store
            .execute_provisioning(
                &f.admin.credential_id,
                ProvisioningCommand::RevokeEndpoint {
                    operation_id: "revoke-enrolled-endpoint".into(),
                    endpoint_id: issued.endpoint_id,
                    expected_revision: 0,
                },
            )
            .unwrap();
        assert!(f.store.credential_metadata(id).is_err());
        assert!(f
            .store
            .check_enrollment_http_bearer(&issued.credential.credential, origin)
            .is_err());
    }
    let f = Fixture::new(SigningAlgorithm::EdDSA, EnrollmentTransport::Quic);
    let issued = f.complete(&f.offer());
    f.store
        .check_enrollment_http_bearer(&issued.credential.credential, None)
        .unwrap();
    assert!(f
        .store
        .check_enrollment_http_bearer(
            &issued.credential.credential,
            Some("https://app.fixture.invalid")
        )
        .is_err());
}

#[test]
fn native_task_bearer_cannot_bypass_the_enrolled_parent_origin_ceiling() {
    use connect_protocol::{tasks::*, Operation, Outcome};
    let mut f = Fixture::new(SigningAlgorithm::EdDSA, EnrollmentTransport::Webtransport);
    f.request.operations = vec![
        "create".into(),
        "send".into(),
        "task_create".into(),
        "task_grant".into(),
    ];
    let issued = f.complete(&f.offer());
    let id = &issued.credential.credential_id;
    let peer = f.request.recipients[0].clone();
    let Outcome::Conversation(room) = f
        .store
        .execute(
            id,
            &Operation::Create {
                request_id: "task-room".into(),
                request: connect_protocol::conversations::CreateRequest::persistent(vec![
                    peer.clone()
                ]),
            },
        )
        .unwrap()
    else {
        panic!("room");
    };
    let task = f
        .store
        .execute_task(
            id,
            &TaskRequest::new(TaskCommand::Create {
                request_id: "origin-task".into(),
                definition: TaskDefinition {
                    conversation_id: room.id,
                    scope: TaskScope {
                        sms_enrollments: vec![],
                        operations: vec![TaskPermission::Send],
                        recipients: vec![peer],
                    },
                    expires_at: crate::now() + 120,
                    initiating_event_id: None,
                },
            }),
        )
        .unwrap();
    let TaskOutcome::Record(task) = task else {
        panic!("task");
    };
    let grant = f
        .store
        .issue_local_task_grant(
            id,
            "origin-grant",
            &GrantDefinition {
                task_id: task.id,
                binding: InvocationBinding {
                    kind: InvocationKind::Native,
                    adapter: "fixture".into(),
                    external_id: "origin".into(),
                },
                operations: vec![TaskPermission::Send],
                expires_at: crate::now() + 60,
            },
        )
        .unwrap();
    f.store
        .check_enrollment_http_bearer(grant.expose_secret(), f.request.browser_origin.as_deref())
        .unwrap();
    assert!(f
        .store
        .check_enrollment_http_bearer(grant.expose_secret(), None)
        .is_err());
    assert!(f
        .store
        .check_enrollment_http_bearer(grant.expose_secret(), Some("https://other.fixture.invalid"))
        .is_err());
    assert!(f
        .store
        .check_unbound_enrollment_bearer(grant.expose_secret())
        .is_err());
}
