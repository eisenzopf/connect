//! Actual trusted HTTP mutations and signed original-ID status/recovery.
use super::*;
use connect_client::{application::ApplicationClientConfig, sdk::AdministrativeClient};
#[path = "administration/node.rs"]
mod node;
struct Administrator {
    rust: AdministrativeClient,
    node: Option<node::Probe>,
}
impl std::ops::Deref for Administrator {
    type Target = AdministrativeClient;
    fn deref(&self) -> &Self::Target {
        &self.rust
    }
}
impl Administrator {
    async fn close(&self, options: RequestOptions) -> sdk::Result<()> {
        if let Some(node) = &self.node {
            node.close().await;
        }
        self.rust.close(options).await
    }
}
fn app(f: &Fixture, config: &CanonicalClientConfig) -> ApplicationClientConfig {
    ApplicationClientConfig {
        endpoint: f.server.http_address.unwrap(),
        credential: config.credential.clone(),
        expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
        timeout_ms: 5000,
    }
}
fn administrator(f: &Fixture, index: usize) -> CanonicalClientConfig {
    super::super::canonical_native_connections::party_with_scope(
        f,
        &f.client.identity_id,
        index,
        vec!["data".into()],
        Scope::operator(),
    )
}
async fn connect(f: &Fixture, config: CanonicalClientConfig) -> Administrator {
    let rust = AdministrativeClient::connect(
        config.clone(),
        "tnt_default".parse().unwrap(),
        app(f, &config),
    )
    .await
    .unwrap();
    let node = if node::enabled() {
        Some(node::Probe::start(f, config).await)
    } else {
        None
    };
    Administrator { rust, node }
}
async fn apply(admin: &Administrator, command: ProvisioningCommand) -> (Value, Value) {
    let id = command.operation_id().to_owned();
    let intent = admin
        .validate_provisioning(serde_json::to_value(command).unwrap())
        .unwrap();
    let saved = intent.reference_json();
    if let Some(node) = &admin.node {
        let result = node.apply(intent.to_json()).await;
        assert_eq!(result["reference"], saved);
        let independent = admin
            .restore(saved.clone())
            .unwrap()
            .wait(options())
            .await
            .unwrap();
        assert_eq!(result["record_id"], json!(independent.snapshot.record_id));
        assert_eq!(result["value"], independent.value);
        return (result["value"].clone(), saved);
    }
    let operation = admin.provision(intent, mutation(&id)).await.unwrap();
    assert_eq!(saved, operation.reference_json());
    let value = operation.wait(options()).await.unwrap().value;
    (value, saved)
}
fn id(value: &Value, field: &str) -> String {
    value[field].as_str().unwrap().into()
}
fn saved(operation: &str) -> Value {
    json!({"schema":"connect-operation-reference/1","operation_id":operation,"command":"identity.provision","scope":{}})
}
fn identity(operation: &str, alias: &str) -> ProvisioningCommand {
    ProvisioningCommand::Identity {
        operation_id: operation.into(),
        alias: alias.into(),
        kind: IdentityKind::Human,
        display_name: Some("private backend label".into()),
    }
}
#[tokio::test]
async fn public_sdk_trusted_administration_covers_every_command_and_recovers_original_receipts_after_restart(
) {
    for algorithm in ["EdDSA", "ES256"] {
        administration_case(algorithm).await;
    }
}
async fn administration_case(algorithm: &'static str) {
    let mut f = Fixture::with_native_end_scope(algorithm, 16).await;
    let mut config = administrator(&f, 239);
    let admin = connect(&f, config.clone()).await;
    let original = identity("sdk-admin-identity", "sdk-admin-target");
    let intent = admin
        .validate_provisioning(serde_json::to_value(&original).unwrap())
        .unwrap();
    assert!(!format!("{intent:?}").contains("private backend label"));
    let (created, original_ref) = apply(&admin, original.clone()).await;
    let target = id(&created, "identity_id");
    let caller = Client::connect(f.client.clone(), "tnt_default".parse().unwrap())
        .await
        .unwrap();
    let room = caller
        .create_conversation(
            vec![],
            ConversationPolicy::Persistent,
            mutation("sdk-admin-room"),
        )
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap()
        .value;
    let (participant, _) = apply(
        &admin,
        ProvisioningCommand::Participant {
            operation_id: "sdk-admin-participant".into(),
            conversation_id: room.id().as_str().into(),
            identity_id: target.clone(),
            role: "member".into(),
        },
    )
    .await;
    assert_eq!(participant["conversation_id"], room.id().as_str());
    assert!(room
        .refresh(options())
        .await
        .unwrap()
        .participants
        .iter()
        .any(|p| p.id.as_str() == participant["participant_id"].as_str().unwrap()));
    let (role, _) = apply(
        &admin,
        ProvisioningCommand::RecordReadRole {
            operation_id: "sdk-admin-record-role".into(),
            identity_id: target.clone(),
            role: "operator".into(),
            private_sources: false,
            active: true,
            expected_revision: None,
        },
    )
    .await;
    assert_eq!(role["active"], true);
    let (endpoint, endpoint_ref) = apply(
        &admin,
        ProvisioningCommand::Endpoint {
            operation_id: "sdk-admin-endpoint".into(),
            identity_id: target.clone(),
            device: DeviceSpec {
                id: "sdk-admin-target-device".into(),
                kind: "desktop".into(),
                platform: "isolated-administrative-test".into(),
            },
            transport: EndpointTransport::Quic,
            address: format!(
                "quic://localhost:{}/sdk-admin-target",
                f.server.address.port()
            ),
            capabilities: vec!["data".into()],
            secret_ref: None,
        },
    )
    .await;
    let endpoint_id = id(&endpoint, "endpoint_id");
    assert!(endpoint.get("address").is_none());
    let (verified, _) = apply(
        &admin,
        ProvisioningCommand::VerifyEndpoint {
            operation_id: "sdk-admin-verify".into(),
            endpoint_id: endpoint_id.clone(),
            expected_revision: endpoint["revision"].as_u64().unwrap(),
            evidence_ref: "isolated-verified-admin-endpoint".into(),
        },
    )
    .await;
    assert_eq!(verified["verification_state"], "verified");
    assert_eq!(
        admin
            .restore(endpoint_ref)
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value["verification_state"],
        "verified"
    );
    // Fixture credential issuance supplies an independent enrollment target; the
    // application obtains resource IDs and performs all administrative mutations
    // exclusively through the public AdministrativeClient.
    let credential = f
        .server
        .store
        .issue(
            &target,
            Scope::messaging(vec![]),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let public = signing(&f.temp.path().join("admin-target-key.der"), algorithm);
    let (key, _) = apply(
        &admin,
        ProvisioningCommand::SigningKey {
            operation_id: "sdk-admin-signing".into(),
            identity_id: target.clone(),
            credential_id: credential.id.clone(),
            device_id: "sdk-admin-target-device".into(),
            key_id: "sdk-admin-target-key".into(),
            algorithm: if algorithm == "EdDSA" {
                SigningAlgorithm::EdDSA
            } else {
                SigningAlgorithm::ES256
            },
            public_key: URL_SAFE_NO_PAD.encode(public),
            not_before: chrono::Utc::now().timestamp() - 1,
            expires_at: chrono::Utc::now().timestamp() + 600,
            expected_revision: None,
        },
    )
    .await;
    let (revoked, _) = apply(
        &admin,
        ProvisioningCommand::RevokeSigningKey {
            operation_id: "sdk-admin-revoke-signing".into(),
            key_id: id(&key, "key_id"),
            expected_revision: key["revision"].as_u64().unwrap(),
        },
    )
    .await;
    assert_eq!(revoked["revoked"], true);
    let (revoked, _) = apply(
        &admin,
        ProvisioningCommand::RevokeCredential {
            operation_id: "sdk-admin-revoke-credential".into(),
            credential_id: credential.id,
        },
    )
    .await;
    assert_eq!(revoked["revoked"], true);
    apply(
        &admin,
        ProvisioningCommand::RevokeEndpoint {
            operation_id: "sdk-admin-revoke-endpoint".into(),
            endpoint_id,
            expected_revision: verified["revision"].as_u64().unwrap(),
        },
    )
    .await;
    let (device, _) = apply(
        &admin,
        ProvisioningCommand::RevokeDevice {
            operation_id: "sdk-admin-revoke-device".into(),
            device_id: "sdk-admin-target-device".into(),
            expected_revision: 0,
        },
    )
    .await;
    assert_eq!(device["revoked"], true);
    let (revoked, _) = apply(
        &admin,
        ProvisioningCommand::RevokeIdentity {
            operation_id: "sdk-admin-revoke-identity".into(),
            identity_id: target.clone(),
            expected_revision: created["revision"].as_u64().unwrap(),
        },
    )
    .await;
    assert_eq!(revoked["revoked"], true);
    assert_eq!(
        admin
            .restore(original_ref.clone())
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value["revoked"],
        true
    );
    caller.close(options()).await.unwrap();
    admin.close(options()).await.unwrap();
    if node::enabled() {
        node::drained(&f).await;
    }
    f.server.shutdown().await.unwrap();
    node::server_joined();
    f.server = RunningCanonicalServer::start(f.config.clone())
        .await
        .unwrap();
    let original_record = admin
        .node
        .as_ref()
        .map(|node| node.record("sdk-admin-identity"));
    config.endpoint = f.server.address;
    let reopened = connect(&f, config).await;
    if let Some(node) = &reopened.node {
        let result = node.read(original_ref.clone(), original_record).await;
        assert_eq!(result["value"]["identity_id"], target);
        assert_eq!(result["value"]["revoked"], true);
    }
    let recovered = reopened
        .restore(original_ref)
        .unwrap()
        .wait(options())
        .await
        .unwrap();
    assert_eq!(recovered.value["identity_id"], target);
    assert_eq!(recovered.value["revoked"], true);
    let (replayed, _) = apply(&reopened, original).await;
    assert_eq!(replayed["identity_id"], target);
    let changed = reopened
        .validate_provisioning(
            serde_json::to_value(identity("sdk-admin-identity", "changed-admin-alias")).unwrap(),
        )
        .unwrap();
    if let Some(node) = &reopened.node {
        node.refuse(changed.to_json(), "conflict", "refused").await;
    }
    let refused = reopened
        .provision(changed, mutation("sdk-admin-identity"))
        .await
        .err()
        .unwrap();
    assert_eq!(refused.kind, sdk::ErrorKind::Conflict);
    assert_eq!(refused.certainty, sdk::Certainty::Refused);
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM operations WHERE operation_id='sdk-admin-identity'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    drop(db);
    reopened.close(options()).await.unwrap();
    if node::enabled() {
        node::drained(&f).await;
    }
    f.server.shutdown().await.unwrap();
    node::server_joined();
    eprintln!("SDK administration {algorithm}: all11 provisioning command families, current metadata/status, no secret address output, stable original IDs, one receipt across restart/exact replay, changed-intent refusal and joined close");
}
#[tokio::test]
async fn public_sdk_administration_requires_current_administrator_for_submission_status_and_replay()
{
    for algorithm in ["EdDSA", "ES256"] {
        authority_case(algorithm).await;
    }
}
async fn authority_case(algorithm: &'static str) {
    let f = Fixture::with_native_end_scope(algorithm, 16).await;
    let config = administrator(&f, 240);
    let manager = connect(&f, config.clone()).await;
    let (value, reference) = apply(
        &manager,
        identity("sdk-admin-owned", "sdk-admin-owned-target"),
    )
    .await;
    let ordinary = connect(&f, f.client.clone()).await;
    let command = ordinary
        .validate_provisioning(
            serde_json::to_value(identity(
                "sdk-admin-forbidden",
                "sdk-admin-forbidden-target",
            ))
            .unwrap(),
        )
        .unwrap();
    if let Some(node) = &ordinary.node {
        node.refuse(command.to_json(), "authentication", "refused")
            .await;
        node.read_refused(reference.clone(), false).await;
    }
    let error = ordinary
        .provision(command, mutation("sdk-admin-forbidden"))
        .await
        .err()
        .unwrap();
    assert_eq!(error.kind, sdk::ErrorKind::Authentication);
    assert_eq!(error.certainty, sdk::Certainty::Refused);
    assert!(ordinary
        .restore(reference.clone())
        .unwrap()
        .status(options())
        .await
        .is_err());
    let mut wrong_app = app(&f, &config);
    wrong_app.credential = f.client.credential.clone();
    assert!(
        matches!(AdministrativeClient::connect(config.clone(),"tnt_default".parse().unwrap(),wrong_app).await,Err(e) if e.kind==sdk::ErrorKind::Configuration)
    );
    let other = administrator(&f, 241);
    let revoker = connect(&f, other).await;
    let credential_id = f
        .server
        .store
        .authenticate(&config.credential)
        .unwrap()
        .credential_id;
    apply(
        &revoker,
        ProvisioningCommand::RevokeCredential {
            operation_id: "sdk-admin-disable-manager".into(),
            credential_id,
        },
    )
    .await;
    if let Some(node) = &manager.node {
        node.read_refused(reference.clone(), true).await;
        // A fresh revoked owner cannot submit a replay or a new mutation. It
        // exercises real initialization refusal without reconnecting the old owner.
        let revoked = node::Probe::start(&f, config.clone()).await;
        revoked
            .refuse(
                serde_json::to_value(identity("sdk-admin-owned", "sdk-admin-owned-target"))
                    .unwrap(),
                "authentication",
                "not-submitted",
            )
            .await;
        revoked
            .refuse(
                serde_json::to_value(identity(
                    "sdk-admin-after-revoke",
                    "sdk-admin-after-revoke-target",
                ))
                .unwrap(),
                "authentication",
                "not-submitted",
            )
            .await;
        revoked.close().await;
    }
    assert!(manager
        .restore(reference)
        .unwrap()
        .status(options())
        .await
        .is_err());
    let command = manager
        .validate_provisioning(
            serde_json::to_value(identity(
                "sdk-admin-after-revoke",
                "sdk-admin-after-revoke-target",
            ))
            .unwrap(),
        )
        .unwrap();
    assert!(manager
        .provision(command, mutation("sdk-admin-after-revoke"))
        .await
        .is_err());
    if let Some(node) = &revoker.node {
        let recovered = node
            .read(
                saved("sdk-admin-owned"),
                Some(manager.node.as_ref().unwrap().record("sdk-admin-owned")),
            )
            .await;
        assert_eq!(recovered["value"], value);
    }
    assert_eq!(
        revoker
            .restore(saved("sdk-admin-owned"))
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value["identity_id"],
        value["identity_id"]
    );
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let count:i64=db.query_row("SELECT COUNT(*) FROM operations WHERE operation_id IN ('sdk-admin-forbidden','sdk-admin-after-revoke')",[],|r|r.get(0)).unwrap();
    assert_eq!(count, 0);
    let original_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM operations WHERE operation_id='sdk-admin-owned'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(original_count, 1);
    drop(db);
    ordinary.close(options()).await.unwrap();
    manager.close(options()).await.unwrap();
    revoker.close(options()).await.unwrap();
    if node::enabled() {
        node::drained(&f).await;
    }
    f.server.shutdown().await.unwrap();
    node::server_joined();
    eprintln!("SDK administrative authority {algorithm}: ordinary refusal, exact-credential config boundary, current revocation blocks submit/read/replay without new rows, independent authorized recovery and cleanup");
}
#[tokio::test]
async fn public_sdk_cancelled_administrative_http_wait_keeps_real_committed_operation_and_no_queued_effect(
) {
    for algorithm in ["EdDSA", "ES256"] {
        uncertainty_case(algorithm).await;
    }
}
async fn uncertainty_case(algorithm: &'static str) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let f = Fixture::with_native_end_scope(algorithm, 16).await;
    let config = administrator(&f, 242);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = listener.local_addr().unwrap();
    let upstream = f.server.http_address.unwrap();
    let ready = std::sync::Arc::new(tokio::sync::Notify::new());
    let release = std::sync::Arc::new(tokio::sync::Notify::new());
    let entered = ready.clone();
    let gate = release.clone();
    let proxy = tokio::spawn(async move {
        let (mut downstream, _) = listener.accept().await.unwrap();
        let mut header = vec![];
        let mut b = [0; 1];
        while !header.ends_with(b"\r\n\r\n") {
            downstream.read_exact(&mut b).await.unwrap();
            header.push(b[0]);
            assert!(header.len() <= 8192);
        }
        let text = String::from_utf8(header).unwrap();
        let len: usize = text
            .lines()
            .find_map(|l| {
                l.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .map(|n| n.trim().parse().unwrap())
            })
            .unwrap();
        assert!(len <= 65_536);
        let mut body = vec![0; len];
        downstream.read_exact(&mut body).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap()["operation_id"],
            "sdk-admin-uncertain"
        );
        let mut request = text
            .lines()
            .filter(|l| {
                !l.to_ascii_lowercase().starts_with("connection:")
                    && !l.to_ascii_lowercase().starts_with("host:")
                    && !l.is_empty()
            })
            .collect::<Vec<_>>()
            .join("\r\n");
        request += &format!("\r\nHost: {upstream}\r\nConnection: close\r\n\r\n");
        let mut server = tokio::net::TcpStream::connect(upstream).await.unwrap();
        server.write_all(request.as_bytes()).await.unwrap();
        server.write_all(&body).await.unwrap();
        let mut response = vec![];
        tokio::time::timeout(
            Duration::from_secs(5),
            server.take(2 * 1024 * 1024 + 1).read_to_end(&mut response),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(response.len() <= 2 * 1024 * 1024);
        assert!(response.starts_with(b"HTTP/1.1 200"));
        entered.notify_one();
        gate.notified().await;
        let _ = downstream.write_all(&response).await;
        let _ = downstream.shutdown().await;
        assert!(
            tokio::time::timeout(Duration::from_millis(100), listener.accept())
                .await
                .is_err(),
            "no hidden provisioning resubmission"
        );
    });
    if node::enabled() {
        node::uncertainty(f, config, endpoint, ready, release, proxy).await;
        return;
    }
    let mut application = app(&f, &config);
    application.endpoint = endpoint;
    let admin = AdministrativeClient::connect(config, "tnt_default".parse().unwrap(), application)
        .await
        .unwrap();
    let mut opts = mutation("sdk-admin-uncertain");
    let cancel = opts.request.signal.clone();
    opts.request.timeout = Duration::from_secs(5);
    let original = admin.provision(
        admin
            .validate_provisioning(
                serde_json::to_value(identity(
                    "sdk-admin-uncertain",
                    "sdk-admin-uncertain-target",
                ))
                .unwrap(),
            )
            .unwrap(),
        opts,
    );
    tokio::pin!(original);
    tokio::select! {result=&mut original=>panic!("unexpected response before held receipt: {}",result.is_ok()),_=ready.notified()=>()};
    let queued_opts = mutation("sdk-admin-never-written");
    let queued_cancel = queued_opts.request.signal.clone();
    let queued = admin.provision(
        admin
            .validate_provisioning(
                serde_json::to_value(identity(
                    "sdk-admin-never-written",
                    "sdk-admin-never-written-target",
                ))
                .unwrap(),
            )
            .unwrap(),
        queued_opts,
    );
    tokio::pin!(queued);
    std::future::poll_fn(|cx| {
        use std::future::Future;
        assert!(queued.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    queued_cancel.cancel();
    assert_eq!(queued.await.err().unwrap().kind, sdk::ErrorKind::Cancelled);
    cancel.cancel();
    let error = original.await.err().unwrap();
    assert_eq!(error.certainty, sdk::Certainty::Uncertain);
    assert_eq!(
        error.operation.unwrap().operation_id.as_str(),
        "sdk-admin-uncertain"
    );
    let recovered = admin
        .restore(saved("sdk-admin-uncertain"))
        .unwrap()
        .wait(options())
        .await
        .unwrap();
    assert_eq!(recovered.value["alias"], "sdk-admin-uncertain-target");
    release.notify_one();
    proxy.await.unwrap();
    // A missing unscoped record has no original source to authorize. Its
    // refusal is not a fabricated unknown/absent observation; independent
    // evidence below proves that this fixture's queued work never wrote.
    assert!(
        matches!(admin.restore(saved("sdk-admin-never-written")).unwrap().status(options()).await,Err(e) if e.kind==sdk::ErrorKind::Authentication && e.certainty==sdk::Certainty::Refused)
    );
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let count:i64=db.query_row("SELECT COUNT(*) FROM operations WHERE operation_id IN ('sdk-admin-uncertain','sdk-admin-never-written')",[],|r|r.get(0)).unwrap();
    assert_eq!(count, 1);
    drop(db);
    admin.close(options()).await.unwrap();
    f.server.shutdown().await.unwrap();
    eprintln!("SDK administrative uncertainty {algorithm}: actual backend commit with held HTTP response, cancelled local waits retain original key, live independent signed recovery, one receipt and zero queued effect/no hidden resubmission, joined cleanup");
}

#[test]
fn local_administrative_validator_captures_original_command_and_uses_shared_reference_parser() {
    let value = serde_json::to_value(identity("local-original", "local-validator")).unwrap();
    let checked =
        sdk::helper::administrative::validate_bytes(&serde_json::to_vec(&value).unwrap(), false);
    assert_eq!(
        checked["profile"],
        sdk::helper::administrative::VALIDATION_PROFILE
    );
    assert_eq!(checked["valid"], true);
    assert_eq!(checked["value"], value);
    let reference = checked["reference"].clone();
    assert_eq!(
        reference,
        json!({"schema":"connect-operation-reference/1","operation_id":"local-original","command":"identity.provision","scope":{}})
    );
    let restored =
        sdk::helper::administrative::validate_bytes(&serde_json::to_vec(&reference).unwrap(), true);
    assert_eq!(restored["valid"], true);
    assert_eq!(restored["kind"], "administrative");
    assert_eq!(restored["reference"], reference);
    for (command, kind) in [
        ("operation.cancel", "operation_cancel"),
        ("operation.reconcile", "operation_reconcile"),
    ] {
        let value = json!({"schema":"connect-operation-reference/1","operation_id":"decision","command":command,"scope":{}});
        let restored =
            sdk::helper::administrative::validate_bytes(&serde_json::to_vec(&value).unwrap(), true);
        assert_eq!(restored["valid"], true);
        assert_eq!(restored["kind"], kind);
    }
}
#[test]
fn local_administrative_validator_refuses_bad_commands_ids_scopes_and_resource_overflow() {
    for bytes in [
        vec![255],
        vec![b' '; 65_537],
        b"null".to_vec(),
        b"{}".to_vec(),
        serde_json::to_vec(
            &json!({"type":"identity","operation_id":"bad/id","alias":"local","kind":"human"}),
        )
        .unwrap(),
    ] {
        assert_eq!(
            sdk::helper::administrative::validate_bytes(&bytes, false),
            json!({"profile":sdk::helper::administrative::VALIDATION_PROFILE,"valid":false})
        );
    }
    let original = json!({"schema":"connect-operation-reference/1","operation_id":"local-original","command":"identity.provision","scope":{}});
    for value in [
        json!({"schema":"other","operation_id":"local-original","command":"identity.provision","scope":{}}),
        json!({"schema":"connect-operation-reference/1","operation_id":"local-original","command":"message.send","scope":{}}),
        json!({"schema":"connect-operation-reference/1","operation_id":"local-original","command":"identity.provision","scope":{"conversation_id":"room"}}),
        json!({"schema":"connect-operation-reference/1","operation_id":"local-original","command":"identity.provision","scope":{},"extra":true}),
    ] {
        assert_eq!(
            sdk::helper::administrative::validate_bytes(&serde_json::to_vec(&value).unwrap(), true)
                ["valid"],
            false
        );
    }
    let mut large = original;
    large["extra"] = json!("x".repeat(4096));
    assert_eq!(
        sdk::helper::administrative::validate_bytes(&serde_json::to_vec(&large).unwrap(), true)
            ["valid"],
        false
    );
}
#[test]
fn local_administrative_validator_keeps_participant_and_trusted_helper_actions_separate() {
    let request = json!({"timeout_ms":100,"action":{"type":"administrative_provision","command":serde_json::to_value(identity("local-original","local-validator")).unwrap(),"mutation":{"operation_id":"local-original"}}});
    assert!(serde_json::from_value::<sdk::helper::Request>(request.clone()).is_err());
    assert!(serde_json::from_value::<sdk::helper::administrative::Request>(request).is_ok());
    let participant =
        json!({"timeout_ms":100,"action":{"type":"conversation_refresh","conversation_id":"room"}});
    assert!(serde_json::from_value::<sdk::helper::Request>(participant.clone()).is_ok());
    assert!(serde_json::from_value::<sdk::helper::administrative::Request>(participant).is_err());
}

#[tokio::test]
#[ignore = "Requires installed Node administrative consumer and explicit CONNECT_GATE6_NODE_ADMIN=1"]
async fn installed_node_eddsa_administration_covers_all_commands_and_original_recovery() {
    assert!(node::enabled());
    administration_case("EdDSA").await;
    node::witness("EdDSA");
}
#[tokio::test]
#[ignore = "Requires installed Node administrative consumer and explicit CONNECT_GATE6_NODE_ADMIN=1"]
async fn installed_node_es256_administration_covers_all_commands_and_original_recovery() {
    assert!(node::enabled());
    administration_case("ES256").await;
    node::witness("ES256");
}

#[tokio::test]
#[ignore = "Requires installed Node administrative consumer and explicit CONNECT_GATE6_NODE_ADMIN=1"]
async fn installed_node_eddsa_administration_enforces_current_authority_and_original_recovery() {
    assert!(node::enabled());
    authority_case("EdDSA").await;
    node::authority_witness("EdDSA");
}
#[tokio::test]
#[ignore = "Requires installed Node administrative consumer and explicit CONNECT_GATE6_NODE_ADMIN=1"]
async fn installed_node_es256_administration_enforces_current_authority_and_original_recovery() {
    assert!(node::enabled());
    authority_case("ES256").await;
    node::authority_witness("ES256");
}

#[tokio::test]
#[ignore = "Requires installed Node administrative consumer and explicit CONNECT_GATE6_NODE_ADMIN=1"]
async fn installed_node_eddsa_administration_recovers_held_committed_receipt_without_replay() {
    assert!(node::enabled());
    uncertainty_case("EdDSA").await;
    node::uncertainty_witness("EdDSA");
}
#[tokio::test]
#[ignore = "Requires installed Node administrative consumer and explicit CONNECT_GATE6_NODE_ADMIN=1"]
async fn installed_node_es256_administration_recovers_held_committed_receipt_without_replay() {
    assert!(node::enabled());
    uncertainty_case("ES256").await;
    node::uncertainty_witness("ES256");
}

#[tokio::test]
#[ignore = "Requires installed Python administrative consumer and explicit CONNECT_GATE6_PYTHON_ADMIN=1"]
async fn installed_python_eddsa_administration_covers_all_commands_and_original_recovery() {
    assert!(node::enabled() && node::python());
    administration_case("EdDSA").await;
    node::witness("EdDSA");
}

#[tokio::test]
#[ignore = "Requires installed Python administrative consumer and explicit CONNECT_GATE6_PYTHON_ADMIN=1"]
async fn installed_python_eddsa_administration_enforces_current_authority_and_original_recovery() {
    assert!(node::enabled() && node::python());
    authority_case("EdDSA").await;
    node::authority_witness("EdDSA");
}

#[tokio::test]
#[ignore = "Requires installed Python administrative consumer and explicit CONNECT_GATE6_PYTHON_ADMIN=1"]
async fn installed_python_eddsa_administration_recovers_held_committed_receipt_without_replay() {
    assert!(node::enabled() && node::python());
    uncertainty_case("EdDSA").await;
    node::uncertainty_witness("EdDSA");
}

#[tokio::test]
#[ignore = "Requires installed Python administrative consumer and explicit CONNECT_GATE6_PYTHON_ADMIN=1"]
async fn installed_python_es256_administration_covers_all_commands_and_original_recovery() {
    assert!(node::enabled() && node::python());
    administration_case("ES256").await;
    node::witness("ES256");
}

#[tokio::test]
#[ignore = "Requires installed Python administrative consumer and explicit CONNECT_GATE6_PYTHON_ADMIN=1"]
async fn installed_python_es256_administration_enforces_current_authority_and_original_recovery() {
    assert!(node::enabled() && node::python());
    authority_case("ES256").await;
    node::authority_witness("ES256");
}

#[tokio::test]
#[ignore = "Requires installed Python administrative consumer and explicit CONNECT_GATE6_PYTHON_ADMIN=1"]
async fn installed_python_es256_administration_recovers_held_committed_receipt_without_replay() {
    assert!(node::enabled() && node::python());
    uncertainty_case("ES256").await;
    node::uncertainty_witness("ES256");
}
