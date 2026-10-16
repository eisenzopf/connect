//! Trusted backend provisioning is durable, scoped and independent of sessions.
#![allow(clippy::unwrap_used)]
use connect_protocol::{provisioning::*, Limits};
use connect_store::{Scope, Store};
use rusqlite::{params, Connection};
use std::sync::Arc;

struct Fixture {
    _dir: tempfile::TempDir,
    store: Arc<Store>,
    admin: String,
    owner: String,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(
            Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap(),
        );
        let root = store.create_identity("root").unwrap();
        let issued = store
            .issue(
                &root.id,
                Scope::operator(),
                chrono::Utc::now().timestamp() + 3600,
            )
            .unwrap();
        Self {
            _dir: dir,
            store,
            admin: issued.id,
            owner: root.id,
        }
    }
    fn identity(&self, operation: &str, alias: &str) -> String {
        match self
            .store
            .execute_provisioning(&self.admin, identity_command(operation, alias))
            .unwrap()
        {
            ProvisioningOutcome::Identity { identity_id, .. } => identity_id,
            _ => panic!("identity outcome"),
        }
    }
    fn connection(&self) -> Connection {
        Connection::open(self.store.path()).unwrap()
    }
}
fn identity_command(operation: &str, alias: &str) -> ProvisioningCommand {
    ProvisioningCommand::Identity {
        operation_id: operation.into(),
        alias: alias.into(),
        kind: IdentityKind::Human,
        display_name: Some("Fixture contact".into()),
    }
}
fn endpoint_command(
    operation: &str,
    owner: &str,
    device: &str,
    transport: EndpointTransport,
    address: &str,
) -> ProvisioningCommand {
    ProvisioningCommand::Endpoint {
        operation_id: operation.into(),
        identity_id: owner.into(),
        device: DeviceSpec {
            id: device.into(),
            kind: "test".into(),
            platform: "fixture".into(),
        },
        transport,
        address: address.into(),
        capabilities: vec!["audio".into(), "messaging".into()],
        secret_ref: Some("env_TEST_PROVIDER_KEY".into()),
    }
}
fn endpoint_id(outcome: ProvisioningOutcome) -> String {
    match outcome {
        ProvisioningOutcome::Endpoint { endpoint_id, .. } => endpoint_id,
        _ => panic!("endpoint"),
    }
}

#[test]
fn identity_provisioning_replays_and_conflicts_without_adopting_aliases() {
    let f = Fixture::new();
    let original = f
        .store
        .execute_provisioning(&f.admin, identity_command("create", "alice"))
        .unwrap();
    assert_eq!(
        f.store
            .execute_provisioning(&f.admin, identity_command("create", "alice"))
            .unwrap(),
        original
    );
    assert_eq!(
        f.store
            .execute_provisioning(&f.admin, identity_command("create", "bob"))
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(
        f.store
            .execute_provisioning(&f.admin, identity_command("different", "alice"))
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(
        f.connection()
            .query_row(
                "SELECT COUNT(*) FROM identities WHERE alias='alice'",
                [],
                |r| r.get::<_, u32>(0)
            )
            .unwrap(),
        1
    );
    let count = f
        .connection()
        .query_row("SELECT COUNT(*) FROM operations", [], |r| {
            r.get::<_, u32>(0)
        })
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn all_endpoint_kinds_require_separate_proof_and_keep_server_routes_private() {
    let f = Fixture::new();
    let alice = f.identity("alice", "alice");
    for (index, (transport, address)) in [
        (EndpointTransport::Quic, "quic://native.example:4433"),
        (
            EndpointTransport::Webtransport,
            "https://browser.example/control",
        ),
        (EndpointTransport::Webrtc, "https://rtc.example/control"),
        (EndpointTransport::Sip, "sip:alice@example.test:5060"),
        (EndpointTransport::Telephone, "+12025550101"),
        (EndpointTransport::Sms, "+12025550101"),
        (EndpointTransport::VapiVoice, "assistant_fixture"),
        (EndpointTransport::VapiChat, "assistant_fixture"),
    ]
    .into_iter()
    .enumerate()
    {
        let command = endpoint_command(
            &format!("endpoint_{index}"),
            &alice,
            &format!("device_{index}"),
            transport,
            address,
        );
        let outcome = f
            .store
            .execute_provisioning(&f.admin, command.clone())
            .unwrap();
        assert!(
            matches!(&outcome, ProvisioningOutcome::Endpoint { verification_state, revision:0,.. } if verification_state=="unverified")
        );
        let serialized = serde_json::to_string(&outcome).unwrap();
        assert!(!serialized.contains(address));
        assert!(!serialized.contains("env_TEST_PROVIDER_KEY"));
        let id = endpoint_id(outcome);
        let verified = f
            .store
            .execute_provisioning(
                &f.admin,
                ProvisioningCommand::VerifyEndpoint {
                    operation_id: format!("verify_{index}"),
                    endpoint_id: id.clone(),
                    expected_revision: 0,
                    evidence_ref: "proof_fixture".into(),
                },
            )
            .unwrap();
        assert!(
            matches!(verified, ProvisioningOutcome::Endpoint { verification_state,revision:1,.. } if verification_state=="verified")
        );
        let (owner, secret): (String, String) = f
            .connection()
            .query_row(
                "SELECT route_owner_id,secret_ref FROM endpoints WHERE id=?1",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(owner, f.owner);
        assert_eq!(secret, "env_TEST_PROVIDER_KEY");
        // Replaying creation observes the current resource instead of restoring
        // the original unverified state or reserving a second endpoint.
        assert!(matches!(
            f.store.execute_provisioning(&f.admin, command).unwrap(),
            ProvisioningOutcome::Endpoint { revision: 1, .. }
        ));
    }
    assert_eq!(
        f.connection()
            .query_row("SELECT COUNT(*) FROM endpoints", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        8
    );
}

#[test]
fn duplicate_claims_and_foreign_device_ownership_roll_back_atomically() {
    let f = Fixture::new();
    let alice = f.identity("alice", "alice");
    let bob = f.identity("bob", "bob");
    f.store
        .execute_provisioning(
            &f.admin,
            endpoint_command(
                "first",
                &alice,
                "device_one",
                EndpointTransport::Telephone,
                "+12025550101",
            ),
        )
        .unwrap();
    assert_eq!(
        f.store
            .execute_provisioning(
                &f.admin,
                endpoint_command(
                    "duplicate",
                    &bob,
                    "new_device",
                    EndpointTransport::Telephone,
                    "+12025550101"
                )
            )
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(
        f.store
            .execute_provisioning(
                &f.admin,
                endpoint_command(
                    "foreign",
                    &bob,
                    "device_one",
                    EndpointTransport::Telephone,
                    "+12025550102"
                )
            )
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(
        f.connection()
            .query_row("SELECT COUNT(*) FROM devices", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        f.connection()
            .query_row("SELECT COUNT(*) FROM endpoints", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        1
    );
}

#[test]
fn endpoint_revocation_is_terminal_and_stale_verification_cannot_restore_it() {
    let f = Fixture::new();
    let alice = f.identity("alice", "alice");
    let create = endpoint_command(
        "phone",
        &alice,
        "device_one",
        EndpointTransport::Telephone,
        "+12025550101",
    );
    let id = endpoint_id(
        f.store
            .execute_provisioning(&f.admin, create.clone())
            .unwrap(),
    );
    let revoke = ProvisioningCommand::RevokeEndpoint {
        operation_id: "revoke".into(),
        endpoint_id: id.clone(),
        expected_revision: 0,
    };
    let revoked = f
        .store
        .execute_provisioning(&f.admin, revoke.clone())
        .unwrap();
    assert_eq!(
        f.store.execute_provisioning(&f.admin, revoke).unwrap(),
        revoked
    );
    assert!(
        matches!(f.store.execute_provisioning(&f.admin,create).unwrap(),ProvisioningOutcome::Endpoint { verification_state,.. } if verification_state=="revoked")
    );
    assert_eq!(
        f.store
            .execute_provisioning(
                &f.admin,
                ProvisioningCommand::VerifyEndpoint {
                    operation_id: "late".into(),
                    endpoint_id: id.clone(),
                    expected_revision: 1,
                    evidence_ref: "late_proof".into()
                }
            )
            .unwrap_err()
            .code,
        "conflict"
    );
    let c = f.connection();
    assert_eq!(
        c.query_row("SELECT secret_ref FROM endpoints WHERE id=?1", [&id], |r| r
            .get::<_, Option<String>>(0))
            .unwrap(),
        None
    );
    assert!(c
        .execute(
            "UPDATE endpoints SET verification_state='verified' WHERE id=?1",
            [&id]
        )
        .is_err());
    assert!(c
        .execute(
            "UPDATE endpoints SET address='+12025550103' WHERE id=?1",
            [&id]
        )
        .is_err());
}

#[test]
fn identity_revocation_retires_credentials_devices_and_endpoints_and_denies_new_issue() {
    let f = Fixture::new();
    let alice = f.identity("alice", "alice");
    let issued = f
        .store
        .issue(
            &alice,
            Scope::messaging(vec![]),
            chrono::Utc::now().timestamp() + 3600,
        )
        .unwrap();
    let id = endpoint_id(
        f.store
            .execute_provisioning(
                &f.admin,
                endpoint_command(
                    "phone",
                    &alice,
                    "device_one",
                    EndpointTransport::Telephone,
                    "+12025550101",
                ),
            )
            .unwrap(),
    );
    f.store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::RevokeIdentity {
                operation_id: "revoke_alice".into(),
                identity_id: alice.clone(),
                expected_revision: 0,
            },
        )
        .unwrap();
    assert!(f.store.authenticate(&issued.token).is_err());
    assert!(f.store.revalidate(&issued.id).is_err());
    assert!(f
        .store
        .issue(
            &alice,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 3600
        )
        .is_err());
    let c = f.connection();
    assert_eq!(
        c.query_row(
            "SELECT verification_state FROM endpoints WHERE id=?1",
            [&id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "revoked"
    );
    assert_eq!(
        c.query_row(
            "SELECT revoked FROM devices WHERE id='device_one'",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap(),
        true
    );
    assert!(f
        .store
        .execute_provisioning(
            &f.admin,
            endpoint_command(
                "new_phone",
                &alice,
                "new_device",
                EndpointTransport::Telephone,
                "+12025550102"
            )
        )
        .is_err());
}

#[test]
fn provisioning_rechecks_administrator_tenant_and_recipient_authority_on_replay() {
    let f = Fixture::new();
    let alice = f.identity("alice", "alice");
    let ordinary = f
        .store
        .issue(
            &alice,
            Scope::communications(vec!["*".into()]),
            chrono::Utc::now().timestamp() + 3600,
        )
        .unwrap();
    assert_eq!(
        f.store
            .execute_provisioning(
                &ordinary.id,
                identity_command("unauthorized", "private-alias")
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    let c = f.connection();
    c.execute_batch("INSERT INTO tenants(id) VALUES('tenant_other');INSERT INTO identities(id,alias,capabilities,tenant_id) VALUES('id_other','other','[]','tenant_other');").unwrap();
    assert_eq!(
        f.store
            .execute_provisioning(
                &f.admin,
                endpoint_command(
                    "cross_tenant",
                    "id_other",
                    "device_other",
                    EndpointTransport::Telephone,
                    "+12025550105"
                )
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    let create = identity_command("replay", "replay-contact");
    f.store
        .execute_provisioning(&f.admin, create.clone())
        .unwrap();
    c.execute(
        "UPDATE credentials SET operations='[\"health\"]' WHERE id=?1",
        [&f.admin],
    )
    .unwrap();
    assert_eq!(
        f.store
            .execute_provisioning(&f.admin, create)
            .unwrap_err()
            .code,
        "forbidden"
    );
}

#[test]
fn endpoint_provisioning_is_reusable_across_conversations_without_history_access() {
    let f = Fixture::new();
    let alice = f.identity("alice", "alice");
    let endpoint = endpoint_id(
        f.store
            .execute_provisioning(
                &f.admin,
                endpoint_command(
                    "phone",
                    &alice,
                    "device_one",
                    EndpointTransport::Telephone,
                    "+12025550101",
                ),
            )
            .unwrap(),
    );
    let c = f.connection();
    c.execute_batch("INSERT INTO conversations(id) VALUES('conv_one'),('conv_two');")
        .unwrap();
    let mut participants = Vec::new();
    for (index, conversation) in ["conv_one", "conv_two"].into_iter().enumerate() {
        let outcome = f
            .store
            .execute_provisioning(
                &f.admin,
                ProvisioningCommand::Participant {
                    operation_id: format!("member_{index}"),
                    conversation_id: conversation.into(),
                    identity_id: alice.clone(),
                    role: "agent".into(),
                },
            )
            .unwrap();
        if let ProvisioningOutcome::Participant { participant_id, .. } = outcome {
            participants.push(participant_id);
        }
    }
    assert_ne!(participants[0], participants[1]);
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM endpoints WHERE id=?1 AND identity_id=?2",
            params![endpoint, alice],
            |r| r.get::<_, u32>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM participants WHERE identity_id=?1",
            [&f.owner],
            |r| r.get::<_, u32>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn invalid_addresses_plaintext_passwords_unknown_fields_and_forged_verification_are_refused() {
    let f = Fixture::new();
    let alice = f.identity("alice", "alice");
    for (transport, address) in [
        (EndpointTransport::Telephone, "not-a-phone"),
        (EndpointTransport::Telephone, "+02025550101"),
        (EndpointTransport::Sip, "sip:alice:password@example.test"),
        (
            EndpointTransport::Webrtc,
            "https://example.test/?secret=value",
        ),
        (EndpointTransport::Quic, "quic://missing-port.example"),
        (
            EndpointTransport::Webtransport,
            "https://user:password@example.test",
        ),
    ] {
        assert_eq!(
            f.store
                .execute_provisioning(
                    &f.admin,
                    endpoint_command("invalid", &alice, "device_one", transport, address)
                )
                .unwrap_err()
                .code,
            "invalid-request"
        );
    }
    let mut value = serde_json::to_value(endpoint_command(
        "forged",
        &alice,
        "device_one",
        EndpointTransport::Telephone,
        "+12025550101",
    ))
    .unwrap();
    value["verified"] = serde_json::Value::Bool(true);
    assert!(serde_json::from_value::<ProvisioningCommand>(value).is_err());
    assert_eq!(
        f.connection()
            .query_row("SELECT COUNT(*) FROM endpoints", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        0
    );
}

#[test]
fn concurrent_retries_share_one_identity_and_current_schema_reopens() {
    let f = Fixture::new();
    let threads = (0..4)
        .map(|_| {
            let store = f.store.clone();
            let admin = f.admin.clone();
            std::thread::spawn(move || {
                store
                    .execute_provisioning(&admin, identity_command("concurrent", "same-person"))
                    .unwrap()
            })
        })
        .collect::<Vec<_>>();
    let results = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect::<Vec<_>>();
    assert!(results.iter().all(|value| value == &results[0]));
    let reopened = Store::open(f.store.path(), Limits::default()).unwrap();
    assert_eq!(
        reopened
            .execute_provisioning(&f.admin, identity_command("concurrent", "same-person"))
            .unwrap(),
        results[0]
    );
}

#[test]
fn expired_receipts_retain_digest_tombstones_and_cannot_execute_again() {
    let f = Fixture::new();
    let command = identity_command("tombstone", "once-only");
    f.store
        .execute_provisioning(&f.admin, command.clone())
        .unwrap();
    f.connection().execute("UPDATE operations SET tombstoned=1,intent_json=NULL,result_json=NULL WHERE operation_id='tombstone'",[]).unwrap();
    assert_eq!(
        f.store
            .execute_provisioning(&f.admin, command)
            .unwrap_err()
            .code,
        "gone"
    );
    assert_eq!(
        f.store
            .execute_provisioning(&f.admin, identity_command("tombstone", "different-alias"))
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(
        f.connection()
            .query_row(
                "SELECT COUNT(*) FROM identities WHERE alias='once-only'",
                [],
                |r| r.get::<_, u32>(0)
            )
            .unwrap(),
        1
    );
}

#[test]
fn previous_gate6_candidate_schema_is_refused_without_touching_existing_state() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("candidate19.sqlite3");
    let store = Store::initialize(&path, Limits::default()).unwrap();
    drop(store);
    let c = Connection::open(&path).unwrap();
    c.pragma_update(None, "user_version", 19).unwrap();
    c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
    drop(c);
    let before = std::fs::read(&path).unwrap();
    assert!(
        matches!(Store::open(&path,Limits::default()),Err(error) if error.code=="unsupported-schema")
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
}
