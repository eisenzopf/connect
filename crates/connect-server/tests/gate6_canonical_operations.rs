//! Actual canonical TLS/QUIC, HTTP and current public SDK share one Store ledger.
//! Session/connection rows below are trusted control fixtures, not media qualification.
#![allow(clippy::unwrap_used)]
#[path = "support/agent_mcp_fixture.rs"]
mod agent;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use connect_client::{ClientConfig, ConnectClient};
use connect_protocol::{provisioning::*, transitions::*, Operation, Outcome, Response};
use connect_service::{CanonicalHandler, StoreKeyResolver, StorePeerAuthority, StoreValidator};
use connect_store::{EvidenceSource, Scope as StoreScope};
use rvoip_auth_core::sig9421::*;
use rvoip_uctp::v2::{
    auth_dispatch::AuthenticationMethods,
    auth_peer::PeerConfig,
    server::{CanonicalProfile, ProfileServer},
    Envelope, RAW_QUIC_ALPN,
};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    time::timeout,
};
fn env(kind: &str, payload: Value, decision: Option<&str>) -> Value {
    let mut value = json!({"v":2,"type":kind,"id":format!("env_{}",uuid::Uuid::new_v4().simple()),"ts":chrono::Utc::now(),"payload":payload});
    if let Some(decision) = decision {
        value["operation_id"] = json!(decision);
    }
    value
}
fn key(id: &str) -> (Sig9421Signer, Vec<u8>) {
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).unwrap();
    (
        Sig9421Signer::ed25519(id, &key.serialize_der()).unwrap(),
        key.public_key_raw().to_vec(),
    )
}
struct ServerKey(Vec<u8>);
impl KeyResolver for ServerKey {
    fn resolve(&self, id: &str) -> Option<Vec<u8>> {
        (id == "canonical-server").then(|| self.0.clone())
    }
}
struct Wire {
    host: quinn::Endpoint,
    client: quinn::Endpoint,
    connection: quinn::Connection,
    server: ProfileServer,
    writer: quinn::SendStream,
    reader: quinn::RecvStream,
    signer: Sig9421Signer,
    verify: Sig9421Verifier,
    completions: std::collections::HashMap<String, (Value, usize)>,
}
impl Wire {
    async fn open(f: &agent::Fixture, config: &ClientConfig) -> Self {
        use rvoip_uctp::substrate::{
            dev_client_config_trusting, make_server_endpoint, self_signed_for_dev,
        };
        let id = format!("device_{}", uuid::Uuid::new_v4().simple());
        let kid = format!("key_{}", uuid::Uuid::new_v4().simple());
        let (signer, public) = key(&kid);
        f.server
            .store
            .execute_provisioning(
                &f.admin().credential_id,
                ProvisioningCommand::Endpoint {
                    operation_id: format!("enroll-{id}"),
                    identity_id: config.identity_id.clone(),
                    device: DeviceSpec {
                        id: id.clone(),
                        kind: "desktop".into(),
                        platform: "fixture".into(),
                    },
                    transport: EndpointTransport::Quic,
                    address: format!("quic://localhost:4444/{id}"),
                    capabilities: vec!["data".into()],
                    secret_ref: None,
                },
            )
            .unwrap();
        f.server
            .store
            .execute_provisioning(
                &f.admin().credential_id,
                ProvisioningCommand::SigningKey {
                    operation_id: format!("enroll-{kid}"),
                    identity_id: config.identity_id.clone(),
                    credential_id: config.credential_id.clone(),
                    device_id: id.clone(),
                    key_id: kid,
                    algorithm: SigningAlgorithm::EdDSA,
                    public_key: URL_SAFE_NO_PAD.encode(public),
                    not_before: chrono::Utc::now().timestamp() - 1,
                    expires_at: chrono::Utc::now().timestamp() + 300,
                    expected_revision: None,
                },
            )
            .unwrap();
        let (server_signer, server_public) = key("canonical-server");
        let peer = Arc::new(PeerConfig {
            methods: Arc::new(AuthenticationMethods::new(Some(StoreValidator::canonical(
                f.server.store.clone(),
            )))),
            verifier: Arc::new(Sig9421Verifier::new(StoreKeyResolver::new(
                f.server.store.clone(),
            ))),
            signer: Arc::new(server_signer),
            authority: StorePeerAuthority::new(f.server.store.clone(), 8, None)
                .await
                .unwrap(),
            pending_capacity: 8,
        });
        let profile =
            CanonicalProfile::new(peer, CanonicalHandler::new(f.server.service.clone()), 8)
                .unwrap()
                .with_physical_handlers(CanonicalHandler::physical_factory(
                    f.server.service.clone(),
                ));
        let (cert, private) = self_signed_for_dev(&["localhost".into()]).unwrap();
        let mut tls = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert.clone()], private)
            .unwrap();
        tls.alpn_protocols = vec![RAW_QUIC_ALPN.to_vec()];
        let host = make_server_endpoint(
            "127.0.0.1:0".parse().unwrap(),
            Arc::new(tls),
            quinn::TransportConfig::default(),
        )
        .unwrap();
        let mut tls = dev_client_config_trusting(&cert).unwrap();
        tls.alpn_protocols = vec![RAW_QUIC_ALPN.to_vec()];
        let mut client = quinn::Endpoint::client("127.0.0.1:0".parse().unwrap()).unwrap();
        client.set_default_client_config(quinn::ClientConfig::new(Arc::new(
            quinn::crypto::rustls::QuicClientConfig::try_from(tls).unwrap(),
        )));
        let connecting = client
            .connect(host.local_addr().unwrap(), "localhost")
            .unwrap();
        let accepted = async { host.accept().await.unwrap().await.unwrap() };
        let (connection, accepted) = tokio::join!(connecting, accepted);
        let connection = connection.unwrap();
        let (send, receive) = tokio::sync::mpsc::channel(8);
        let server = rvoip_quic::canonical::start(
            receive,
            profile,
            format!("quic://localhost:{}", host.local_addr().unwrap().port()),
        )
        .unwrap();
        send.send(accepted).await.unwrap();
        let (writer, reader) = connection.open_bi().await.unwrap();
        let mut wire = Self {
            host,
            client,
            connection,
            server,
            writer,
            reader,
            signer,
            verify: Sig9421Verifier::new(Arc::new(ServerKey(server_public))),
            completions: Default::default(),
        };
        let hello = env(
            "auth.hello",
            json!({"auth_methods":["bearer"],"application_profiles":["connect/2"],"device":{"id":id,"kind":"desktop","platform":"fixture","sdk_version":"fixture"}}),
            None,
        );
        wire.send(&hello, false).await;
        let challenge = wire.read().await;
        assert_eq!(challenge.kind(), "auth.challenge");
        let mut response = env(
            "auth.response",
            json!({"method":"bearer","credential":config.credential}),
            None,
        );
        response["in_reply_to"] = json!(challenge.id());
        wire.send(&response, true).await;
        assert_eq!(wire.read().await.kind(), "auth.session");
        wire
    }
    async fn send(&mut self, value: &Value, signed: bool) {
        if matches!(
            value["type"].as_str(),
            Some(
                "conversation.create"
                    | "conversation.close"
                    | "conversation.get"
                    | "operation.get"
                    | "operation.cancel"
                    | "operation.reconcile"
                    | "conversation.participant.add"
                    | "conversation.participant.update"
                    | "conversation.participant.remove"
            )
        ) {
            self.completions
                .insert(value["id"].as_str().unwrap().into(), (value.clone(), 0));
        }
        let value = if signed {
            self.signer.sign(value).unwrap()
        } else {
            value.clone()
        };
        let bytes = serde_json::to_vec(&value).unwrap();
        self.writer.write_u32(bytes.len() as u32).await.unwrap();
        self.writer.write_all(&bytes).await.unwrap();
    }
    // Existing semantic cases inspect facts. Validate and drain the newly
    // explicit completion marker; dedicated SDK cases consume the whole batch.
    async fn read(&mut self) -> Envelope {
        loop {
            let length = timeout(Duration::from_secs(5), self.reader.read_u32())
                .await
                .unwrap()
                .unwrap();
            assert!(length < 1024 * 1024);
            let mut bytes = vec![0; length as usize];
            self.reader.read_exact(&mut bytes).await.unwrap();
            let envelope = Envelope::parse(&bytes).unwrap();
            self.verify.verify(envelope.original()).await.unwrap();
            if let Some(id) = envelope.original()["in_reply_to"].as_str() {
                if envelope.kind() == "ack"
                    && envelope.original()["payload"]["details"]["reply_complete"] == true
                {
                    let (request, count) = self
                        .completions
                        .remove(id)
                        .expect("completion for a known command");
                    assert!(count > 0);
                    for field in ["cid", "sid", "connid", "operation_id"] {
                        assert_eq!(envelope.original().get(field), request.get(field));
                    }
                    continue;
                }
                if envelope.kind() == "error" {
                    self.completions.remove(id);
                } else if let Some((_, count)) = self.completions.get_mut(id) {
                    *count += 1;
                }
            }
            return envelope;
        }
    }
    async fn exchange(&mut self, value: Value) -> Envelope {
        self.send(&value, true).await;
        let reply = self.read().await;
        assert_eq!(
            reply.original()["in_reply_to"],
            value["id"],
            "reply kind {} on {}",
            reply.kind(),
            value["type"]
        );
        for field in ["cid", "sid", "connid", "operation_id"] {
            assert_eq!(reply.original().get(field), value.get(field));
        }
        reply
    }
    async fn close(self) {
        self.connection.close(0u32.into(), b"fixture-complete");
        self.server.shutdown().await.unwrap();
        self.host.close(0u32.into(), b"fixture-complete");
        self.client.close(0u32.into(), b"fixture-complete");
    }
}
async fn transition(
    f: &agent::Fixture,
    client: &mut ConnectClient,
    config: &ClientConfig,
    name: &str,
) -> TransitionRequest {
    let cid = client
        .create(&format!("room-{name}"), vec![f.peer.identity_id.clone()])
        .await
        .unwrap()
        .id;
    let sid = client
        .session_invite(
            &format!("invite-{name}"),
            &cid,
            &f.peer.identity_id,
            vec!["audio".into()],
        )
        .await
        .unwrap()
        .id;
    let mut peer = f.peer_client().await;
    peer.session_accept(&format!("accept-{name}"), &cid, &sid)
        .await
        .unwrap();
    drop(peer);
    let snapshot = f
        .server
        .store
        .membership_snapshot(&config.credential_id, &cid)
        .unwrap();
    let config_identity = snapshot
        .participants
        .iter()
        .find(|p| p.identity_id == f.agent.identity_id)
        .unwrap()
        .participant_id
        .clone();
    let c = rusqlite::Connection::open(f.server.store.path()).unwrap();
    c.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    for (n, identity) in [&f.agent.identity_id, &f.peer.identity_id]
        .into_iter()
        .enumerate()
    {
        let pid = &snapshot
            .participants
            .iter()
            .find(|p| &p.identity_id == identity)
            .unwrap()
            .participant_id;
        let dev = format!("{name}-media-device-{n}");
        let endpoint = format!("{name}-media-endpoint-{n}");
        let connection = format!("{name}-media-conn-{n}");
        c.execute("INSERT INTO devices(id,identity_id,tenant_id,kind,platform) VALUES(?1,?2,'tnt_default','fixture','fixture')",rusqlite::params![dev,identity]).unwrap();
        c.execute("INSERT INTO endpoints(id,identity_id,tenant_id,device_id,transport,address,capabilities_json,verification_state,verification_evidence,verified_at) VALUES(?1,?2,'tnt_default',?3,'webrtc',?4,'[\"audio\"]','verified','control-fixture',?5)",rusqlite::params![endpoint,identity,dev,format!("https://fixture.invalid/{name}/{n}"),chrono::Utc::now().timestamp()]).unwrap();
        c.execute("INSERT INTO session_participants(session_id,conversation_id,tenant_id,participant_id,membership_epoch,role,state) VALUES(?1,?2,'tnt_default',?3,1,'participant','present') ON CONFLICT(session_id,participant_id,membership_epoch) DO UPDATE SET role=excluded.role,state=excluded.state",rusqlite::params![sid,cid,pid]).unwrap();
        c.execute("INSERT INTO connections(id,session_id,participant_id,membership_epoch,identity_id,tenant_id,endpoint_id,generation,state,observed_runtime_id) VALUES(?1,?2,?3,1,?4,'tnt_default',?5,1,'active',?6)",rusqlite::params![connection,sid,pid,identity,endpoint,f.server.store.runtime_epoch()]).unwrap();
    }
    TransitionRequest {
        payload_extensions: Default::default(),
        operation_id: name.into(),
        conversation_id: cid,
        session_id: sid,
        connection_id: None,
        expected_revision: 0,
        expires_at: chrono::Utc::now().timestamp() + 300,
        intent: TransitionIntent::Consult {
            retained_connid: format!("{name}-media-conn-1"),
            private_participants: vec![config_identity],
            hold_treatment: HoldTreatment::Silence,
        },
    }
}
async fn http(
    f: &agent::Fixture,
    config: &ClientConfig,
    action: OperationAction,
    sample: Option<&str>,
) -> connect_protocol::Result<OperationRecord> {
    let request = Operation::DurableOperation { request: action };
    let response = tokio::task::spawn_blocking(reqwest::Client::new)
        .await
        .unwrap()
        .post(format!(
            "http://{}/v1/operations",
            f.server.http_address.unwrap()
        ))
        .bearer_auth(&config.credential)
        .json(&request)
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    let value: Value = response.json().await.unwrap();
    let expected = match value["result"]["Err"]["code"].as_str() {
        Some("unauthorized") => 401,
        Some("forbidden") => 403,
        Some("authority-busy" | "authority-unavailable" | "storage-unavailable") => 503,
        _ => 200,
    };
    assert_eq!(status, expected, "actual HTTP refusal class");
    if let (Some(directory), Some(sample)) =
        (std::env::var_os("CONNECT_GATE6_HTTP_SAMPLES"), sample)
    {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join(sample),serde_json::to_vec_pretty(&json!({"api":"operations","route":"/v1/operations","status":status,"request":request,"response":value})).unwrap()).unwrap();
    }
    let response: Response = serde_json::from_value(value).unwrap();
    response.result.and_then(|outcome| match outcome {
        Outcome::DurableOperation(record) => Ok(record),
        _ => panic!("wrong durable operation response"),
    })
}
fn compare(reply: &Envelope, record: &OperationRecord) {
    assert_eq!(reply.kind(), "operation.updated");
    let payload = &reply.original()["payload"];
    assert_eq!(payload["operation_id"], record.operation_id);
    assert_eq!(payload["state"], json!(record.state));
    assert_eq!(payload["revision"], record.revision);
    assert_eq!(payload.get("result"), record.result.as_ref());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_canonical_quic_operation_get_cancel_reconcile_share_http_and_sdk_decision_ids() {
    let f = agent::Fixture::start().await;
    let config = f.issue_as_operator(&f.agent, StoreScope::operator());
    let mut client = ConnectClient::open(config.clone()).await.unwrap();
    let request = transition(&f, &mut client, &config, "canonical-transition").await;
    let accepted = client.admit_transition(request.clone()).await.unwrap();
    let mut wire = Wire::open(&f, &config).await;
    let mut get = env(
        "operation.get",
        json!({"operation_id":request.operation_id}),
        None,
    );
    get["cid"] = json!(request.conversation_id);
    get["sid"] = json!(request.session_id);
    let reply = wire.exchange(get).await;
    compare(&reply, &accepted);
    compare(
        &reply,
        &http(
            &f,
            &config,
            OperationAction::Get {
                operation_id: request.operation_id.clone(),
            },
            Some("canonical-op-get.json"),
        )
        .await
        .unwrap(),
    );
    let cancel = OperationAction::Cancel {
        operation_id: "canonical-cancel".into(),
        target_operation_id: request.operation_id.clone(),
        reason: Some("stop before execution".into()),
    };
    let mut command = env(
        "operation.cancel",
        json!({"operation_id":request.operation_id,"reason":"stop before execution"}),
        Some("canonical-cancel"),
    );
    command["cid"] = json!(request.conversation_id);
    command["sid"] = json!(request.session_id);
    let reply = wire.exchange(command.clone()).await;
    let receipt = http(
        &f,
        &config,
        cancel.clone(),
        Some("canonical-op-cancel.json"),
    )
    .await
    .unwrap();
    compare(&reply, &receipt);
    assert_eq!(client.operation_action(cancel).await.unwrap(), receipt);
    command["id"] = json!(format!("env_{}", uuid::Uuid::new_v4().simple()));
    assert_eq!(
        wire.exchange(command).await.original()["payload"],
        reply.original()["payload"]
    );
    assert_eq!(
        client
            .operation_action(OperationAction::Get {
                operation_id: request.operation_id.clone()
            })
            .await
            .unwrap()
            .state,
        OperationState::Cancelled
    );
    let evidence = f
        .server
        .store
        .record_transition_evidence(
            &config.credential_id,
            &request.operation_id,
            "canonical-cancelled-fact",
            EvidenceSource::OperatorAttestation,
            ReconcileOutcome::Cancelled,
            None,
        )
        .unwrap();
    let reconcile = OperationAction::Reconcile {
        operation_id: "canonical-reconcile".into(),
        target_operation_id: request.operation_id.clone(),
        outcome: ReconcileOutcome::Cancelled,
        evidence_id: evidence.clone(),
    };
    let command = env(
        "operation.reconcile",
        json!({"operation_id":request.operation_id,"outcome":"cancelled","evidence_id":evidence}),
        Some("canonical-reconcile"),
    );
    let reply = wire.exchange(command).await;
    let receipt = http(
        &f,
        &config,
        reconcile.clone(),
        Some("canonical-op-reconcile.json"),
    )
    .await
    .unwrap();
    compare(&reply, &receipt);
    assert_eq!(client.operation_action(reconcile).await.unwrap(), receipt);
    let before = connect_store::Store::inspect_snapshot(f.server.store.path()).unwrap();
    let mut spoof = env(
        "operation.get",
        json!({"operation_id":request.operation_id,"actor_id":f.peer.identity_id}),
        None,
    );
    spoof["sid"] = json!("foreign-session");
    let denied = wire.exchange(spoof).await;
    assert_eq!(denied.kind(), "error");
    assert_eq!(denied.original()["payload"]["code"], 403);
    assert_eq!(
        connect_store::Store::inspect_snapshot(f.server.store.path()).unwrap(),
        before
    );
    wire.close().await;
    drop(client);
    f.server.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_canonical_quic_operation_denials_and_reconnect_preserve_current_ledger_authority() {
    let f = agent::Fixture::start().await;
    let config = f.issue_as_operator(&f.agent, StoreScope::operator());
    let mut client = ConnectClient::open(config.clone()).await.unwrap();
    let request = transition(&f, &mut client, &config, "reconnect-transition").await;
    client.admit_transition(request.clone()).await.unwrap();
    let mut wire = Wire::open(&f, &config).await;
    let command = env(
        "operation.cancel",
        json!({"operation_id":request.operation_id}),
        Some("reconnect-cancel"),
    );
    let first = wire.exchange(command).await;
    wire.close().await;
    let mut wire = Wire::open(&f, &config).await;
    let command = env(
        "operation.cancel",
        json!({"operation_id":request.operation_id}),
        Some("reconnect-cancel"),
    );
    assert_eq!(
        wire.exchange(command).await.original()["payload"],
        first.original()["payload"]
    );
    let conflict = env(
        "operation.cancel",
        json!({"operation_id":request.operation_id,"reason":"different"}),
        Some("reconnect-cancel"),
    );
    assert_eq!(
        wire.exchange(conflict).await.original()["payload"]["code"],
        409
    );
    let invalid = env(
        "operation.reconcile",
        json!({"operation_id":request.operation_id,"outcome":"cancelled","evidence_id":"unattributed"}),
        Some("unattributed-decision"),
    );
    assert_eq!(
        wire.exchange(invalid).await.original()["payload"]["code"],
        403
    );
    let foreign = f.issue_as_operator(&f.foreign, StoreScope::operator());
    assert_eq!(
        http(
            &f,
            &foreign,
            OperationAction::Get {
                operation_id: request.operation_id.clone()
            },
            Some("canonical-op-foreign.json")
        )
        .await
        .unwrap_err()
        .code,
        "forbidden"
    );
    let mut foreign_wire = Wire::open(&f, &foreign).await;
    let before = connect_store::Store::inspect_snapshot(f.server.store.path()).unwrap();
    let get = env(
        "operation.get",
        json!({"operation_id":request.operation_id}),
        None,
    );
    assert_eq!(
        foreign_wire.exchange(get).await.original()["payload"]["code"],
        403
    );
    assert_eq!(
        connect_store::Store::inspect_snapshot(f.server.store.path()).unwrap(),
        before
    );
    foreign_wire.close().await;
    wire.close().await;
    drop(client);
    f.server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_canonical_quic_lazy_operation_recovery_preserves_unknown_effect_across_reconnect_without_resubmission(
) {
    let f = agent::Fixture::start().await;
    let config = f.issue_as_operator(&f.agent, StoreScope::operator());
    let mut client = ConnectClient::open(config.clone()).await.unwrap();
    let request = transition(&f, &mut client, &config, "unknown-transition").await;
    client.admit_transition(request.clone()).await.unwrap();
    let (lease, _) = f
        .server
        .store
        .claim_transition(&config.credential_id, &request.operation_id, 60)
        .unwrap()
        .unwrap();
    // Trusted control fixture marks one possible submission. No provider or
    // physical media operation is performed by this test.
    f.server
        .store
        .prepare_transition_effect(
            &config.credential_id,
            &lease,
            "possible-effect",
            "fixture",
            "possible-effect",
            IdempotencyMode::None,
            None,
        )
        .unwrap();
    f.server
        .store
        .begin_transition_effect(&config.credential_id, &lease, "possible-effect")
        .unwrap();
    let mut wire = Wire::open(&f, &config).await;
    let c = rusqlite::Connection::open(f.server.store.path()).unwrap();
    c.execute(
        "UPDATE operations SET lease_expires_at=0 WHERE operation_id=?1",
        [&request.operation_id],
    )
    .unwrap();
    assert_eq!(
        c.query_row(
            "SELECT state FROM operations WHERE operation_id=?1",
            [&request.operation_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "preparing"
    );
    drop(c);
    let get = env(
        "operation.get",
        json!({"operation_id":request.operation_id}),
        None,
    );
    let first = wire.exchange(get).await;
    let record = http(
        &f,
        &config,
        OperationAction::Get {
            operation_id: request.operation_id.clone(),
        },
        Some("canonical-op-unknown.json"),
    )
    .await
    .unwrap();
    compare(&first, &record);
    assert_eq!(record.state, OperationState::Unknown);
    assert_eq!(record.effects.len(), 1);
    assert_eq!(record.effects[0].attempt, 1);
    assert_eq!(record.effects[0].state, EffectState::Unknown);
    wire.close().await;
    let mut wire = Wire::open(&f, &config).await;
    assert_eq!(
        wire.exchange(env(
            "operation.get",
            json!({"operation_id":request.operation_id}),
            None
        ))
        .await
        .original()["payload"],
        first.original()["payload"]
    );
    assert_eq!(
        client
            .operation_action(OperationAction::Get {
                operation_id: request.operation_id.clone()
            })
            .await
            .unwrap(),
        record
    );
    assert!(f
        .server
        .store
        .claim_transition(&config.credential_id, &request.operation_id, 60)
        .unwrap()
        .is_none());
    wire.close().await;
    drop(client);
    f.server.shutdown().await.unwrap();
}

struct RestoreOperationsTable(Option<rusqlite::Connection>);
impl Drop for RestoreOperationsTable {
    fn drop(&mut self) {
        if let Some(connection) = self.0.take() {
            let _ = connection.execute_batch(
                "ALTER TABLE deliberately_unavailable_operations RENAME TO operations",
            );
        }
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_canonical_quic_policy_busy_and_storage_refusals_are_retryable_without_decision_mutation(
) {
    use connect_store::AuthorityWait;
    let f = agent::Fixture::start().await;
    let config = f.issue_as_operator(&f.agent, StoreScope::operator());
    let mut client = ConnectClient::open(config.clone()).await.unwrap();
    let request = transition(&f, &mut client, &config, "busy-transition").await;
    client.admit_transition(request.clone()).await.unwrap();
    let mut wire = Wire::open(&f, &config).await;
    let read = f
        .server
        .store
        .read_authority(
            &config.credential_id,
            Some(&request.conversation_id),
            &AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    let before = connect_store::Store::inspect_snapshot(f.server.store.path()).unwrap();
    let command = env(
        "operation.cancel",
        json!({"operation_id":request.operation_id}),
        Some("busy-cancel"),
    );
    let refusal = wire.exchange(command).await;
    assert_eq!(refusal.kind(), "error");
    assert_eq!(
        refusal.original()["payload"],
        json!({"code":503,"category":"authorization","reason":"authority_unavailable"})
    );
    let cancel = OperationAction::Cancel {
        operation_id: "busy-cancel".into(),
        target_operation_id: request.operation_id.clone(),
        reason: None,
    };
    assert_eq!(
        http(&f, &config, cancel.clone(), Some("canonical-op-busy.json"))
            .await
            .unwrap_err()
            .code,
        "authority-busy"
    );
    assert_eq!(
        connect_store::Store::inspect_snapshot(f.server.store.path()).unwrap(),
        before
    );
    drop(read);
    let command = env(
        "operation.cancel",
        json!({"operation_id":request.operation_id}),
        Some("busy-cancel"),
    );
    let accepted = wire.exchange(command).await;
    compare(&accepted, &client.operation_action(cancel).await.unwrap());
    let connection = rusqlite::Connection::open(f.server.store.path()).unwrap();
    let count: i64 = connection
        .query_row("SELECT COUNT(*) FROM operations", [], |r| r.get(0))
        .unwrap();
    // Explicit SQL fault injection affects operation lookup, not the binding
    // watch's key/credential facts. Restore before checking application state.
    connection
        .execute_batch("ALTER TABLE operations RENAME TO deliberately_unavailable_operations")
        .unwrap();
    let restore = RestoreOperationsTable(Some(connection));
    let unavailable = wire
        .exchange(env(
            "operation.get",
            json!({"operation_id":request.operation_id}),
            None,
        ))
        .await;
    assert_eq!(
        http(
            &f,
            &config,
            OperationAction::Get {
                operation_id: request.operation_id.clone()
            },
            Some("canonical-message-storage.json")
        )
        .await
        .unwrap_err()
        .code,
        "storage-unavailable"
    );
    drop(restore);
    assert_eq!(
        unavailable.original()["payload"],
        json!({"code":503,"category":"storage","reason":"storage_unavailable"})
    );
    assert_eq!(
        rusqlite::Connection::open(f.server.store.path())
            .unwrap()
            .query_row("SELECT COUNT(*) FROM operations", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        count
    );
    compare(
        &wire
            .exchange(env(
                "operation.get",
                json!({"operation_id":request.operation_id}),
                None,
            ))
            .await,
        &client
            .operation_action(OperationAction::Get {
                operation_id: request.operation_id.clone(),
            })
            .await
            .unwrap(),
    );
    wire.close().await;
    drop(client);
    f.server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_canonical_message_unread_ack_reconnect_and_rotation_recover_one_shared_receipt() {
    use connect_protocol::messages::MessageDraft;
    let f = agent::Fixture::start().await;
    let config = f.agent.clone(); // Named ordinary communication scopes, including operation.get.
    let mut client = ConnectClient::open(config.clone()).await.unwrap();
    let cid = client
        .create("message-recovery-room", vec![f.peer.identity_id.clone()])
        .await
        .unwrap()
        .id;
    let mut wire = Wire::open(&f, &config).await;
    let id = "canonical-message-recovery";
    let command = |body: &str| {
        let mut value = env(
            "message.send",
            json!({"msg_id":"canonical_recovered_message","to":"all","content_type":"text/plain","body":body,"body_encoding":"utf8","label":"connect-messages"}),
            Some(id),
        );
        value["cid"] = json!(cid);
        value
    };
    // Do not consume the ack. Observe durable commitment through another real
    // facade, then end the physical connection before the caller sees a result.
    wire.send(&command("one durable message"), true).await;
    let get = OperationAction::Get {
        operation_id: id.into(),
    };
    let first = timeout(Duration::from_secs(10), async {
        loop {
            match http(&f, &config, get.clone(), None).await {
                Ok(record) => break record,
                Err(e) if matches!(e.code.as_str(), "forbidden" | "authority-busy") => {
                    tokio::time::sleep(Duration::from_millis(20)).await
                }
                Err(e) => panic!("unexpected recovery error {}", e.code),
            }
        }
    })
    .await
    .unwrap();
    wire.close().await;
    drop(client);
    assert_eq!(first.state, OperationState::Completed);
    let accepted: Outcome = serde_json::from_value(first.result.clone().unwrap()).unwrap();
    let Outcome::Accepted {
        receipt: Some(receipt),
        message_id,
        ..
    } = &accepted
    else {
        panic!("missing core recovery reference")
    };
    assert!(receipt.matches(id, &cid));
    let replacement = f.issue_as_operator(
        &f.agent,
        agent::scope(vec![
            f.agent.identity_id.clone(),
            f.peer.identity_id.clone(),
        ]),
    );
    f.revoke(&config.credential_id);
    let mut wire = Wire::open(&f, &replacement).await;
    let mut query = env("operation.get", json!({"operation_id":id}), None);
    query["cid"] = json!(cid);
    compare(&wire.exchange(query).await, &first);
    assert_eq!(
        http(
            &f,
            &replacement,
            get.clone(),
            Some("canonical-message-get.json")
        )
        .await
        .unwrap(),
        first
    );
    let mut client = ConnectClient::open(replacement.clone()).await.unwrap();
    assert_eq!(
        client
            .operation_action(receipt.status_reference.clone())
            .await
            .unwrap(),
        first
    );
    let mut draft = MessageDraft::text_all("one durable message");
    draft.msg_id = Some("canonical_recovered_message".into());
    let retry = client.send_message(id, &cid, draft.clone()).await.unwrap();
    assert_eq!(&retry.receipt, receipt);
    assert_eq!(&retry.message_id, message_id);
    let operation = Operation::Send {
        request_id: id.into(),
        conversation_id: cid.clone(),
        message: draft,
    };
    let response = tokio::task::spawn_blocking(reqwest::Client::new)
        .await
        .unwrap()
        .post(format!(
            "http://{}/v1/operations",
            f.server.http_address.unwrap()
        ))
        .bearer_auth(&replacement.credential)
        .json(&operation)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let response: Value = response.json().await.unwrap();
    assert_eq!(
        response["result"]["Ok"],
        serde_json::to_value(&accepted).unwrap()
    );
    if let Some(dir) = std::env::var_os("CONNECT_GATE6_HTTP_SAMPLES") {
        std::fs::write(std::path::PathBuf::from(dir).join("canonical-message-send.json"),serde_json::to_vec_pretty(&json!({"api":"messages","route":"/v1/operations","status":200,"request":operation,"response":response})).unwrap()).unwrap();
    }
    let ack = wire.exchange(command("one durable message")).await;
    assert_eq!(ack.kind(), "ack");
    assert_eq!(
        ack.original()["payload"]["details"],
        serde_json::to_value(&accepted).unwrap()
    );
    assert_eq!(
        wire.exchange(command("changed intent")).await.original()["payload"]["code"],
        409
    );
    for (field, bad) in [
        ("cid", "different_room"),
        ("sid", "different_session"),
        ("connid", "different_connection"),
    ] {
        let mut query = env("operation.get", json!({"operation_id":id}), None);
        query[field] = json!(bad);
        assert_eq!(
            wire.exchange(query).await.original()["payload"]["code"],
            403
        );
    }
    assert!(client
        .send_message(
            "canonical-rejected-message",
            &cid,
            MessageDraft::text_all("")
        )
        .await
        .is_err());
    let rejected = http(
        &f,
        &replacement,
        OperationAction::Get {
            operation_id: "canonical-rejected-message".into(),
        },
        Some("canonical-message-failed.json"),
    )
    .await
    .unwrap();
    assert_eq!(rejected.state, OperationState::Failed);
    assert!(rejected.result.is_none());
    compare(
        &wire
            .exchange(env(
                "operation.get",
                json!({"operation_id":"canonical-rejected-message"}),
                None,
            ))
            .await,
        &rejected,
    );
    let c = rusqlite::Connection::open(f.server.store.path()).unwrap();
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM operations WHERE operation_id=?1",
            [id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM messages WHERE id='canonical_recovered_message'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM journal WHERE message_id='canonical_recovered_message'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    wire.close().await;
    drop(client);
    f.server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_canonical_conversation_quic_http_native_sdk_recovery_and_idle_worker() {
    use connect_protocol::conversations::*;
    let f = agent::Fixture::start().await;
    let config = f.issue_as_operator(&f.agent, StoreScope::operator());
    let mut wire = Wire::open(&f, &config).await;
    let input = CreateRequest {
        tenant_id: Some("tnt_default".into()),
        policy: ConversationPolicy::Persistent,
        idle_close_secs: Some(9),
        metadata: Some(json!({"topic":"private-topic"})),
        initial_participants: vec![InitialParticipant {
            identity_id: f.peer.identity_id.clone(),
            role: "guest".into(),
        }],
    };
    let create = env(
        "conversation.create",
        serde_json::to_value(&input).unwrap(),
        Some("shared-conversation"),
    );
    wire.send(&create, true).await;
    let opened = wire.read().await;
    assert_eq!(
        opened.kind(),
        "conversation.opened",
        "{:?}",
        opened.original()
    );
    assert_eq!(opened.original()["in_reply_to"], create["id"]);
    let cid = opened.original()["cid"].as_str().unwrap().to_owned();
    assert_eq!(
        opened.original()["payload"]["metadata"],
        json!({"topic":"private-topic"})
    );
    assert_eq!(opened.original()["payload"]["idle_close_secs"], 9);
    let mut native = ConnectClient::open(config.clone()).await.unwrap();
    let from_native = native
        .create_conversation("shared-conversation", input.clone())
        .await
        .unwrap();
    assert_eq!(from_native.id, cid);
    assert_eq!(
        from_native
            .participants
            .iter()
            .find(|p| p.identity_id == f.peer.identity_id)
            .unwrap()
            .role,
        "guest"
    );
    async fn http_conversation(
        f: &agent::Fixture,
        config: &ClientConfig,
        op: &Operation,
        sample: &str,
    ) -> Outcome {
        let response = tokio::task::spawn_blocking(reqwest::Client::new)
            .await
            .unwrap()
            .post(format!(
                "http://{}/v1/operations",
                f.server.http_address.unwrap()
            ))
            .bearer_auth(&config.credential)
            .json(op)
            .send()
            .await
            .unwrap();
        let status = response.status().as_u16();
        assert_eq!(status, 200);
        let value: Value = response.json().await.unwrap();
        if let Some(dir) = std::env::var_os("CONNECT_GATE6_HTTP_SAMPLES") {
            let dir = std::path::PathBuf::from(dir);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(sample),serde_json::to_vec_pretty(&json!({"api":"conversations","route":"/v1/operations","status":status,"request":op,"response":value})).unwrap()).unwrap();
        }
        serde_json::from_value::<Response>(value)
            .unwrap()
            .result
            .unwrap()
    }
    let created = http_conversation(
        &f,
        &config,
        &Operation::Create {
            request_id: "shared-conversation".into(),
            request: input.clone(),
        },
        "conversation-create.json",
    )
    .await;
    assert_eq!(created, Outcome::Conversation(from_native.clone()));
    let query = ListQuery {
        filter: ListFilter {
            state: ConversationState::All,
            identity_id: Some(f.peer.identity_id.clone()),
            ..Default::default()
        },
        limit: Some(1),
        ..Default::default()
    };
    http_conversation(
        &f,
        &config,
        &Operation::Conversations {
            query: query.clone(),
        },
        "conversation-list.json",
    )
    .await;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let data = json!({"base":format!("http://{}",f.server.http_address.unwrap()),"token":config.credential,"request":input,"cid":cid,"peer":f.peer.identity_id});
    tokio::task::spawn_blocking(move || {
        use std::{
            io::Write,
            process::{Command, Stdio},
        };
        for (program, script) in [
            ("node", "sdk/tests/conversations_probe.mjs"),
            ("python3", "sdk/tests/conversations_probe.py"),
        ] {
            let mut child = Command::new(program)
                .current_dir(&root)
                .env("PYTHONPATH", root.join("sdk/python"))
                .arg(script)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(serde_json::to_string(&data).unwrap().as_bytes())
                .unwrap();
            let result = child.wait_with_output().unwrap();
            assert!(
                result.status.success(),
                "SDK probe failed: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert_eq!(
                serde_json::from_slice::<Value>(&result.stdout).unwrap(),
                json!({"passed":true})
            );
        }
    })
    .await
    .unwrap();
    let options = CloseOptions {
        expected_revision: Some(from_native.revision),
        reason_code: Some(12),
        reason: Some("done".into()),
    };
    let mut close = env(
        "conversation.close",
        json!({"reason_code":12,"reason":"done"}),
        Some("shared-close"),
    );
    close["cid"] = json!(cid);
    close["expected_revision"] = json!(from_native.revision);
    let closed = wire.exchange(close).await;
    assert_eq!(closed.kind(), "conversation.closed");
    let outcome = http_conversation(
        &f,
        &config,
        &Operation::Close {
            request_id: "shared-close".into(),
            conversation_id: cid.clone(),
            options: options.clone(),
        },
        "conversation-close.json",
    )
    .await;
    assert!(matches!(outcome,Outcome::Conversation(v) if v.closed&&v.reason_code==Some(12)));
    assert_eq!(
        native
            .close_conversation_with_options("shared-close", &cid, options)
            .await
            .unwrap()
            .reason_code,
        Some(12)
    );
    wire.close().await;
    let mut wire = Wire::open(&f, &config).await;
    let get = env(
        "operation.get",
        json!({"operation_id":"shared-conversation"}),
        None,
    );
    let recovered = wire.exchange(get).await;
    assert_eq!(recovered.original()["payload"]["result"]["data"]["id"], cid);
    assert_eq!(
        recovered.original()["payload"]["result"]["data"]["closed"],
        true
    );
    wire.close().await;
    let ephemeral = native
        .create_conversation(
            "idle-runtime",
            CreateRequest {
                tenant_id: None,
                policy: ConversationPolicy::Ephemeral,
                idle_close_secs: Some(1),
                metadata: None,
                initial_participants: vec![],
            },
        )
        .await
        .unwrap();
    timeout(Duration::from_secs(5), async {
        loop {
            let page = native
                .list_conversations(ListQuery {
                    filter: ListFilter {
                        state: ConversationState::Closed,
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .await
                .unwrap();
            if let Some(v) = page.conversations.iter().find(|v| v.id == ephemeral.id) {
                assert_eq!(v.reason.as_deref(), Some("idle timeout"));
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap();
    drop(native);
    f.server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_canonical_membership_unread_reply_rotation_and_http_native_replay_share_one_ledger()
{
    use connect_protocol::{membership::*, provisioning::IdentityKind};
    let f = agent::Fixture::start().await;
    let config = f.issue_as_operator(&f.agent, StoreScope::collaboration(vec!["*".into()]));
    let mut native = ConnectClient::open(config.clone()).await.unwrap();
    let conversation = native
        .create("canonical-member-room", vec![])
        .await
        .unwrap();
    let cid = conversation.id;
    let add = MembershipRequest {
        operation_id: "quic-member-add".into(),
        conversation_id: cid.clone(),
        expected_revision: conversation.revision,
        change: MembershipAction::Add {
            identity_id: f.peer.identity_id.clone(),
            kind: IdentityKind::Human,
            role: "guest".into(),
        },
    };
    fn command(request: &MembershipRequest) -> Value {
        let (kind, payload) = match &request.change {
            MembershipAction::Add {
                identity_id,
                kind,
                role,
            } => (
                "conversation.participant.add",
                json!({"identity_id":identity_id,"kind":kind,"role":role}),
            ),
            MembershipAction::UpdateRole {
                participant_id,
                role,
            } => (
                "conversation.participant.update",
                json!({"participant_id":participant_id,"role":role}),
            ),
            MembershipAction::Remove {
                participant_id,
                reason,
            } => (
                "conversation.participant.remove",
                json!({"participant_id":participant_id,"reason":reason}),
            ),
            _ => panic!("canonical membership family"),
        };
        let mut value = env(kind, payload, Some(&request.operation_id));
        value["cid"] = json!(request.conversation_id);
        value["expected_revision"] = json!(request.expected_revision);
        value
    }
    let mut wire = Wire::open(&f, &config).await;
    wire.send(&command(&add), true).await;
    // Lose the response intentionally. Recover status without resubmitting admission.
    let record = timeout(Duration::from_secs(5), async {
        loop {
            match native
                .operation_action(OperationAction::Get {
                    operation_id: add.operation_id.clone(),
                })
                .await
            {
                Ok(record) => break record,
                Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(record.state, OperationState::Completed);
    wire.close().await;
    let rotated = f.issue_as_operator(&f.agent, StoreScope::collaboration(vec!["*".into()]));
    let mut native = ConnectClient::open(rotated.clone()).await.unwrap();
    let mut wire = Wire::open(&f, &rotated).await;
    let recovered = wire
        .exchange(env(
            "operation.get",
            json!({"operation_id":add.operation_id}),
            None,
        ))
        .await;
    compare(&recovered, &record);
    let mut request = add;
    let mut pid = String::new();
    for step in 0..3 {
        let input = command(&request);
        let reply = wire.exchange(input.clone()).await;
        let changed = wire.read().await;
        assert_eq!(changed.kind(), "conversation.participant.changed");
        for field in ["cid", "operation_id"] {
            assert_eq!(changed.original().get(field), input.get(field));
        }
        assert_eq!(changed.original()["in_reply_to"], input["id"]);
        assert_eq!(
            changed.original()["payload"]["kind"],
            ["added", "updated", "removed"][step]
        );
        let result = native.change_membership(request.clone()).await.unwrap();
        let current = http(
            &f,
            &rotated,
            OperationAction::Get {
                operation_id: request.operation_id.clone(),
            },
            None,
        )
        .await
        .unwrap();
        compare(&reply, &current);
        assert_eq!(
            current.result.as_ref().unwrap(),
            &serde_json::to_value(Outcome::Membership(result.clone())).unwrap()
        );
        let MembershipOutcome::Participant {
            participant,
            revision,
            ..
        } = result
        else {
            panic!()
        };
        if step == 0 {
            pid = participant.participant_id.clone();
            assert_eq!(participant.role, "guest");
        } else if step == 1 {
            assert_eq!(participant.role, "reviewer");
        } else {
            assert!(!participant.active);
            assert_eq!(
                participant.removal_reason.as_deref(),
                Some("review complete")
            );
        }
        assert_eq!(participant.membership_epoch, 1);
        let response = tokio::task::spawn_blocking(reqwest::Client::new)
            .await
            .unwrap()
            .post(format!(
                "http://{}/v1/operations",
                f.server.http_address.unwrap()
            ))
            .bearer_auth(&rotated.credential)
            .json(&Operation::Membership {
                request: request.clone(),
            })
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let from_http: Response = response.json().await.unwrap();
        assert_eq!(
            serde_json::to_value(from_http.result.unwrap()).unwrap(),
            *current.result.as_ref().unwrap()
        );
        if step == 2 {
            let mut changed = input.clone();
            changed["id"] = json!("changed-removal-reason");
            changed["payload"]["reason"] = json!("changed");
            let refusal = wire.exchange(changed).await;
            assert_eq!(refusal.kind(), "error");
            assert_eq!(refusal.original()["payload"]["code"], 409);
        }
        request = MembershipRequest {
            operation_id: format!("quic-member-step-{}", step + 1),
            conversation_id: cid.clone(),
            expected_revision: revision,
            change: if step == 0 {
                MembershipAction::UpdateRole {
                    participant_id: pid.clone(),
                    role: "reviewer".into(),
                }
            } else {
                MembershipAction::Remove {
                    participant_id: pid.clone(),
                    reason: Some("review complete".into()),
                }
            },
        };
    }
    let mut missing = command(&request);
    missing.as_object_mut().unwrap().remove("expected_revision");
    missing["operation_id"] = json!("missing-member-revision");
    // An envelope missing its required revision is refused by wire parsing, before correlation is trusted.
    wire.send(&missing, true).await;
    assert_eq!(wire.read().await.kind(), "error");
    wire.close().await;
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_canonical_subscription_snapshot_live_journal_catchup_and_lease_end_resync() {
    use connect_protocol::events::EventQuery;
    let f = agent::Fixture::start().await;
    let config = f.issue_as_operator(&f.agent, StoreScope::collaboration(vec!["*".into()]));
    let mut native = ConnectClient::open(config.clone()).await.unwrap();
    let room = native
        .create("streamed-room", vec![f.peer.identity_id.clone()])
        .await
        .unwrap();
    let mut wire = Wire::open(&f, &config).await;
    let mut subscribe = env(
        "conversation.subscribe",
        json!({"live":true,"limit":10,"lease_seconds":30}),
        None,
    );
    subscribe["cid"] = json!(room.id);
    let snapshot = wire.exchange(subscribe.clone()).await;
    assert_eq!(snapshot.kind(), "conversation.snapshot");
    let initial = wire.read().await;
    assert_eq!(initial.kind(), "ack");
    assert_eq!(initial.original()["payload"]["details"]["live"], true);
    let cursor = initial.original()["payload"]["details"]["cursor"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(cursor, snapshot.original()["payload"]["cursor"]);
    let sent = native
        .send(
            "live-after-snapshot",
            &room.id,
            "After the snapshot watermark",
        )
        .await
        .unwrap();
    let mut received = vec![];
    loop {
        let event = wire.read().await;
        assert_eq!(event.original()["in_reply_to"], subscribe["id"]);
        if event.kind() == "ack" {
            assert_eq!(
                event.original()["payload"]["details"]["resync_required"],
                false
            );
            break;
        }
        assert_eq!(event.kind(), "conversation.event");
        received.push(event.original()["payload"]["data"].clone());
    }
    let message = received
        .iter()
        .find(|e| e["data"]["kind"] == "message")
        .unwrap();
    assert_eq!(message["data"]["data"]["id"], sent.message_id);
    let replay = native
        .conversation_events(
            &room.id,
            EventQuery {
                after: Some(cursor.clone()),
                limit: Some(100),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    for observed in received {
        let retained = replay
            .events
            .iter()
            .find(|e| e.event_id == observed["event_id"])
            .unwrap();
        let mut expected = serde_json::to_value(retained).unwrap();
        expected.as_object_mut().unwrap().remove("cursor");
        let mut observed = observed;
        observed.as_object_mut().unwrap().remove("cursor");
        assert_eq!(observed, expected);
    }
    wire.close().await;
    // A replacement physical peer resumes the retained cursor without another mutation.
    let mut wire = Wire::open(&f, &config).await;
    let mut catchup = env(
        "conversation.subscribe",
        json!({"after":cursor,"live":false,"limit":1}),
        None,
    );
    catchup["cid"] = json!(room.id);
    wire.send(&catchup, true).await;
    let first = wire.read().await;
    assert_eq!(first.original()["in_reply_to"], catchup["id"]);
    assert_eq!(first.original()["cid"], catchup["cid"]);
    assert_eq!(
        first.original().get("operation_id"),
        replay.events[0]
            .operation_id
            .as_ref()
            .map(|id| serde_json::Value::String(id.clone()))
            .as_ref()
    );
    assert_eq!(first.kind(), "conversation.event");
    assert_eq!(
        first.original()["payload"]["event_id"],
        replay.events[0].event_id
    );
    assert_eq!(wire.read().await.kind(), "ack");
    // Finite live lease ends with an explicit newly authorized resync boundary.
    let mut short = env(
        "conversation.subscribe",
        json!({"after":replay.cursor,"live":true,"limit":10,"lease_seconds":3}),
        None,
    );
    short["cid"] = json!(room.id);
    let ready = wire.exchange(short.clone()).await;
    assert_eq!(ready.kind(), "ack");
    assert_eq!(ready.original()["payload"]["details"]["live"], true);
    let boundary = wire.read().await;
    assert_eq!(boundary.kind(), "conversation.snapshot");
    assert_eq!(boundary.original()["in_reply_to"], short["id"]);
    let ended = wire.read().await;
    assert_eq!(ended.kind(), "ack");
    assert_eq!(
        ended.original()["payload"]["details"]["resync_required"],
        true
    );
    assert_eq!(ended.original()["payload"]["details"]["live"], false);
    wire.close().await;
    f.close().await;
}

// Trusted enrollment on the disposable installation; the client carries only its signer.
fn bind_wire_credential(f: &agent::Fixture, wire: &Wire, config: &ClientConfig) {
    let signed = wire
        .signer
        .sign(&env(
            "auth.refresh",
            json!({"method":"bearer","credential":"fixture-unused"}),
            None,
        ))
        .unwrap();
    let key = signed["signature"]["keyid"].as_str().unwrap();
    let c = rusqlite::Connection::open(f.server.store.path()).unwrap();
    let (device,public,not_before,expires,revision):(String,Vec<u8>,i64,i64,u64)=c.query_row("SELECT device_id,public_key,not_before,expires_at,revision FROM peer_signing_keys WHERE key_id=?1",[key],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
    drop(c);
    f.server
        .store
        .execute_provisioning(
            &f.admin().credential_id,
            ProvisioningCommand::SigningKey {
                operation_id: format!("bind-{}", config.credential_id),
                identity_id: config.identity_id.clone(),
                credential_id: config.credential_id.clone(),
                device_id: device,
                key_id: key.into(),
                algorithm: SigningAlgorithm::EdDSA,
                public_key: URL_SAFE_NO_PAD.encode(public),
                not_before,
                expires_at: expires,
                expected_revision: Some(revision),
            },
        )
        .unwrap();
}
async fn start_live(wire: &mut Wire, cid: &str) -> Value {
    let mut request = env(
        "conversation.subscribe",
        json!({"live":true,"limit":100,"lease_seconds":20}),
        None,
    );
    request["cid"] = json!(cid);
    assert_eq!(
        wire.exchange(request.clone()).await.kind(),
        "conversation.snapshot"
    );
    let ack = wire.read().await;
    assert_eq!(ack.kind(), "ack");
    assert_eq!(ack.original()["payload"]["details"]["live"], true);
    request
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_canonical_subscription_refresh_retires_old_stream_without_borrowing_successor_scope(
) {
    let f = agent::Fixture::start().await;
    let config = f.issue_as_operator(&f.agent, StoreScope::collaboration(vec!["*".into()]));
    let mut native = ConnectClient::open(config.clone()).await.unwrap();
    let room = native
        .create("refresh-stream-room", vec![f.peer.identity_id.clone()])
        .await
        .unwrap();
    let mut wire = Wire::open(&f, &config).await;
    start_live(&mut wire, &room.id).await;
    let narrow = f.issue_as_operator(&f.agent, StoreScope::communications(vec!["*".into()]));
    bind_wire_credential(&f, &wire, &narrow);
    let refresh = env(
        "auth.refresh",
        json!({"method":"bearer","credential":narrow.credential}),
        None,
    );
    assert_eq!(wire.exchange(refresh).await.kind(), "auth.session");
    native
        .send(
            "after-narrow-refresh",
            &room.id,
            "Old subscription may not borrow a new credential",
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    let mut history = env("message.history", json!({"limit":100}), None);
    history["cid"] = json!(room.id);
    assert_eq!(wire.exchange(history).await.kind(), "message.send");
    assert_eq!(wire.read().await.kind(), "ack");
    let mut subscribe = env("conversation.subscribe", json!({"live":true}), None);
    subscribe["cid"] = json!(room.id);
    let denied = wire.exchange(subscribe).await;
    assert_eq!(denied.kind(), "error");
    assert_eq!(denied.original()["payload"]["code"], 403);
    wire.close().await;
    f.close().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_canonical_subscription_credential_revocation_has_no_idle_guard_and_emits_terminal_pair(
) {
    let f = agent::Fixture::start().await;
    let config = f.issue_as_operator(&f.agent, StoreScope::collaboration(vec!["*".into()]));
    let mut native = ConnectClient::open(config.clone()).await.unwrap();
    let room = native.create("revoked-stream-room", vec![]).await.unwrap();
    let mut wire = Wire::open(&f, &config).await;
    start_live(&mut wire, &room.id).await;
    let start = tokio::time::Instant::now();
    f.revoke(&config.credential_id);
    assert!(start.elapsed() < Duration::from_secs(2));
    assert_eq!(wire.read().await.kind(), "identity.assurance-changed");
    assert_eq!(wire.read().await.kind(), "auth.bye");
    wire.close().await;
    f.close().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_canonical_subscription_does_not_revive_after_member_removal_and_readmission() {
    use connect_protocol::{membership::*, provisioning::IdentityKind};
    let f = agent::Fixture::start().await;
    let config = f.issue_as_operator(&f.agent, StoreScope::collaboration(vec!["*".into()]));
    let manager = f.issue_as_operator(&f.peer, StoreScope::collaboration(vec!["*".into()]));
    let mut owner = ConnectClient::open(config.clone()).await.unwrap();
    let mut manager = ConnectClient::open(manager).await.unwrap();
    let room = owner
        .create("epoch-stream-room", vec![f.peer.identity_id.clone()])
        .await
        .unwrap();
    let mut wire = Wire::open(&f, &config).await;
    start_live(&mut wire, &room.id).await;
    let state = manager.membership_snapshot(&room.id).await.unwrap();
    let member = state
        .participants
        .iter()
        .find(|p| p.identity_id == config.identity_id)
        .unwrap();
    let removed = manager
        .change_membership(MembershipRequest {
            operation_id: "stream-remove".into(),
            conversation_id: room.id.clone(),
            expected_revision: state.revision,
            change: MembershipAction::Remove {
                participant_id: member.participant_id.clone(),
                reason: None,
            },
        })
        .await
        .unwrap();
    let MembershipOutcome::Participant { revision, .. } = removed else {
        panic!()
    };
    manager
        .change_membership(MembershipRequest {
            operation_id: "stream-readmit".into(),
            conversation_id: room.id.clone(),
            expected_revision: revision,
            change: MembershipAction::Add {
                identity_id: config.identity_id.clone(),
                kind: IdentityKind::Human,
                role: "participant".into(),
            },
        })
        .await
        .unwrap();
    manager
        .send(
            "new-epoch-message",
            &room.id,
            "This is only available under fresh epoch authority",
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    let mut history = env("message.history", json!({"limit":100}), None);
    history["cid"] = json!(room.id);
    assert_eq!(wire.exchange(history).await.kind(), "message.send");
    assert_eq!(wire.read().await.kind(), "ack");
    wire.close().await;
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_canonical_subscription_capacity_replacement_and_http_cursor_parity() {
    use connect_protocol::events::EventQuery;
    let f = agent::Fixture::start().await;
    let config = f.issue_as_operator(&f.agent, StoreScope::collaboration(vec!["*".into()]));
    let mut native = ConnectClient::open(config.clone()).await.unwrap();
    let mut rooms = vec![];
    for n in 0..9 {
        rooms.push(
            native
                .create(&format!("subscription-capacity-{n}"), vec![])
                .await
                .unwrap()
                .id,
        );
    }
    let mut wire = Wire::open(&f, &config).await;
    let mut first = None;
    for room in &rooms[..8] {
        let request = start_live(&mut wire, room).await;
        if first.is_none() {
            first = Some(request)
        }
    }
    let mut excess = env(
        "conversation.subscribe",
        json!({"live":true,"lease_seconds":20}),
        None,
    );
    excess["cid"] = json!(rooms[8]);
    let refused = wire.exchange(excess).await;
    assert_eq!(refused.kind(), "error");
    assert_eq!(refused.original()["payload"]["code"], 429);
    let replacement = start_live(&mut wire, &rooms[0]).await;
    let sent = native
        .send(
            "after-subscription-replacement",
            &rooms[0],
            "Only the replacement registration may stream this",
        )
        .await
        .unwrap();
    let mut message = None;
    loop {
        let event = wire.read().await;
        assert_eq!(event.original()["in_reply_to"], replacement["id"]);
        assert_ne!(
            event.original()["in_reply_to"],
            first.as_ref().unwrap()["id"]
        );
        if event.kind() == "ack" {
            break;
        }
        assert_eq!(event.kind(), "conversation.event");
        if event.original()["payload"]["data"]["data"]["kind"] == "message" {
            message = Some(event.original()["payload"]["data"].clone());
        }
    }
    let message = message.unwrap();
    assert_eq!(message["data"]["data"]["id"], sent.message_id);
    let request = Operation::ConversationEvents {
        conversation_id: rooms[0].clone(),
        query: EventQuery {
            after: Some(message["cursor"].as_str().unwrap().into()),
            limit: Some(100),
            ..Default::default()
        },
    };
    let response = tokio::task::spawn_blocking(reqwest::Client::new)
        .await
        .unwrap()
        .post(format!(
            "http://{}/v1/operations",
            f.server.http_address.unwrap()
        ))
        .bearer_auth(&config.credential)
        .json(&request)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    if let Some(dir) = std::env::var_os("CONNECT_GATE6_HTTP_SAMPLES") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("subscription-http-cursor.json"),serde_json::to_vec_pretty(&json!({"api":"events","route":"/v1/operations","status":200,"request":request,"response":body})).unwrap()).unwrap();
    }
    let Outcome::ConversationEvents(page) = serde_json::from_value::<Response>(body)
        .unwrap()
        .result
        .unwrap()
    else {
        panic!()
    };
    assert!(!page.resync_required);
    assert!(page
        .events
        .iter()
        .all(|event| event.event_id != message["event_id"]));
    wire.close().await;
    f.close().await;
}

#[path = "gate6_canonical_operations/read_receipts.rs"]
mod read_receipts;
