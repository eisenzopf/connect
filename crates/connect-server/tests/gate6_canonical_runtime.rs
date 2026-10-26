#![recursion_limit = "256"]
//! Actual production listener and public signed SDK. No fixture driver/factory.
#![allow(clippy::unwrap_used)]
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use connect_client::{
    canonical::{CanonicalClient, CanonicalClientConfig, CommandScope},
    ClientConfig, ClientError,
};
use connect_protocol::{provisioning::*, Operation, Outcome};
use connect_server::{
    canonical::{CanonicalServerConfig, RunningCanonicalServer},
    cli::init,
    config::ServerConfig,
};
use connect_store::Scope;
use rvoip_uctp::v2::{payloads::Payload, Envelope};
use serde_json::{json, Value};
use std::{path::Path, time::Duration};

#[path = "support/canonical_listener_fixture.rs"]
mod canonical_listener_fixture;
use canonical_listener_fixture::*;

#[path = "support/external_sdk_consumer.rs"]
mod external_sdk_consumer;

#[path = "support/connector_media_contract.rs"]
mod connector_media_contract;

#[path = "support/canonical_agent_rehearsal.rs"]
mod canonical_agent_rehearsal;

#[path = "support/canonical_mcp.rs"]
mod canonical_mcp;

#[path = "support/canonical_stdio.rs"]
mod canonical_stdio;
#[path = "support/credential_installation.rs"]
mod credential_installation;
#[path = "support/credential_issuance.rs"]
mod credential_issuance;

fn payload(kind: &str, value: Value) -> Payload {
    serde_json::from_value(json!({"type":kind,"payload":value})).unwrap()
}
async fn send(
    client: &mut CanonicalClient,
    kind: &str,
    value: Value,
    cid: Option<&str>,
    operation: Option<&str>,
) -> String {
    client
        .send(
            payload(kind, value),
            CommandScope {
                conversation_id: cid.map(str::to_owned),
                operation_id: operation.map(str::to_owned),
                ..Default::default()
            },
        )
        .await
        .unwrap()
}
async fn reply(client: &mut CanonicalClient, id: &str, kind: &str) -> Envelope {
    use canonical_native_connections::ControlReception;
    let result = client.receive_control().await.unwrap();
    assert_eq!(result.kind(), kind, "{result:?}");
    assert_eq!(result.original()["in_reply_to"], id);
    result
}
async fn create(f: &Fixture, client: &mut CanonicalClient, id: &str) -> String {
    let mut input =
        connect_client::conversations::CreateRequest::persistent(vec![f.peer.identity_id.clone()]);
    input.tenant_id = Some("tnt_default".into());
    client.create_conversation(id, input).await.unwrap().id
}

#[tokio::test]
async fn canonical_http_sdk_uses_shared_commands_receipts_membership_and_cursor_facts() {
    use connect_client::{
        canonical::CanonicalHttpClient, conversations::*, membership::MembershipOutcome,
        MessageDraft,
    };
    let f = Fixture::with_membership_scope("EdDSA", 8, true).await;
    let mut http = CanonicalHttpClient::open(canonical_http_config(&f))
        .await
        .unwrap();
    let mut native = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut input = CreateRequest::persistent(vec![f.peer.identity_id.clone()]);
    input.tenant_id = Some("tnt_default".into());
    let room = http
        .create_conversation("http-canonical-create", input.clone())
        .await
        .unwrap();
    assert_eq!(
        native
            .create_conversation("http-canonical-create", input)
            .await
            .unwrap()
            .id,
        room.id
    );
    assert!(http
        .list_conversations(serde_json::from_value(json!({})).unwrap())
        .await
        .unwrap()
        .conversations
        .iter()
        .any(|c| c.id == room.id));
    let query = || serde_json::from_value(json!({"live":false})).unwrap();
    let baseline = http
        .subscribe_conversation(&room.id, query())
        .await
        .unwrap();
    let cursor = baseline.ack().unwrap().details.as_ref().unwrap()["cursor"]
        .as_str()
        .unwrap()
        .to_owned();
    let sent = http
        .send_message(
            "http-canonical-send",
            &room.id,
            "http_canonical_message",
            MessageDraft::text_all("HTTP canonical data"),
        )
        .await
        .unwrap();
    let replay = native
        .send_message(
            "http-canonical-send",
            &room.id,
            "http_canonical_message",
            MessageDraft::text_all("HTTP canonical data"),
        )
        .await
        .unwrap();
    assert_eq!(sent.message_id, replay.message_id);
    assert_eq!(sent.receipt, replay.receipt);
    let history = http
        .history(&room.id, serde_json::from_value(json!({})).unwrap())
        .await
        .unwrap();
    assert_eq!(history.messages.len(), 1);
    assert_eq!(history.messages[0].body, "HTTP canonical data");
    let status = http
        .operation_get(
            "http-canonical-send",
            CommandScope {
                conversation_id: Some(room.id.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        status.result.as_ref().unwrap()["data"]["message_id"],
        sent.message_id
    );
    let actor = f.server.store.authenticate(&f.client.credential).unwrap();
    f.server
        .store
        .execute(
            &actor.credential_id,
            &Operation::ReceiptPreferencesSet {
                request_id: "http-read-preference".into(),
                conversation_id: room.id.clone(),
                delivery: true,
                read: true,
            },
        )
        .unwrap();
    let peer = f
        .server
        .store
        .issue(
            &f.peer.identity_id,
            Scope::communications(vec![f.client.identity_id.clone()]),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let mut peer_config = canonical_http_config(&f);
    peer_config.credential = peer.token;
    let mut peer_http = CanonicalHttpClient::open(peer_config).await.unwrap();
    peer_http
        .send_message(
            "http-peer-send",
            &room.id,
            "http_peer_message",
            MessageDraft::text_all("read me"),
        )
        .await
        .unwrap();
    let timestamp = chrono::Utc::now();
    let read = http
        .report_read("http-read", &room.id, "http_peer_message", timestamp)
        .await
        .unwrap();
    assert_eq!(serde_json::to_value(read.recorded).unwrap(), json!("read"));
    let native_read = native
        .report_read("http-read", &room.id, "http_peer_message", timestamp)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(read).unwrap(),
        serde_json::to_value(native_read).unwrap()
    );
    let catchup = http
        .subscribe_conversation(
            &room.id,
            serde_json::from_value(json!({"after":cursor,"live":false})).unwrap(),
        )
        .await
        .unwrap();
    assert!(catchup
        .frames
        .iter()
        .any(|e| e.kind() == "conversation.event"
            && e.original()["payload"]["data"]["operation_id"] == "http-canonical-send"));
    let native_catchup = native
        .subscribe_conversation(
            &room.id,
            serde_json::from_value(json!({"after":cursor,"live":false})).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        catchup
            .frames
            .iter()
            .filter(|e| e.kind() == "conversation.event")
            .count(),
        native_catchup
            .frames
            .iter()
            .filter(|e| e.kind() == "conversation.event")
            .count()
    );
    let outsider = f
        .server
        .store
        .create_identity("http-extra-participant")
        .unwrap();
    let snapshot = http
        .conversation_snapshot(&room.id, serde_json::from_value(json!({})).unwrap())
        .await
        .unwrap();
    let addition = payload(
        "conversation.participant.add",
        json!({"identity_id":outsider.id,"kind":"human","role":"observer"}),
    );
    let added = http
        .participant_change(
            &room.id,
            "http-add",
            Some(snapshot.revision),
            addition.clone(),
        )
        .await
        .unwrap();
    let replay = native
        .participant_change(&room.id, "http-add", Some(snapshot.revision), addition)
        .await
        .unwrap();
    assert_eq!(added, replay);
    let MembershipOutcome::Participant { participant, .. } = added else {
        panic!()
    };
    let snapshot = http
        .conversation_snapshot(&room.id, serde_json::from_value(json!({})).unwrap())
        .await
        .unwrap();
    http.participant_change(
        &room.id,
        "http-role",
        Some(snapshot.revision),
        payload(
            "conversation.participant.update",
            json!({"participant_id":participant.participant_id,"role":"reviewer"}),
        ),
    )
    .await
    .unwrap();
    let snapshot = http
        .conversation_snapshot(&room.id, serde_json::from_value(json!({})).unwrap())
        .await
        .unwrap();
    let removed = http
        .participant_change(
            &room.id,
            "http-remove",
            Some(snapshot.revision),
            payload(
                "conversation.participant.remove",
                json!({"participant_id":participant.participant_id,"reason":"HTTP parity"}),
            ),
        )
        .await
        .unwrap();
    assert!(
        matches!(removed,MembershipOutcome::Participant {participant,..} if !participant.active)
    );
    let snapshot = http
        .conversation_snapshot(&room.id, serde_json::from_value(json!({})).unwrap())
        .await
        .unwrap();
    let options = CloseOptions {
        expected_revision: Some(snapshot.revision),
        reason_code: Some(42),
        reason: Some("HTTP close".into()),
    };
    assert!(
        http.close_conversation("http-close", &room.id, options.clone())
            .await
            .unwrap()
            .closed
    );
    assert!(
        native
            .close_conversation("http-close", &room.id, options)
            .await
            .unwrap()
            .closed
    );
    native.close();
    f.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn signed_invitation_decisions_share_http_receipts_without_changing_other_sessions() {
    use connect_protocol::session_decisions::{Decision, Request};
    fn http_request(r: &Request) -> Value {
        let mut value = json!({"v":2,"type":r.decision.required_scope(),"id":format!("http_{}",uuid::Uuid::new_v4().simple()),"ts":chrono::Utc::now(),
            "cid":r.conversation_id,"sid":r.session_id,"operation_id":r.operation_id,"payload":{"reason_code":r.reason_code,"reason":r.reason}});
        if let Some(by) = &r.by_participant {
            value["payload"]["by"] = json!(by);
        }
        if let Some(revision) = r.expected_revision {
            value["expected_revision"] = json!(revision);
        }
        value
    }
    for decision in [Decision::Cancel, Decision::Reject] {
        let f = Fixture::new("EdDSA", 8).await;
        let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
        let cid = create(&f, &mut client, "decision-conversation").await;
        let actor = f.server.store.authenticate(&f.client.credential).unwrap();
        let peer = f
            .server
            .store
            .issue(
                &f.peer.identity_id,
                Scope::communications(vec![f.client.identity_id.clone()]),
                chrono::Utc::now().timestamp() + 600,
            )
            .unwrap();
        let (initiator, target) = match decision {
            Decision::Cancel => (actor.credential_id.clone(), f.peer.identity_id.clone()),
            Decision::Reject => (peer.id, f.client.identity_id.clone()),
        };
        let mut sessions = vec![];
        // Trusted fixture arrangement uses the pre-existing invitation API. This
        // test qualifies decisions, not the later general Session creation model.
        for id in ["selected-invitation", "other-invitation"] {
            let Outcome::Session(s) = f
                .server
                .store
                .execute(
                    &initiator,
                    &Operation::SessionInvite {
                        request_id: id.into(),
                        conversation_id: cid.clone(),
                        participant_id: target.clone(),
                        media: vec!["audio".into()],
                    },
                )
                .unwrap()
            else {
                panic!()
            };
            sessions.push(s.id);
        }
        let sid = &sessions[0];
        let snapshot = f
            .server
            .store
            .membership_snapshot(&actor.credential_id, &cid)
            .unwrap();
        let by = snapshot
            .participants
            .iter()
            .find(|p| p.identity_id == f.client.identity_id)
            .unwrap()
            .participant_id
            .clone();
        let scope = CommandScope {
            conversation_id: Some(cid.clone()),
            session_id: Some(sid.clone()),
            operation_id: Some("decision-operation".into()),
            expected_revision: Some(0),
            ..Default::default()
        };
        let batch = client
            .request(
                payload(
                    decision.required_scope(),
                    json!({"by":by,"reason_code":42,"reason":"invitation decision"}),
                ),
                scope.clone(),
            )
            .await
            .unwrap();
        batch.ack().unwrap();
        let native = batch
            .frames
            .iter()
            .find(|e| e.kind() == "operation.updated")
            .unwrap()
            .original()["payload"]
            .clone();
        assert_eq!(native["state"], "completed");
        assert_eq!(native["result"]["data"]["reason_code"], 42);
        assert_eq!(native["result"]["data"]["revision"], 1);
        let request = Request {
            operation_id: "decision-operation".into(),
            conversation_id: cid.clone(),
            session_id: sid.clone(),
            decision,
            by_participant: Some(by),
            expected_revision: Some(0),
            reason_code: 42,
            reason: "invitation decision".into(),
        };
        let http = tokio::task::spawn_blocking(reqwest::Client::new)
            .await
            .unwrap();
        let mut samples = vec![];
        let operation = http_request(&request);
        let response = http
            .post(format!(
                "http://{}/v2/commands",
                f.server.http_address.unwrap()
            ))
            .bearer_auth(&f.client.credential)
            .json(&operation)
            .send()
            .await
            .unwrap();
        let status = response.status().as_u16();
        assert_eq!(status, 200);
        let response: Value = response.json().await.unwrap();
        samples
            .push(json!({"case":"replay","status":status,"request":operation,"response":response}));
        assert_eq!(
            response["envelopes"][0]["payload"]["result"],
            native["result"]
        );
        let mut http_canonical =
            connect_client::canonical::CanonicalHttpClient::open(canonical_http_config(&f))
                .await
                .unwrap();
        let http_decision = http_canonical
            .decide_session_invitation(request.clone())
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(http_decision).unwrap(),
            native["result"]["data"]
        );
        let replay = client
            .decide_session_invitation(request.clone())
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(replay).unwrap(),
            native["result"]["data"]
        );
        let recovered = client
            .operation_get(
                "decision-operation",
                CommandScope {
                    conversation_id: Some(cid.clone()),
                    session_id: Some(sid.clone()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(recovered.result.unwrap(), native["result"]);
        for (case, code) in [("spoof", "forbidden"), ("stale", "conflict")] {
            let mut invalid = request.clone();
            invalid.operation_id = format!("decision-{case}");
            invalid.session_id = sessions[1].clone();
            if case == "spoof" {
                invalid.by_participant = Some("foreign-participant".into());
            } else {
                invalid.expected_revision = Some(2);
            }
            assert!(
                matches!(client.decide_session_invitation(invalid.clone()).await,
                Err(ClientError::Application(e)) if e.code==code)
            );
            let op = http_request(&invalid);
            let response = http
                .post(format!(
                    "http://{}/v2/commands",
                    f.server.http_address.unwrap()
                ))
                .bearer_auth(&f.client.credential)
                .json(&op)
                .send()
                .await
                .unwrap();
            let status = response.status().as_u16();
            assert_eq!(status, if case == "spoof" { 403 } else { 409 });
            let response: Value = response.json().await.unwrap();
            samples.push(json!({"case":case,"status":status,"request":op,"response":response}));
            assert_eq!(
                response["envelopes"].as_array().unwrap().last().unwrap()["payload"]["code"],
                status
            );
        }
        if let Ok(directory) = std::env::var("CONNECT_G6_SESSION_DECISION_SAMPLES") {
            std::fs::write(
                std::path::Path::new(&directory).join(format!("{decision:?}.json")),
                serde_json::to_vec_pretty(&samples).unwrap(),
            )
            .unwrap();
        }
        let unchanged = f
            .server
            .store
            .execute(
                &actor.credential_id,
                &Operation::SessionGet {
                    conversation_id: cid.clone(),
                    session_id: sessions[1].clone(),
                },
            )
            .unwrap();
        assert!(matches!(unchanged,Outcome::Session(s) if s.state=="invited"));
        client.close();
        f.server.shutdown().await.unwrap();
    }
}
#[tokio::test]
async fn production_signed_sdk_shares_http_receipts_and_recovers_same_operation_after_restart() {
    for algorithm in ["EdDSA", "ES256"] {
        let mut f = Fixture::new(algorithm, 8).await;
        let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
        let cid = create(&f, &mut client, "create-canonical").await;
        let body = json!({"msg_id":"canonical_runtime_message","label":"connect-messages","content_type":"text/plain","body":"production canonical","body_encoding":"utf8","to":"all","reliability":{"mode":"reliable_ordered"}});
        let id = send(
            &mut client,
            "message.send",
            body.clone(),
            Some(&cid),
            Some("send-canonical"),
        )
        .await;
        let ack = reply(&mut client, &id, "ack").await;
        let details = ack.original()["payload"]["details"].clone();
        // Native system trust/proxy setup can block for seconds on macOS.
        // Keep that setup off the executor that drives the colocated QUIC peer.
        let mut http =
            connect_client::canonical::CanonicalHttpClient::open(canonical_http_config(&f))
                .await
                .unwrap();
        let history = http
            .history(&cid, serde_json::from_value(json!({"limit":100})).unwrap())
            .await
            .unwrap();
        assert_eq!(history.messages.len(), 1);
        assert_eq!(history.messages[0].body, "production canonical");
        client.close();
        let shutdown = tokio::time::Instant::now();
        f.server.shutdown().await.unwrap();
        assert!(shutdown.elapsed() < Duration::from_secs(10));
        f.server = RunningCanonicalServer::start(f.config.clone())
            .await
            .unwrap();
        f.client.endpoint = f.server.address;
        let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
        let id = send(
            &mut client,
            "message.send",
            body,
            Some(&cid),
            Some("send-canonical"),
        )
        .await;
        let replay = reply(&mut client, &id, "ack").await;
        assert_eq!(replay.original()["payload"]["details"], details);
        let id = send(
            &mut client,
            "message.history",
            json!({"limit":10}),
            Some(&cid),
            None,
        )
        .await;
        let message = reply(&mut client, &id, "message.send").await;
        assert_eq!(
            message.original()["payload"]["body"],
            "production canonical"
        );
        reply(&mut client, &id, "ack").await;
        client.close();
        let shutdown = tokio::time::Instant::now();
        f.server.shutdown().await.unwrap();
        assert!(shutdown.elapsed() < Duration::from_secs(10));
    }
}
#[tokio::test]
async fn production_client_rejects_wrong_server_pin_tls_trust_and_enrolled_device_or_signer() {
    let f = Fixture::new("EdDSA", 8).await;
    let wrong = f.temp.path().join("wrong.pub");
    std::fs::write(&wrong, vec![3u8; 32]).unwrap();
    let mut cfg = f.client.clone();
    cfg.server_public_key = wrong;
    assert!(matches!(
        CanonicalClient::open(cfg).await,
        Err(ClientError::Authentication)
    ));
    let mut cfg = f.client.clone();
    cfg.device_platform = "foreign-platform".into();
    assert!(CanonicalClient::open(cfg).await.is_err());
    let mut cfg = f.client.clone();
    cfg.signing_key_id = "foreign-key".into();
    assert!(CanonicalClient::open(cfg).await.is_err());
    let wrong = f.temp.path().join("wrong.der");
    signing(&wrong, "EdDSA");
    let mut cfg = f.client.clone();
    cfg.signing_private_key = wrong;
    assert!(CanonicalClient::open(cfg).await.is_err());
    let cert = connect_server::tls::generate_local_certificate().unwrap();
    let wrong = f.temp.path().join("wrong-cert.der");
    std::fs::write(&wrong, cert.cert.der()).unwrap();
    let mut cfg = f.client.clone();
    cfg.trust_certificate = wrong;
    assert!(matches!(
        CanonicalClient::open(cfg).await,
        Err(ClientError::Transport)
    ));
    let mut cfg = f.client.clone();
    cfg.server_algorithm = "ES256".into();
    assert!(matches!(
        CanonicalClient::open(cfg).await,
        Err(ClientError::Configuration)
    ));
    let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
    let id = send(
        &mut client,
        "conversation.list",
        json!({"limit":1}),
        None,
        None,
    )
    .await;
    reply(&mut client, &id, "ack").await;
    client.close();
    f.server.shutdown().await.unwrap();
}
#[tokio::test]
async fn cancelled_partial_receive_retires_peer_and_releases_its_physical_admission_slot() {
    let f = Fixture::new("EdDSA", 1).await;
    let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(30), client.receive())
            .await
            .is_err()
    );
    assert!(matches!(
        client
            .send(
                payload("conversation.list", json!({"limit":1})),
                Default::default()
            )
            .await,
        Err(ClientError::Transport)
    ));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    let mut next = loop {
        match CanonicalClient::open(f.client.clone()).await {
            Ok(client) => break client,
            Err(_) if tokio::time::Instant::now() < deadline => tokio::task::yield_now().await,
            Err(error) => panic!("slot not released: {error:?}"),
        }
    };
    let id = send(
        &mut next,
        "conversation.list",
        json!({"limit":1}),
        None,
        None,
    )
    .await;
    reply(&mut next, &id, "ack").await;
    next.close();
    f.server.shutdown().await.unwrap();
}
#[tokio::test]
async fn production_current_key_revocation_retires_signed_client_without_borrowing_new_authority() {
    let f = Fixture::new("EdDSA", 8).await;
    let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
    let actor = f.server.store.authenticate(&f.admin).unwrap();
    f.server
        .store
        .execute_provisioning(
            &actor.credential_id,
            ProvisioningCommand::RevokeSigningKey {
                operation_id: "retire-canonical-key".into(),
                key_id: f.client.signing_key_id.clone(),
                expected_revision: f.key_revision,
            },
        )
        .unwrap();
    let terminal = client.receive().await.unwrap();
    assert!(matches!(
        terminal.kind(),
        "identity.assurance-changed" | "auth.bye"
    ));
    assert!(client.receive().await.is_err());
    assert!(CanonicalClient::open(f.client.clone()).await.is_err());
    f.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn invalid_preflight_has_no_transport_effect_and_signing_expiry_closes_native_intake() {
    let mut f = Fixture::new("EdDSA", 8).await;
    let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
    assert!(matches!(
        client
            .send(
                payload(
                    "conversation.create",
                    json!({"tenant_id":"tnt_default","policy":"persistent"})
                ),
                Default::default()
            )
            .await,
        Err(ClientError::Protocol)
    ));
    let id = send(
        &mut client,
        "conversation.list",
        json!({"limit":1}),
        None,
        None,
    )
    .await;
    reply(&mut client, &id, "ack").await;
    client.close();
    f.server.shutdown().await.unwrap();
    f.config.signing_key_expires_at = chrono::Utc::now() + chrono::Duration::seconds(2);
    f.server = RunningCanonicalServer::start(f.config.clone())
        .await
        .unwrap();
    f.client.endpoint = f.server.address;
    let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(4), f.server.service.readiness.failed())
        .await
        .unwrap();
    assert!(client.receive().await.is_err());
    assert!(CanonicalClient::open(f.client.clone()).await.is_err());
    assert!(f.server.shutdown().await.is_err());
    assert!(RunningCanonicalServer::start(f.config).await.is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn installed_canonical_cli_serves_signed_sdk_and_sigterm_releases_installation() {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let f = Fixture::new("ES256", 8).await;
    f.server.shutdown().await.unwrap();
    let config = f.temp.path().join("canonical-server.toml");
    std::fs::write(&config, toml::to_string(&f.config).unwrap()).unwrap();
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_connect"))
        .arg("serve")
        .arg("--config")
        .arg(&config)
        .kill_on_drop(true)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let ready = tokio::time::timeout(Duration::from_secs(10), async {
        for _ in 0..8 {
            let line = lines.next_line().await.unwrap().expect("CLI ready line");
            assert!(line.len() < 4096);
            if line.starts_with("ready ") {
                return line;
            }
        }
        panic!("no ready line")
    })
    .await
    .unwrap();
    assert!(ready.contains("profile=connect/2"));
    let address = ready
        .split_whitespace()
        .find_map(|p| p.strip_prefix("address="))
        .unwrap()
        .parse()
        .unwrap();
    let mut config = f.client;
    config.endpoint = address;
    let mut client = CanonicalClient::open(config).await.unwrap();
    let id = send(
        &mut client,
        "conversation.list",
        json!({"limit":1}),
        None,
        None,
    )
    .await;
    reply(&mut client, &id, "ack").await;
    let pid = rustix::process::Pid::from_raw(child.id().unwrap() as i32).unwrap();
    rustix::process::kill_process(pid, rustix::process::Signal::TERM).unwrap();
    let status = tokio::time::timeout(Duration::from_secs(10), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success());
    assert!(client.receive().await.is_err());
    let server = RunningCanonicalServer::start(f.config).await.unwrap();
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn production_physical_factory_delivers_live_subscription_with_original_request_correlation()
{
    let f = Fixture::new("EdDSA", 8).await;
    let mut subscriber = CanonicalClient::open(f.client.clone()).await.unwrap();
    let cid = create(&f, &mut subscriber, "subscribe-room").await;
    let request = send(
        &mut subscriber,
        "conversation.subscribe",
        json!({"live":true,"limit":10,"lease_seconds":30}),
        Some(&cid),
        None,
    )
    .await;
    reply(&mut subscriber, &request, "conversation.snapshot").await;
    let ack = reply(&mut subscriber, &request, "ack").await;
    assert_eq!(ack.original()["payload"]["details"]["live"], true);
    let mut writer = CanonicalClient::open(f.client.clone()).await.unwrap();
    let heartbeat = writer.keepalive().await.unwrap();
    reply(&mut writer, &heartbeat, "ack").await;
    let sent=send(&mut writer,"message.send",json!({"msg_id":"canonical_live_message","label":"connect-messages","content_type":"text/plain","body":"live production","body_encoding":"utf8","to":"all"}),Some(&cid),Some("canonical-live-send")).await;
    reply(&mut writer, &sent, "ack").await;
    let mut found = false;
    let mut complete = false;
    for _ in 0..16 {
        let event = subscriber.receive().await.unwrap();
        assert_eq!(event.original()["in_reply_to"], request);
        assert_eq!(event.original()["cid"], cid);
        if event.kind() == "ack" {
            complete = true;
            break;
        }
        assert_eq!(event.kind(), "conversation.event");
        if event.original()["payload"]["data"]
            .to_string()
            .contains("canonical_live_message")
        {
            found = true;
        }
    }
    assert!(found && complete);
    writer.close();
    subscriber.close();
    f.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn signed_typed_calls_share_receipts_page_history_and_replay_closed_conversations() {
    use connect_client::{conversations::*, MessageDraft};
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::new(algorithm, 8).await;
        let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
        let mut input = CreateRequest::persistent(vec![f.peer.identity_id.clone()]);
        input.tenant_id = Some("tnt_default".into());
        input.metadata = Some(json!({"sdk":"typed"}));
        let room = client
            .create_conversation("typed-room", input.clone())
            .await
            .unwrap();
        assert!(!room.closed);
        assert!(room
            .receipt
            .as_ref()
            .unwrap()
            .matches("typed-room", &room.id));
        for n in 0..3 {
            let sent = client
                .send_message(
                    &format!("typed-send-{n}"),
                    &room.id,
                    &format!("typed_message_{n}"),
                    MessageDraft::text_all(format!("message {n}")),
                )
                .await
                .unwrap();
            assert!(sent.receipt.matches(&format!("typed-send-{n}"), &room.id));
            let state = client
                .operation_get(
                    &sent.request_id,
                    CommandScope {
                        conversation_id: Some(room.id.clone()),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            assert_eq!(state.operation_id.as_str(), sent.request_id);
            assert!(state.result.is_some());
        }
        let mut query = match payload("message.history", json!({"limit":1})) {
            Payload::MessageHistory(v) => v,
            _ => unreachable!(),
        };
        let mut ids = Vec::new();
        loop {
            let page = client.history(&room.id, query.clone()).await.unwrap();
            assert!(page.messages.len() <= 1);
            ids.extend(page.messages.iter().map(|m| m.msg_id.as_str().to_owned()));
            if page.next_cursor.is_none() {
                break;
            }
            query.cursor = page.next_cursor;
        }
        assert_eq!(
            ids,
            vec!["typed_message_0", "typed_message_1", "typed_message_2"]
        );
        let closed = client
            .close_conversation(
                "typed-close",
                &room.id,
                CloseOptions {
                    expected_revision: Some(room.revision),
                    reason_code: Some(42),
                    reason: Some("typed close".into()),
                },
            )
            .await
            .unwrap();
        assert!(closed.closed);
        assert!(closed
            .receipt
            .as_ref()
            .unwrap()
            .matches("typed-close", &room.id));
        let replay = client
            .create_conversation("typed-room", input)
            .await
            .unwrap();
        assert_eq!(replay.id, room.id);
        assert!(replay.closed);
        assert_eq!(replay.receipt, room.receipt);
        let page = client
            .list_conversations(ListQuery {
                filter: ListFilter {
                    state: ConversationState::Closed,
                    ..Default::default()
                },
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(page.conversations.len(), 1);
        assert_eq!(page.conversations[0].id, room.id);
        assert!(page.conversations[0].closed);
        client.close();
        f.server.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn signed_batch_collector_keeps_live_facts_separate_and_business_refusal_keeps_peer_usable() {
    use connect_client::{conversations::*, MessageDraft};
    let f = Fixture::new("EdDSA", 8).await;
    let mut subscriber = CanonicalClient::open(f.client.clone()).await.unwrap();
    let cid = create(&f, &mut subscriber, "typed-live-room").await;
    let query = match payload(
        "conversation.subscribe",
        json!({"live":true,"limit":10,"lease_seconds":30}),
    ) {
        Payload::ConversationSubscribe(v) => v,
        _ => unreachable!(),
    };
    let initial = subscriber
        .subscribe_conversation(&cid, query)
        .await
        .unwrap();
    assert_eq!(
        initial.frames.first().unwrap().kind(),
        "conversation.snapshot"
    );
    let request_id = initial.request_id;
    let mut writer = CanonicalClient::open(f.client.clone()).await.unwrap();
    writer
        .send_message(
            "typed-live-send",
            &cid,
            "typed_live_message",
            MessageDraft::text_all("queued live fact"),
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    let query = match payload("conversation.get", json!({})) {
        Payload::ConversationGet(v) => v,
        _ => unreachable!(),
    };
    let snapshot = subscriber.conversation_snapshot(&cid, query).await.unwrap();
    assert!(!snapshot.items.is_empty());
    let mut found = false;
    for _ in 0..16 {
        let frame = subscriber.receive().await.unwrap();
        assert_eq!(frame.original()["in_reply_to"], request_id);
        if frame.kind() == "ack" {
            break;
        }
        if frame.original().to_string().contains("typed_live_message") {
            found = true;
        }
    }
    assert!(found);
    let mut changed = CreateRequest::persistent(vec![]);
    changed.tenant_id = Some("tnt_default".into());
    assert!(
        matches!(writer.create_conversation("typed-live-room",changed).await,Err(ClientError::Application(e)) if e.code=="conflict")
    );
    let outside = f
        .server
        .store
        .create_identity("outside-recipient-scope")
        .unwrap()
        .id;
    let actor = f.server.store.authenticate(&f.client.credential).unwrap();
    let revision = f
        .server
        .store
        .membership_snapshot(&actor.credential_id, &cid)
        .unwrap()
        .revision;
    assert!(matches!(writer.participant_change(
        &cid, "typed-forbidden-admission", Some(revision),
        payload("conversation.participant.add", json!({"identity_id":outside,"kind":"human","role":"guest"})),
    ).await, Err(ClientError::Application(e)) if e.code == "forbidden"));
    assert_eq!(
        f.server
            .store
            .membership_snapshot(&actor.credential_id, &cid)
            .unwrap()
            .revision,
        revision
    );
    assert_eq!(
        writer
            .list_conversations(ListQuery::default())
            .await
            .unwrap()
            .conversations
            .len(),
        1
    );
    writer.close();
    subscriber.close();
    f.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancelled_signed_batch_preserves_original_operation_for_reconnect_replay() {
    use connect_client::{conversations::ListQuery, MessageDraft};
    let f = Fixture::new("EdDSA", 8).await;
    let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
    let cid = create(&f, &mut client, "cancelled-batch-room").await;
    let token = tokio_util::sync::CancellationToken::new();
    token.cancel();
    assert!(matches!(
        client
            .request_with_cancel(
                payload("conversation.list", json!({})),
                Default::default(),
                token
            )
            .await,
        Err(ClientError::Cancelled)
    ));
    assert_eq!(
        client
            .list_conversations(ListQuery::default())
            .await
            .unwrap()
            .conversations
            .len(),
        1
    );
    let lock = rusqlite::Connection::open(&f.config.database).unwrap();
    lock.execute_batch("BEGIN IMMEDIATE").unwrap();
    let cancel = tokio_util::sync::CancellationToken::new();
    let signal = cancel.clone();
    let operation = async {
        client.request_with_cancel(payload("message.send",json!({"msg_id":"cancelled_batch_message","label":"connect-messages","content_type":"text/plain","body":"one logical message","body_encoding":"utf8","to":"all"})),CommandScope{conversation_id:Some(cid.clone()),operation_id:Some("cancelled-batch-send".into()),..Default::default()},cancel).await
    };
    let result = tokio::join!(operation, async {
        tokio::time::sleep(Duration::from_millis(40)).await;
        signal.cancel();
    })
    .0;
    assert!(matches!(result, Err(ClientError::Cancelled)));
    lock.execute_batch("ROLLBACK").unwrap();
    assert!(matches!(
        client
            .send(payload("conversation.list", json!({})), Default::default())
            .await,
        Err(ClientError::Transport)
    ));
    let mut recovered = CanonicalClient::open(f.client.clone()).await.unwrap();
    let first = recovered
        .send_message(
            "cancelled-batch-send",
            &cid,
            "cancelled_batch_message",
            MessageDraft::text_all("one logical message"),
        )
        .await
        .unwrap();
    let replay = recovered
        .send_message(
            "cancelled-batch-send",
            &cid,
            "cancelled_batch_message",
            MessageDraft::text_all("one logical message"),
        )
        .await
        .unwrap();
    assert_eq!(first, replay);
    let query = match payload("message.history", json!({"limit":10})) {
        Payload::MessageHistory(v) => v,
        _ => unreachable!(),
    };
    assert_eq!(
        recovered.history(&cid, query).await.unwrap().messages.len(),
        1
    );
    recovered.close();
    f.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn signed_typed_membership_and_read_calls_verify_receipts_and_current_authority() {
    use connect_client::{
        conversations::CreateRequest, membership::MembershipOutcome, MessageDraft,
    };
    let f = Fixture::with_membership_scope("ES256", 8, true).await;
    let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut request = CreateRequest::persistent(vec![f.peer.identity_id.clone()]);
    request.tenant_id = Some("tnt_default".into());
    let room = client
        .create_conversation("typed-member-room", request)
        .await
        .unwrap();
    let target = f
        .server
        .store
        .create_identity("typed-member-target")
        .unwrap()
        .id;
    let added = client
        .participant_change(
            &room.id,
            "typed-member-add",
            Some(room.revision),
            payload(
                "conversation.participant.add",
                json!({"identity_id":target,"kind":"human","role":"guest"}),
            ),
        )
        .await
        .unwrap();
    let MembershipOutcome::Participant {
        participant,
        revision,
        ..
    } = added
    else {
        panic!("participant")
    };
    let updated = client
        .participant_change(
            &room.id,
            "typed-member-update",
            Some(revision),
            payload(
                "conversation.participant.update",
                json!({"participant_id":participant.participant_id,"role":"reviewer"}),
            ),
        )
        .await
        .unwrap();
    let MembershipOutcome::Participant {
        participant,
        revision,
        ..
    } = updated
    else {
        panic!("participant")
    };
    assert_eq!(participant.role, "reviewer");
    let removed = client
        .participant_change(
            &room.id,
            "typed-member-remove",
            Some(revision),
            payload(
                "conversation.participant.remove",
                json!({"participant_id":participant.participant_id,"reason":"typed test removal"}),
            ),
        )
        .await
        .unwrap();
    let MembershipOutcome::Participant { participant, .. } = removed else {
        panic!("participant")
    };
    assert!(!participant.active);
    assert_eq!(
        participant.removal_reason.as_deref(),
        Some("typed test removal")
    );
    // The current public control facade sends as the other enrolled Identity.
    let Outcome::Accepted { message_id, .. } = f
        .server
        .service
        .execute(
            f.peer.credential_id.clone(),
            Operation::Send {
                request_id: "typed-receipt-source".into(),
                conversation_id: room.id.clone(),
                message: MessageDraft::text_all("typed receipt subject"),
            },
        )
        .await
        .unwrap()
    else {
        panic!("message")
    };
    let delivered = client
        .report_read(
            "typed-read-delivery",
            &room.id,
            &message_id,
            chrono::Utc::now(),
        )
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(delivered.recorded).unwrap(),
        json!("delivered")
    );
    f.server
        .service
        .execute(
            f.server
                .store
                .authenticate(&f.client.credential)
                .unwrap()
                .credential_id,
            Operation::ReceiptPreferencesSet {
                request_id: "typed-read-opt-in".into(),
                conversation_id: room.id.clone(),
                delivery: true,
                read: true,
            },
        )
        .await
        .unwrap();
    let recorded_at = chrono::Utc::now();
    let read = client
        .report_read("typed-read-explicit", &room.id, &message_id, recorded_at)
        .await
        .unwrap();
    let replay = client
        .report_read("typed-read-explicit", &room.id, &message_id, recorded_at)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&read).unwrap(),
        serde_json::to_value(&replay).unwrap()
    );
    assert_eq!(serde_json::to_value(read.recorded).unwrap(), json!("read"));
    let absent = client
        .operation_get(
            "typed-never-submitted",
            CommandScope {
                conversation_id: Some(room.id),
                ..Default::default()
            },
        )
        .await;
    assert!(matches!(absent, Err(ClientError::Application(e)) if e.code == "forbidden"));
    client.close();
    f.server.shutdown().await.unwrap();
}

#[path = "support/canonical_http_cases.rs"]
mod canonical_http_cases;

#[path = "support/canonical_session_control.rs"]
mod general_session_control;

#[path = "support/canonical_native_connections.rs"]
mod canonical_native_connections;

#[path = "support/canonical_auxiliary_clauses.rs"]
mod auxiliary_clauses;

#[path = "support/canonical_session_cancellation.rs"]
mod caller_cancellations;

#[path = "support/sdk_handles.rs"]
mod sdk_handles;
