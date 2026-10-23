//! Real production native adapter, signed SDK confirmation and durable readiness.
#[path = "canonical_native_connections/capabilities.rs"]
mod capabilities;
#[path = "canonical_native_connections/capability_quic.rs"]
mod capability_quic;
#[path = "canonical_native_connections/cleanup.rs"]
mod cleanup;
#[path = "canonical_native_connections/codecs.rs"]
mod codecs;
#[path = "canonical_native_connections/consultation.rs"]
mod consultation;
#[path = "canonical_native_connections/data.rs"]
mod data;
#[path = "canonical_native_connections/dtmf.rs"]
mod dtmf;
#[path = "canonical_native_connections/dtmf_diagnostics.rs"]
mod dtmf_diagnostics;
#[path = "canonical_native_connections/future.rs"]
mod future;
#[path = "canonical_native_connections/gates.rs"]
mod gates;
#[path = "canonical_native_connections/journey.rs"]
mod journey;
#[path = "canonical_native_connections/lifecycle.rs"]
mod lifecycle;
#[path = "canonical_native_connections/negotiation.rs"]
mod negotiation;
#[path = "canonical_native_connections/participants.rs"]
mod participants;
#[path = "canonical_native_connections/public_assistant.rs"]
mod public_assistant;
#[path = "canonical_native_connections/quality.rs"]
mod quality;
#[path = "canonical_native_connections/selector_policy.rs"]
mod selector_policy;
#[path = "canonical_native_connections/session_activation.rs"]
mod session_activation;
#[path = "canonical_native_connections/session_codecs.rs"]
mod session_codecs;
#[path = "canonical_native_connections/solo.rs"]
mod solo;
#[path = "canonical_native_connections/step_up.rs"]
mod step_up;
use super::*;

// Control-focused cases consume legitimate asynchronous lifecycle hints. The
// lifecycle module uses raw receive() and verifies their contents and authority.
pub(super) trait ControlReception {
    async fn receive_control(&mut self) -> connect_client::Result<Envelope>;
}
impl ControlReception for CanonicalClient {
    async fn receive_control(&mut self) -> connect_client::Result<Envelope> {
        let until = tokio::time::Instant::now() + Duration::from_secs(8);
        for _ in 0..128 {
            let frame = self.receive().await?;
            if !matches!(
                frame.kind(),
                "session.started"
                    | "recording.vcon-ready"
                    | "session.ended"
                    | "session.participant.joined"
                    | "session.participant.left"
                    | "connection.ready"
                    | "stream.opened"
                    | "stream.closed"
                    | "connection.quality"
            ) {
                return Ok(frame);
            }
            if tokio::time::Instant::now() >= until {
                return Err(ClientError::Timeout);
            }
        }
        Err(ClientError::Protocol)
    }
}
use connect_client::{
    canonical::CanonicalHttpClient,
    session_control::{Action, Request},
};
async fn ready_endpoint(f: &Fixture) {
    ready_endpoint_with_capabilities(f, vec!["audio".into(), "video".into()]).await;
}
async fn ready_endpoint_with_capabilities(f: &Fixture, capabilities: Vec<String>) {
    static LOGS: std::sync::Once = std::sync::Once::new();
    LOGS.call_once(|| {
        if dtmf_diagnostics::install() {
            return;
        }
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "connect_service::canonical_native=debug".into()),
            )
            .with_test_writer()
            .try_init();
    });
    let admin = f.server.store.authenticate(&f.admin).unwrap().credential_id;
    let outcome = f
        .server
        .store
        .execute_provisioning(
            &admin,
            ProvisioningCommand::Endpoint {
                operation_id: "native-audio-endpoint".into(),
                identity_id: f.client.identity_id.clone(),
                device: DeviceSpec {
                    id: f.client.device_id.clone(),
                    kind: f.client.device_kind.clone(),
                    platform: f.client.device_platform.clone(),
                },
                transport: EndpointTransport::Quic,
                address: format!("quic://localhost:{}/native-audio", f.server.address.port()),
                capabilities,
                secret_ref: None,
            },
        )
        .unwrap();
    let ProvisioningOutcome::Endpoint { endpoint_id, .. } = outcome else {
        panic!()
    };
    f.server
        .store
        .execute_provisioning(
            &admin,
            ProvisioningCommand::VerifyEndpoint {
                operation_id: "native-audio-verified".into(),
                endpoint_id,
                expected_revision: 0,
                evidence_ref: "local-designated-runtime-test".into(),
            },
        )
        .unwrap();
}
async fn session(f: &Fixture, client: &mut CanonicalClient) -> String {
    session_with_media(f, client, "voice", json!({"audio":["g.711-mu"]})).await
}
async fn session_with_media(
    f: &Fixture,
    client: &mut CanonicalClient,
    medium: &str,
    capabilities: serde_json::Value,
) -> String {
    session_with_acceptance(f, client, medium, capabilities.clone(), capabilities).await
}
async fn session_with_acceptance(
    f: &Fixture,
    client: &mut CanonicalClient,
    medium: &str,
    capabilities: Value,
    accepted: Value,
) -> String {
    let mut input =
        connect_client::conversations::CreateRequest::persistent(vec![f.peer.identity_id.clone()]);
    input.tenant_id = Some("tnt_default".into());
    let room = client
        .create_conversation("native-room", input)
        .await
        .unwrap();
    let peer = room
        .participants
        .iter()
        .find(|p| p.identity_id == f.peer.identity_id)
        .unwrap()
        .participant_id
        .clone();
    client
        .session_command(Request {
            operation_id: "native-invite".into(),
            conversation_id: room.id.clone(),
            session_id: "native-session".into(),
            by_participant: None,
            expected_revision: None,
            action: Action::Invite {
                participants: vec![peer],
                medium: medium.into(),
                intent: "conference".into(),
                capabilities: capabilities.clone(),
            },
        })
        .await
        .unwrap();
    let key = f
        .server
        .store
        .issue(
            &f.peer.identity_id,
            Scope::collaboration(vec!["*".into()]),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let mut config = canonical_http_config(f);
    config.credential = key.token;
    let mut peer = CanonicalHttpClient::open(config).await.unwrap();
    peer.session_command(Request {
        operation_id: "native-accept".into(),
        conversation_id: room.id.clone(),
        session_id: "native-session".into(),
        by_participant: None,
        expected_revision: Some(0),
        action: Action::Accept {
            capabilities: accepted,
        },
    })
    .await
    .unwrap();
    room.id
}
async fn begin_negotiation(
    client: &mut CanonicalClient,
    cid: &str,
    operation: &str,
    connid: &str,
) -> (Envelope, Envelope, CommandScope) {
    begin_named_negotiation(client, cid, operation, connid, "voice").await
}
async fn begin_named_negotiation(
    client: &mut CanonicalClient,
    cid: &str,
    operation: &str,
    connid: &str,
    logical: &str,
) -> (Envelope, Envelope, CommandScope) {
    let scope = CommandScope {
        conversation_id: Some(cid.into()),
        session_id: Some("native-session".into()),
        connection_id: Some(connid.into()),
        operation_id: Some(operation.into()),
        expected_revision: Some(0),
    };
    client.send(payload("connection.offer",json!({"substrate":"quic","capabilities":{"audio_codecs":[{"name":"pcmu","params":{"sample_rate":8000,"channels":1}}],"max_streams_per_connection":8,"identity_assurance_offered":"identified"},"streams_offered":[{"id":logical,"kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}]})),scope.clone()).await.unwrap();
    let answer = client.receive_control().await.unwrap();
    assert_eq!(
        answer.kind(),
        "connection.answer",
        "{:?}",
        answer.original()
    );
    let callback = client.receive_control().await.unwrap();
    assert_eq!(
        callback.kind(),
        "connection.offer",
        "{:?}",
        callback.original()
    );
    (answer, callback, scope)
}
async fn negotiate(
    client: &mut CanonicalClient,
    cid: &str,
    operation: &str,
    connid: &str,
) -> Envelope {
    negotiate_named(client, cid, operation, connid, "voice").await
}
async fn negotiate_named(
    client: &mut CanonicalClient,
    cid: &str,
    operation: &str,
    connid: &str,
    logical: &str,
) -> Envelope {
    let (answer, callback, scope) =
        begin_named_negotiation(client, cid, operation, connid, logical).await;
    let p = &callback.original()["payload"];
    let streams=p["substrate_setup"]["streams"].as_array().unwrap().iter().map(|s|json!({"id":s["strm_id"],"kind":s["kind"],"direction":match s["direction"].as_str().unwrap(){"sendonly"=>"recvonly","recvonly"=>"sendonly",_=>"sendrecv"},"codec":s["codec"]})).collect::<Vec<_>>();
    let response=serde_json::from_value(json!({"by_participant":p["by_participant"],"substrate":p["substrate"],"capabilities":{"audio_codecs":[{"name":"pcmu","params":{"sample_rate":8000,"channels":1}}],"max_streams_per_connection":8,"identity_assurance_offered":"identified"},"streams_answered":streams,"substrate_setup":p["substrate_setup"]})).unwrap();
    client
        .answer_connection_offer(&callback, response, &scope)
        .await
        .unwrap();
    answer
}
#[tokio::test]
async fn production_native_offer_confirms_real_resources_admits_and_retires_only_its_slot_on_eof() {
    let f = Fixture::with_membership_scope("EdDSA", 12, true).await;
    ready_endpoint(&f).await;
    let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
    let cid = session(&f, &mut client).await;
    let mut http = CanonicalHttpClient::open(canonical_http_config(&f))
        .await
        .unwrap();
    let baseline = http
        .conversation_snapshot(&cid, serde_json::from_value(json!({})).unwrap())
        .await
        .unwrap();
    let answer = negotiate(
        &mut client,
        &cid,
        "actual-native-offer",
        "native-connection",
    )
    .await;
    assert!(
        answer.original()["payload"]["substrate_setup"]["streams"][0]["stream_local_id"]
            .as_u64()
            .unwrap()
            > 0
    );
    let scope = CommandScope {
        conversation_id: Some(cid.clone()),
        session_id: Some("native-session".into()),
        ..Default::default()
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let record = loop {
        let record = http
            .operation_get("actual-native-offer", scope.clone())
            .await
            .unwrap();
        if serde_json::to_value(record.state).unwrap() == json!("completed") {
            break record;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "native status {:?}",
            serde_json::to_value(&record).unwrap()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let durable = f
        .server
        .store
        .execute_operation_action(
            &f.server
                .store
                .authenticate(&f.client.credential)
                .unwrap()
                .credential_id,
            connect_protocol::transitions::OperationAction::Get {
                operation_id: "actual-native-offer".into(),
            },
        )
        .unwrap();
    let events = f
        .server
        .store
        .conversation_events(
            &f.server
                .store
                .authenticate(&f.client.credential)
                .unwrap()
                .credential_id,
            &cid,
            connect_protocol::events::EventQuery {
                after: Some(baseline.cursor.clone()),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(events.events.iter().any(|e|matches!(&e.data,connect_protocol::events::JournalData::ConnectionReady(c) if c.connection_id=="native-connection" && c.state=="ready")));
    let batch = http
        .subscribe_conversation(
            &cid,
            serde_json::from_value(json!({"after":baseline.cursor,"live":false})).unwrap(),
        )
        .await
        .unwrap();
    assert!(batch.frames.iter().any(|e| e.kind() == "conversation.event"
        && e.original()["payload"]["data"]["data"]["kind"] == "connection_ready"));
    if let Some(directory) = std::env::var_os("CONNECT_GATE6_NATIVE_EVENT_SAMPLES") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("native-ready.json"),
            serde_json::to_vec_pretty(&Outcome::ConversationEvents(events)).unwrap(),
        )
        .unwrap();
    }
    assert_eq!(durable.effects.len(), 1);
    assert_eq!(
        serde_json::to_value(&durable.effects[0].state).unwrap(),
        json!("confirmed")
    );
    assert_eq!(
        f.server
            .service
            .canonical_runtime_snapshot()
            .unwrap()
            .connections,
        1
    );
    let page = http
        .conversation_snapshot(&cid, serde_json::from_value(json!({})).unwrap())
        .await
        .unwrap();
    let rows = serde_json::to_value(page).unwrap();
    assert!(rows["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["kind"] == "connection"
            && item["data"]["connection_id"] == "native-connection"
            && item["data"]["state"] == "ready"));
    client.close();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let page = http
            .conversation_snapshot(&cid, serde_json::from_value(json!({})).unwrap())
            .await
            .unwrap();
        let rows = serde_json::to_value(page).unwrap();
        if rows["items"].as_array().unwrap().iter().any(|item| {
            item["kind"] == "connection"
                && item["data"]["connection_id"] == "native-connection"
                && item["data"]["state"] == "retired"
        }) {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        f.server
            .service
            .canonical_runtime_snapshot()
            .unwrap()
            .connections,
        0
    );
    assert_eq!(
        http.operation_get("actual-native-offer", scope)
            .await
            .unwrap()
            .operation_id,
        record.operation_id
    );
    f.server.shutdown().await.unwrap();
}

fn party(f: &Fixture, identity: &str, index: usize) -> CanonicalClientConfig {
    party_with_streams(f, identity, index, false)
}
fn party_with_streams(
    f: &Fixture,
    identity: &str,
    index: usize,
    streams: bool,
) -> CanonicalClientConfig {
    party_with_capabilities(f, identity, index, streams, vec!["audio".into()])
}
fn party_with_capabilities(
    f: &Fixture,
    identity: &str,
    index: usize,
    streams: bool,
    capabilities: Vec<String>,
) -> CanonicalClientConfig {
    let mut scope = Scope::collaboration(vec!["*".into()]);
    if streams {
        scope
            .operations
            .extend(["stream.subscribe".into(), "stream.unsubscribe".into()]);
    }
    party_with_scope(f, identity, index, capabilities, scope)
}
pub(super) fn party_with_scope(
    f: &Fixture,
    identity: &str,
    index: usize,
    capabilities: Vec<String>,
    scope: Scope,
) -> CanonicalClientConfig {
    let admin = f.server.store.authenticate(&f.admin).unwrap().credential_id;
    let credential = f
        .server
        .store
        .issue(identity, scope, chrono::Utc::now().timestamp() + 600)
        .unwrap();
    let mut config = f.client.clone();
    config.identity_id = identity.into();
    config.credential = credential.token;
    config.device_id = format!("native-party-device-{index}");
    config.signing_key_id = format!("native-party-key-{index}");
    config.signing_private_key = f.temp.path().join(format!("native-party-{index}.der"));
    let public = signing(&config.signing_private_key, &config.signing_algorithm);
    let ProvisioningOutcome::Endpoint { endpoint_id, .. } = f
        .server
        .store
        .execute_provisioning(
            &admin,
            ProvisioningCommand::Endpoint {
                operation_id: format!("native-party-endpoint-{index}"),
                identity_id: identity.into(),
                device: DeviceSpec {
                    id: config.device_id.clone(),
                    kind: config.device_kind.clone(),
                    platform: config.device_platform.clone(),
                },
                transport: EndpointTransport::Quic,
                address: format!(
                    "quic://localhost:{}/native-party-{index}",
                    f.server.address.port()
                ),
                capabilities,
                secret_ref: None,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    f.server
        .store
        .execute_provisioning(
            &admin,
            ProvisioningCommand::VerifyEndpoint {
                operation_id: format!("native-party-verify-{index}"),
                endpoint_id,
                expected_revision: 0,
                evidence_ref: "local-designated-native-test".into(),
            },
        )
        .unwrap();
    f.server
        .store
        .execute_provisioning(
            &admin,
            ProvisioningCommand::SigningKey {
                operation_id: format!("native-party-enroll-{index}"),
                identity_id: identity.into(),
                credential_id: credential.id,
                device_id: config.device_id.clone(),
                key_id: config.signing_key_id.clone(),
                algorithm: if config.signing_algorithm == "ES256" {
                    SigningAlgorithm::ES256
                } else {
                    SigningAlgorithm::EdDSA
                },
                public_key: URL_SAFE_NO_PAD.encode(public),
                not_before: chrono::Utc::now().timestamp() - 1,
                expires_at: chrono::Utc::now().timestamp() + 600,
                expected_revision: None,
            },
        )
        .unwrap();
    config
}
async fn wait_slots(f: &Fixture, expected: usize) {
    let end = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let actual = f
            .server
            .service
            .canonical_runtime_snapshot()
            .unwrap()
            .connections;
        if actual == expected {
            break;
        }
        assert!(
            tokio::time::Instant::now() < end,
            "actual slot count {actual}, expected {expected}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
async fn assert_audio_reaches_other_participant(
    clients: &mut [CanonicalClient],
    source_handle: u16,
    target_handles: &[u16],
    sequence: u32,
) {
    use rvoip_uctp::substrate::{
        pack_rtp_datagram, unpack_rtp_datagram, RtpDatagram, RtpMediaPayload,
    };
    let (source, others) = clients.split_at_mut(1);
    let sender = async {
        for seq in sequence..sequence + 60 {
            let packet = RtpDatagram {
                flags: 0,
                stream_local_id: source_handle,
                seq,
                rtp: RtpMediaPayload {
                    payload: (0..160)
                        .map(|n| if n % 16 < 8 { 0x80 } else { 0x00 })
                        .collect::<Vec<u8>>()
                        .into(),
                    payload_type: 0,
                    sequence_number: seq as u16,
                    timestamp: seq.wrapping_mul(160),
                    ssrc: 0x4736_0001,
                },
            };
            source[0]
                .send_native_rtp_datagram(&pack_rtp_datagram(&packet).unwrap())
                .unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    let receiver = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let bytes = others[0].receive_native_rtp_datagram().await.unwrap();
            let packet = unpack_rtp_datagram(&bytes).unwrap();
            if packet.rtp.payload_type != 0 {
                continue;
            }
            assert!(target_handles.contains(&packet.stream_local_id));
            if packet.rtp.payload.iter().any(|v| *v != 0xff && *v != 0x7f) {
                break;
            }
        }
    });
    let (_, result) = tokio::join!(sender, receiver);
    result.expect(
        "authorized audio must reach the other participant through the actual managed mixer",
    );
}
#[tokio::test]
async fn production_four_kinds_six_native_slots_preserve_other_peers_after_one_leaves() {
    six_native_slots(false, false).await;
}
#[tokio::test]
async fn production_native_audio_survives_another_participant_leaving() {
    six_native_slots(true, false).await;
}
#[tokio::test]
async fn production_native_membership_removal_retires_only_that_party_and_preserves_other_audio() {
    six_native_slots(true, true).await;
}
async fn six_native_slots(check_audio: bool, remove_membership: bool) {
    let f = Fixture::with_membership_scope("EdDSA", 16, true).await;
    ready_endpoint(&f).await;
    let admin = f.server.store.authenticate(&f.admin).unwrap().credential_id;
    let mut identities = vec![f.client.identity_id.clone(), f.peer.identity_id.clone()];
    for (index, kind) in [IdentityKind::External, IdentityKind::System]
        .into_iter()
        .enumerate()
    {
        let ProvisioningOutcome::Identity { identity_id, .. } = f
            .server
            .store
            .execute_provisioning(
                &admin,
                ProvisioningCommand::Identity {
                    operation_id: format!("native-kind-{index}"),
                    alias: format!("native-kind-{index}"),
                    kind,
                    display_name: None,
                },
            )
            .unwrap()
        else {
            panic!()
        };
        identities.push(identity_id);
    }
    let mut configs = vec![f.client.clone()];
    for (index, identity) in identities.iter().enumerate().skip(1) {
        configs.push(party_with_streams(&f, identity, index, check_audio))
    }
    let mut clients = Vec::new();
    for config in &configs {
        clients.push(CanonicalClient::open(config.clone()).await.unwrap())
    }
    let mut input =
        connect_client::conversations::CreateRequest::persistent(identities[1..].to_vec());
    input.tenant_id = Some("tnt_default".into());
    let room = clients[0]
        .create_conversation("native-six-room", input)
        .await
        .unwrap();
    let pids = identities
        .iter()
        .map(|identity| {
            room.participants
                .iter()
                .find(|p| &p.identity_id == identity)
                .unwrap()
                .participant_id
                .clone()
        })
        .collect::<Vec<_>>();
    clients[0]
        .session_command(Request {
            operation_id: "native-six-invite".into(),
            conversation_id: room.id.clone(),
            session_id: "native-session".into(),
            by_participant: None,
            expected_revision: None,
            action: Action::Invite {
                participants: pids[1..].to_vec(),
                medium: "voice".into(),
                intent: "conference".into(),
                capabilities: json!({"audio":["g.711-mu"]}),
            },
        })
        .await
        .unwrap();
    for (index, client) in clients.iter_mut().enumerate().skip(1) {
        client
            .session_command(Request {
                operation_id: format!("native-six-accept-{index}"),
                conversation_id: room.id.clone(),
                session_id: "native-session".into(),
                by_participant: None,
                expected_revision: Some(index as u64 - 1),
                action: Action::Accept {
                    capabilities: json!({"audio":["g.711-mu"]}),
                },
            })
            .await
            .unwrap();
    }
    let mut setups = Vec::new();
    for (index, actor) in [0, 0, 1, 1, 2, 3].into_iter().enumerate() {
        let answer = negotiate_named(
            &mut clients[actor],
            &room.id,
            &format!("native-six-offer-{index}"),
            &format!("native-six-slot-{index}"),
            &format!("voice-{index}"),
        )
        .await;
        setups.push(answer);
        // Poll the authoritative operation rather than assuming a signed answer
        // is ready or that admission already committed.
        let mut http_config = canonical_http_config(&f);
        http_config.credential = configs[actor].credential.clone();
        let mut http = CanonicalHttpClient::open(http_config).await.unwrap();
        let end = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let status = http
                .operation_get(
                    &format!("native-six-offer-{index}"),
                    CommandScope {
                        conversation_id: Some(room.id.clone()),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            if status.state == rvoip_uctp::v2::payloads::OperationState::Completed {
                break;
            };
            assert!(
                tokio::time::Instant::now() < end,
                "native slot {index} status {:?}",
                status.state
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    wait_slots(&f, 6).await;
    assert_eq!(
        f.server
            .service
            .canonical_runtime_snapshot()
            .unwrap()
            .media_readers,
        4
    );
    let handles = setups
        .iter()
        .take(2)
        .map(|s| {
            s.original()["payload"]["substrate_setup"]["streams"][0]["stream_local_id"]
                .as_u64()
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_ne!(handles[0], handles[1]);
    let target_handles = setups[2..4]
        .iter()
        .map(|s| {
            s.original()["payload"]["substrate_setup"]["streams"][0]["stream_local_id"]
                .as_u64()
                .unwrap() as u16
        })
        .collect::<Vec<_>>();
    if check_audio {
        assert!(stream_control(
            &mut clients[1],
            &room.id,
            "native-six-slot-2",
            "six-select-host",
            1,
            "stream.subscribe",
            json!({"subscriptions":[{"from_participant":pids[0],"kinds":["audio"]}]})
        )
        .await
        .is_none());
        assert_audio_reaches_other_participant(&mut clients, handles[0] as u16, &target_handles, 1)
            .await;
    }
    assert!(clients[1]
        .session_command(Request {
            operation_id: "native-denied-camera".into(),
            conversation_id: room.id.clone(),
            session_id: "native-session".into(),
            by_participant: None,
            expected_revision: Some(3),
            action: Action::Upgrade {
                media: vec!["video".into()]
            },
        })
        .await
        .is_err());
    wait_slots(&f, 6).await;
    if remove_membership {
        let mut http = CanonicalHttpClient::open(canonical_http_config(&f))
            .await
            .unwrap();
        let revision = http
            .conversation_snapshot(&room.id, serde_json::from_value(json!({})).unwrap())
            .await
            .unwrap()
            .revision;
        let removed = clients[0]
            .participant_change(
                &room.id,
                "native-remove-external",
                Some(revision),
                payload(
                    "conversation.participant.remove",
                    json!({"participant_id":pids[2],"reason":"remove live party"}),
                ),
            )
            .await
            .unwrap();
        assert!(
            matches!(removed, connect_client::membership::MembershipOutcome::Participant{participant,..} if !participant.active)
        );
    } else {
        clients[2]
            .session_command(Request {
                operation_id: "native-external-leave".into(),
                conversation_id: room.id.clone(),
                session_id: "native-session".into(),
                by_participant: None,
                expected_revision: Some(3),
                action: Action::Leave {
                    participant_id: pids[2].clone(),
                },
            })
            .await
            .unwrap();
    }
    wait_slots(&f, 5).await;
    if check_audio {
        assert_audio_reaches_other_participant(
            &mut clients,
            handles[0] as u16,
            &target_handles,
            100,
        )
        .await;
    }
    let mut http = CanonicalHttpClient::open(canonical_http_config(&f))
        .await
        .unwrap();
    let page = http
        .conversation_snapshot(&room.id, serde_json::from_value(json!({})).unwrap())
        .await
        .unwrap();
    let page = serde_json::to_value(page).unwrap();
    assert_eq!(
        page["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v["kind"] == "connection" && v["data"]["state"] == "ready")
            .count(),
        5
    );
    if remove_membership {
        assert!(clients[2]
            .send_message(
                "native-removed-chat",
                &room.id,
                "native-removed-chat-message",
                connect_client::messages::MessageDraft::text_all("removed member cannot publish")
            )
            .await
            .is_err());
        assert!(clients[2]
            .session_command(Request {
                operation_id: "native-removed-rejoin".into(),
                conversation_id: room.id.clone(),
                session_id: "native-session".into(),
                by_participant: None,
                expected_revision: Some(3),
                action: Action::Accept {
                    capabilities: json!({"audio":["g.711-mu"]})
                }
            })
            .await
            .is_err());
    } else {
        clients[2]
            .send_message(
                "native-leaver-chat",
                &room.id,
                "native-leaver-chat-message",
                connect_client::messages::MessageDraft::text_all(
                    "Conversation remains usable after media leave",
                ),
            )
            .await
            .unwrap();
    }
    assert!(clients[0]
        .close_conversation("native-close-while-live", &room.id, Default::default())
        .await
        .is_err());
    wait_slots(&f, 5).await;
    clients[0]
        .session_command(Request {
            operation_id: "native-end-before-close".into(),
            conversation_id: room.id.clone(),
            session_id: "native-session".into(),
            by_participant: None,
            expected_revision: Some(if remove_membership { 3 } else { 4 }),
            action: Action::End {
                reason_code: 0,
                reason: "explicit Session end before Conversation close".into(),
            },
        })
        .await
        .unwrap();
    wait_slots(&f, 0).await;
    clients[0]
        .close_conversation(
            "native-close-conversation",
            &room.id,
            connect_client::conversations::CloseOptions {
                expected_revision: Some(
                    http.conversation_snapshot(
                        &room.id,
                        serde_json::from_value(json!({})).unwrap(),
                    )
                    .await
                    .unwrap()
                    .revision,
                ),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    wait_slots(&f, 0).await;
    let page = serde_json::to_value(
        http.conversation_snapshot(&room.id, serde_json::from_value(json!({})).unwrap())
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(page["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| v["kind"] == "connection")
        .all(|v| v["data"]["state"] == "retired"));
    for client in &clients {
        client.close()
    }

    f.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn production_restart_retires_confirmed_and_unconfirmed_native_slots_without_resubmission() {
    let mut f = Fixture::with_membership_scope("EdDSA", 12, true).await;
    ready_endpoint(&f).await;
    let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
    let cid = session(&f, &mut client).await;
    negotiate(&mut client, &cid, "restart-ready", "restart-ready-slot").await;
    let mut http = CanonicalHttpClient::open(canonical_http_config(&f))
        .await
        .unwrap();
    let scope = CommandScope {
        conversation_id: Some(cid.clone()),
        ..Default::default()
    };
    let end = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if http
            .operation_get("restart-ready", scope.clone())
            .await
            .unwrap()
            .state
            == rvoip_uctp::v2::payloads::OperationState::Completed
        {
            break;
        }
        assert!(tokio::time::Instant::now() < end);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // Obtain actual allocated native parameters but withhold the signed peer
    // confirmation. This attempt must never become ready or be submitted twice.
    begin_negotiation(&mut client, &cid, "restart-pending", "restart-pending-slot").await;
    let credential = f
        .server
        .store
        .authenticate(&f.client.credential)
        .unwrap()
        .credential_id;
    let record = |op: &str| {
        f.server
            .store
            .execute_operation_action(
                &credential,
                connect_protocol::transitions::OperationAction::Get {
                    operation_id: op.into(),
                },
            )
            .unwrap()
    };
    let before_ready = record("restart-ready");
    let before_pending = record("restart-pending");
    assert_eq!(before_pending.effects.len(), 1);
    assert_eq!(
        before_pending.state,
        connect_protocol::transitions::OperationState::Preparing
    );
    let epoch = f.server.store.runtime_epoch().to_owned();
    // Abrupt owner drop leaves durable native attempts for the successor;
    // no graceful drain is allowed to stand in for restart reconciliation.
    drop(f.server);
    client.close();
    let end = tokio::time::Instant::now() + Duration::from_secs(5);
    f.server = loop {
        match RunningCanonicalServer::start(f.config.clone()).await {
            Ok(server) => break server,
            Err(error)
                if error
                    .downcast_ref::<connect_protocol::Error>()
                    .is_some_and(|e| e.code == "installation-busy")
                    && tokio::time::Instant::now() < end =>
            {
                tokio::time::sleep(Duration::from_millis(20)).await
            }
            Err(error) => panic!("successor native runtime could not start: {error}"),
        }
    };
    assert_ne!(epoch, f.server.store.runtime_epoch());
    f.client.endpoint = f.server.address;
    let credential = f
        .server
        .store
        .authenticate(&f.client.credential)
        .unwrap()
        .credential_id;
    for (before, expected) in [
        (
            &before_ready,
            connect_protocol::transitions::OperationState::Completed,
        ),
        (
            &before_pending,
            connect_protocol::transitions::OperationState::Failed,
        ),
    ] {
        let after = f
            .server
            .store
            .execute_operation_action(
                &credential,
                connect_protocol::transitions::OperationAction::Get {
                    operation_id: before.operation_id.clone(),
                },
            )
            .unwrap();
        assert_eq!(after.record_id, before.record_id);
        assert_eq!(after.state, expected);
        assert_eq!(after.effects.len(), 1);
        assert_eq!(after.effects[0].id, before.effects[0].id);
    }
    let mut http = CanonicalHttpClient::open(canonical_http_config(&f))
        .await
        .unwrap();
    let page = serde_json::to_value(
        http.conversation_snapshot(&cid, serde_json::from_value(json!({})).unwrap())
            .await
            .unwrap(),
    )
    .unwrap();
    let slots = page["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| v["kind"] == "connection")
        .collect::<Vec<_>>();
    assert_eq!(slots.len(), 2);
    assert!(slots.iter().all(|v| v["data"]["state"] == "retired"));
    wait_slots(&f, 0).await;
    let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
    // A fresh signature preserving the original logical intent returns its old
    // status, without opening a new native exchange or changing the effect ID.
    for (op, slot) in [
        ("restart-ready", "restart-ready-slot"),
        ("restart-pending", "restart-pending-slot"),
    ] {
        let scope = CommandScope {
            conversation_id: Some(cid.clone()),
            session_id: Some("native-session".into()),
            connection_id: Some(slot.into()),
            operation_id: Some(op.into()),
            expected_revision: Some(0),
        };
        client.send(payload("connection.offer",json!({"substrate":"quic","capabilities":{"audio_codecs":[{"name":"pcmu","params":{"sample_rate":8000,"channels":1}}],"max_streams_per_connection":8,"identity_assurance_offered":"identified"},"streams_offered":[{"id":"voice","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}]})),scope).await.unwrap();
        let response = client.receive_control().await.unwrap();
        assert_eq!(
            response.kind(),
            "operation.updated",
            "{:?}",
            response.original()
        );
        assert_eq!(client.receive_control().await.unwrap().kind(), "ack");
    }
    wait_slots(&f, 0).await;
    client.close();
    f.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn production_signed_parameter_mismatch_keeps_preparing_until_exact_confirmation_recovers() {
    let f = Fixture::with_membership_scope("EdDSA", 12, true).await;
    ready_endpoint(&f).await;
    let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
    let cid = session(&f, &mut client).await;
    negotiate(&mut client, &cid, "good-native", "good-slot").await;
    wait_slots(&f, 1).await;
    let (_, callback, scope) = begin_negotiation(&mut client, &cid, "bad-native", "bad-slot").await;
    let p = &callback.original()["payload"];
    let streams=p["substrate_setup"]["streams"].as_array().unwrap().iter().map(|s|json!({"id":s["strm_id"],"kind":s["kind"],"direction":"sendrecv","codec":{"name":"pcma","payload_type":8,"params":{"sample_rate":8000,"channels":1}}})).collect::<Vec<_>>();
    let response=serde_json::from_value(json!({"by_participant":p["by_participant"],"substrate":p["substrate"],"capabilities":{"audio_codecs":[{"name":"pcmu","params":{"sample_rate":8000,"channels":1}}],"max_streams_per_connection":8,"identity_assurance_offered":"identified"},"streams_answered":streams,"substrate_setup":p["substrate_setup"]})).unwrap();
    client
        .answer_connection_offer(&callback, response, &scope)
        .await
        .unwrap();
    let error = client.receive_control().await.unwrap();
    assert_eq!(error.kind(), "error");
    let mut http = CanonicalHttpClient::open(canonical_http_config(&f))
        .await
        .unwrap();
    let status_scope = CommandScope {
        conversation_id: Some(cid.clone()),
        ..Default::default()
    };
    assert_eq!(
        http.operation_get("bad-native", status_scope.clone())
            .await
            .unwrap()
            .state,
        rvoip_uctp::v2::payloads::OperationState::Preparing
    );
    wait_slots(&f, 2).await;
    let page = serde_json::to_value(
        http.conversation_snapshot(&cid, serde_json::from_value(json!({})).unwrap())
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        page["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v["kind"] == "connection" && v["data"]["state"] == "ready")
            .count(),
        1
    );
    let streams=p["substrate_setup"]["streams"].as_array().unwrap().iter().map(|s|json!({"id":s["strm_id"],"kind":s["kind"],"direction":"sendrecv","codec":s["codec"]})).collect::<Vec<_>>();
    let response=serde_json::from_value(json!({"by_participant":p["by_participant"],"substrate":p["substrate"],"capabilities":{"audio_codecs":[{"name":"pcmu","params":{"sample_rate":8000,"channels":1}}],"max_streams_per_connection":8,"identity_assurance_offered":"identified"},"streams_answered":streams,"substrate_setup":p["substrate_setup"]})).unwrap();
    client
        .answer_connection_offer(&callback, response, &scope)
        .await
        .unwrap();
    let end = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if http
            .operation_get("bad-native", status_scope.clone())
            .await
            .unwrap()
            .state
            == rvoip_uctp::v2::payloads::OperationState::Completed
        {
            break;
        }
        assert!(tokio::time::Instant::now() < end);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    wait_slots(&f, 2).await;
    let credential = f
        .server
        .store
        .authenticate(&f.client.credential)
        .unwrap()
        .credential_id;
    let record = f
        .server
        .store
        .execute_operation_action(
            &credential,
            connect_protocol::transitions::OperationAction::Get {
                operation_id: "bad-native".into(),
            },
        )
        .unwrap();
    assert_eq!(record.effects.len(), 1);
    let page = serde_json::to_value(
        http.conversation_snapshot(&cid, serde_json::from_value(json!({})).unwrap())
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        page["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v["kind"] == "connection" && v["data"]["state"] == "ready")
            .count(),
        2
    );
    client.close();
    wait_slots(&f, 0).await;
    f.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn production_cancelled_native_confirmation_cannot_admit_or_mark_ready_and_preserves_other_slot(
) {
    let f = Fixture::with_membership_scope("EdDSA", 12, true).await;
    ready_endpoint(&f).await;
    let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
    let cid = session(&f, &mut client).await;
    negotiate(&mut client, &cid, "cancel-good", "cancel-good-slot").await;
    let mut http = CanonicalHttpClient::open(canonical_http_config(&f))
        .await
        .unwrap();
    let status_scope = CommandScope {
        conversation_id: Some(cid.clone()),
        ..Default::default()
    };
    let end = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if http
            .operation_get("cancel-good", status_scope.clone())
            .await
            .unwrap()
            .state
            == rvoip_uctp::v2::payloads::OperationState::Completed
        {
            break;
        }
        assert!(tokio::time::Instant::now() < end);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let mut permission = Scope::collaboration(vec!["*".into()]);
    permission.operations.push("operation.cancel".into());
    let credential = f
        .server
        .store
        .issue(
            &f.client.identity_id,
            permission,
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let mut config = canonical_http_config(&f);
    config.credential = credential.token;
    let mut canceller = CanonicalHttpClient::open(config).await.unwrap();
    let (_, callback, scope) =
        begin_negotiation(&mut client, &cid, "cancel-pending", "cancel-pending-slot").await;
    canceller
        .request(
            payload("operation.cancel", json!({"operation_id":"cancel-pending"})),
            CommandScope {
                conversation_id: Some(cid.clone()),
                operation_id: Some("cancel-decision".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        http.operation_get("cancel-pending", status_scope.clone())
            .await
            .unwrap()
            .state,
        rvoip_uctp::v2::payloads::OperationState::Unknown
    );
    let p = &callback.original()["payload"];
    let streams=p["substrate_setup"]["streams"].as_array().unwrap().iter().map(|s|json!({"id":s["strm_id"],"kind":s["kind"],"direction":"sendrecv","codec":s["codec"]})).collect::<Vec<_>>();
    let response=serde_json::from_value(json!({"by_participant":p["by_participant"],"substrate":p["substrate"],"capabilities":{"audio_codecs":[{"name":"pcmu","params":{"sample_rate":8000,"channels":1}}],"max_streams_per_connection":8,"identity_assurance_offered":"identified"},"streams_answered":streams,"substrate_setup":p["substrate_setup"]})).unwrap();
    client
        .answer_connection_offer(&callback, response, &scope)
        .await
        .unwrap();
    let end = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if http
            .operation_get("cancel-pending", status_scope.clone())
            .await
            .unwrap()
            .state
            == rvoip_uctp::v2::payloads::OperationState::Cancelled
        {
            break;
        }
        assert!(tokio::time::Instant::now() < end);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    wait_slots(&f, 1).await;
    let page = serde_json::to_value(
        http.conversation_snapshot(&cid, serde_json::from_value(json!({})).unwrap())
            .await
            .unwrap(),
    )
    .unwrap();
    for (slot, state) in [
        ("cancel-good-slot", "ready"),
        ("cancel-pending-slot", "retired"),
    ] {
        assert!(page["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["kind"] == "connection"
                && v["data"]["connection_id"] == slot
                && v["data"]["state"] == state));
    }
    client.close();
    wait_slots(&f, 0).await;
    f.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn production_native_stream_source_uses_real_current_parent_without_creating_a_control_effect(
) {
    let f = Fixture::with_native_stream_scope("EdDSA", 12).await;
    ready_endpoint(&f).await;
    let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
    let cid = session(&f, &mut client).await;
    negotiate(
        &mut client,
        &cid,
        "native-stream-parent",
        "native-stream-parent-slot",
    )
    .await;
    let mut http = CanonicalHttpClient::open(canonical_http_config(&f))
        .await
        .unwrap();
    let scope = CommandScope {
        conversation_id: Some(cid.clone()),
        ..Default::default()
    };
    let end = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if http
            .operation_get("native-stream-parent", scope.clone())
            .await
            .unwrap()
            .state
            == rvoip_uctp::v2::payloads::OperationState::Completed
        {
            break;
        }
        assert!(tokio::time::Instant::now() < end);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let credential = f
        .server
        .store
        .authenticate(&f.client.credential)
        .unwrap()
        .credential_id;
    let request = connect_store::PeerAuthorityRequest {
        credentials: vec![credential.clone()],
        device: connect_store::PeerDeviceClaim {
            id: f.client.device_id.clone(),
            kind: f.client.device_kind.clone(),
            platform: f.client.device_platform.clone(),
        },
        key_id: f.client.signing_key_id.clone(),
        algorithm: f.client.signing_algorithm.clone(),
        command: connect_store::PeerCommand::NativeStream(
            connect_protocol::native_streams::Intent {
                operation_id: "source-projection-only".into(),
                conversation_id: cid.clone(),
                session_id: "native-session".into(),
                connection_id: "native-stream-parent-slot".into(),
                expected_revision: Some(1),
                by_participant: None,
                command: connect_protocol::native_streams::Command::Subscribe,
                payload: json!({"subscriptions":[{"strm_id":"voice"}]}),
            },
        ),
    };
    // This backend source check uses the real signed parent's durable result.
    // It does not submit a subscription or stand in for its public handler.
    let proof = f
        .server
        .store
        .admit_peer_authority(
            &request,
            &connect_store::AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    let source = f
        .server
        .store
        .project_native_stream_with_peer(&credential, &proof)
        .unwrap();
    assert_eq!(source.parent_operation_id, "native-stream-parent");
    assert_eq!(source.connection.connection.state, "ready");
    assert_eq!(source.stream_revision, 1);
    assert!(http
        .operation_get("source-projection-only", scope)
        .await
        .is_err());
    drop(proof);
    client.close();
    wait_slots(&f, 0).await;
    // Physical disposal precedes the durable retirement transaction. Source
    // planning is metadata; runtime ownership is checked separately on effects.
    let end = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if f.server
            .store
            .admit_peer_authority(
                &request,
                &connect_store::AuthorityWait::new(Duration::from_secs(1)),
            )
            .is_err()
        {
            break;
        }
        assert!(
            tokio::time::Instant::now() < end,
            "native source did not retire"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    f.server.shutdown().await.unwrap();
}

async fn stream_control(
    client: &mut CanonicalClient,
    cid: &str,
    conn: &str,
    operation: &str,
    revision: u64,
    kind: &str,
    input: serde_json::Value,
) -> Option<Envelope> {
    stream_control_state(
        client,
        cid,
        conn,
        operation,
        revision,
        kind,
        input,
        "completed",
    )
    .await
}
async fn stream_control_state(
    client: &mut CanonicalClient,
    cid: &str,
    conn: &str,
    operation: &str,
    revision: u64,
    kind: &str,
    input: serde_json::Value,
    expected_state: &str,
) -> Option<Envelope> {
    let scope = CommandScope {
        conversation_id: Some(cid.into()),
        session_id: Some("native-session".into()),
        connection_id: Some(conn.into()),
        operation_id: Some(operation.into()),
        expected_revision: Some(revision),
    };
    let requested = input.clone();
    let request_id = client.send_value(kind, input, scope.clone()).await.unwrap();
    let mut callback = None;
    let mut failed = false;
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(8), client.receive_control())
            .await
            .unwrap()
            .unwrap();
        if let Some(path) = std::env::var_os("CONNECT_G6_SUBSCRIPTION_SAMPLES") {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .unwrap();
            writeln!(file, "{}", json!({"command":kind,"payload":requested,"operation_id":operation,"expected_revision":revision,"frame":frame.original()})).unwrap();
        }
        match frame.kind() {
            "connection.offer" => {
                let p = &frame.original()["payload"];
                let streams=p["substrate_setup"]["streams"].as_array().unwrap().iter().map(|s|json!({"id":s["strm_id"],"kind":s["kind"],"direction":match s["direction"].as_str().unwrap(){"sendonly"=>"recvonly","recvonly"=>"sendonly",_=>"sendrecv"},"codec":s["codec"]})).collect::<Vec<_>>();
                let answer=serde_json::from_value(json!({"by_participant":p["by_participant"],"substrate":"quic","capabilities":{"audio_codecs":[{"name":"pcmu","params":{"sample_rate":8000,"channels":1}}],"max_streams_per_connection":8,"identity_assurance_offered":"identified"},"streams_answered":streams,"substrate_setup":p["substrate_setup"]})).unwrap();
                client
                    .answer_connection_offer(&frame, answer, &scope)
                    .await
                    .unwrap();
                callback = Some(frame);
            }
            "operation.updated" => {
                assert_eq!(
                    frame.original()["payload"]["state"],
                    expected_state,
                    "{:?}",
                    frame.original()
                );
                failed = expected_state == "failed";
            }
            "ack" => {
                assert!(
                    !(kind == "stream.subscribe" && expected_state == "failed"),
                    "definitive subscription refusals must terminate with correlated 404/488"
                );
                break;
            }
            "error" if kind == "stream.subscribe" && expected_state == "failed" => {
                assert!(
                    failed,
                    "a definitive subscription refusal retains its failed operation"
                );
                assert!(
                    callback.is_none(),
                    "refused selection must not allocate native resources"
                );
                assert_eq!(frame.original()["in_reply_to"], request_id);
                assert!(matches!(
                    frame.original()["payload"]["code"].as_u64(),
                    Some(404 | 488)
                ));
                break;
            }
            _ => panic!("unexpected native control reply: {:?}", frame.original()),
        }
    }
    callback
}
#[tokio::test]
async fn production_signed_native_stream_subscription_routes_audio_and_replays_without_another_effect(
) {
    native_stream_audio_case(false).await;
}
#[tokio::test]
async fn production_native_stream_captured_commit_recovers_storage_without_retaining_original_readers_or_reapplying(
) {
    native_stream_audio_case(true).await;
}
async fn native_stream_audio_case(recording_failure: bool) {
    let f = Fixture::with_native_stream_scope("EdDSA", 12).await;
    ready_endpoint(&f).await;
    let remote_config = party(&f, &f.peer.identity_id, 73);
    let mut target = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut source = CanonicalClient::open(remote_config.clone()).await.unwrap();
    let cid = session(&f, &mut target).await;
    let target_answer = negotiate_named(
        &mut target,
        &cid,
        "stream-target-offer",
        "stream-target",
        "target-voice",
    )
    .await;
    let source_answer = negotiate_named(
        &mut source,
        &cid,
        "stream-source-offer",
        "stream-source",
        "remote-voice",
    )
    .await;
    for (config, operation) in [
        (&f.client, "stream-target-offer"),
        (&remote_config, "stream-source-offer"),
    ] {
        let mut input = canonical_http_config(&f);
        input.credential = config.credential.clone();
        let mut http = CanonicalHttpClient::open(input).await.unwrap();
        let end = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            if http
                .operation_get(
                    operation,
                    CommandScope {
                        conversation_id: Some(cid.clone()),
                        ..Default::default()
                    },
                )
                .await
                .unwrap()
                .state
                == rvoip_uctp::v2::payloads::OperationState::Completed
            {
                break;
            }
            assert!(tokio::time::Instant::now() < end);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    if recording_failure {
        let c = rusqlite::Connection::open(&f.config.database).unwrap();
        c.execute_batch("CREATE TRIGGER isolated_stream_recording_failure BEFORE INSERT ON journal WHEN json_extract(NEW.data_json,'$.operation_id')='select-remote' AND json_extract(NEW.data_json,'$.state') IN ('committing','completed') BEGIN SELECT RAISE(ABORT,'isolated known outcome recording failure'); END;").unwrap();
    }
    let callback = stream_control_state(
        &mut target,
        &cid,
        "stream-target",
        "select-remote",
        1,
        "stream.subscribe",
        json!({"subscriptions":[{"strm_id":"remote-voice"}],"future_extension":{"retain":"exact"}}),
        if recording_failure {
            "preparing"
        } else {
            "completed"
        },
    )
    .await;
    if recording_failure {
        let end = tokio::time::Instant::now() + Duration::from_secs(1);
        loop {
            match f.server.store.try_acquire_authority(&[
                connect_store::AuthorityScope::conversation(
                    &cid,
                    connect_store::AuthorityMode::Write,
                )
                .unwrap(),
            ]) {
                Ok(held) => {
                    drop(held);
                    break;
                }
                Err(_) => {
                    assert!(
                        tokio::time::Instant::now() < end,
                        "captured receipt retained original command readers"
                    );
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }
        }
        let credential = f
            .server
            .store
            .authenticate(&f.client.credential)
            .unwrap()
            .credential_id;
        let before = f
            .server
            .store
            .execute_operation_action(
                &credential,
                connect_protocol::transitions::OperationAction::Get {
                    operation_id: "select-remote".into(),
                },
            )
            .unwrap();
        assert_eq!(
            before.state,
            connect_protocol::transitions::OperationState::Preparing
        );
        assert_eq!(before.effects.len(), 1);
        let mut permission = Scope::collaboration(vec!["*".into()]);
        permission
            .operations
            .extend(["operation.cancel".into(), "stream.subscribe".into()]);
        let cancellation = f
            .server
            .store
            .issue(
                &f.client.identity_id,
                permission,
                chrono::Utc::now().timestamp() + 600,
            )
            .unwrap();
        let mut config = canonical_http_config(&f);
        config.credential = cancellation.token;
        let mut canceller = CanonicalHttpClient::open(config).await.unwrap();
        canceller
            .request(
                payload("operation.cancel", json!({"operation_id":"select-remote"})),
                CommandScope {
                    conversation_id: Some(cid.clone()),
                    operation_id: Some("late-stream-cancel".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let cancelled = f
            .server
            .store
            .execute_operation_action(
                &credential,
                connect_protocol::transitions::OperationAction::Get {
                    operation_id: "select-remote".into(),
                },
            )
            .unwrap();
        assert_eq!(
            cancelled.state,
            connect_protocol::transitions::OperationState::Unknown
        );
        assert!(cancelled.cancel_requested);
        rusqlite::Connection::open(&f.config.database)
            .unwrap()
            .execute_batch("DROP TRIGGER isolated_stream_recording_failure")
            .unwrap();
        let end = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let after = f
                .server
                .store
                .execute_operation_action(
                    &credential,
                    connect_protocol::transitions::OperationAction::Get {
                        operation_id: "select-remote".into(),
                    },
                )
                .unwrap();
            assert_eq!(after.record_id, before.record_id);
            assert_eq!(after.effects[0].id, before.effects[0].id);
            if after.state == connect_protocol::transitions::OperationState::Completed {
                break;
            }
            assert!(tokio::time::Instant::now() < end);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    // Audio uses the existing negotiated mix output when its format matches.
    let target_handle = match callback.as_ref() {
        Some(callback) => callback.original()["payload"]["substrate_setup"]["streams"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["kind"] == "audio")
            .unwrap()["stream_local_id"]
            .as_u64()
            .unwrap() as u16,
        None => target_answer.original()["payload"]["substrate_setup"]["streams"][0]
            ["stream_local_id"]
            .as_u64()
            .unwrap() as u16,
    };
    let source_handle = source_answer.original()["payload"]["substrate_setup"]["streams"][0]
        ["stream_local_id"]
        .as_u64()
        .unwrap() as u16;
    let mut http = CanonicalHttpClient::open(canonical_http_config(&f))
        .await
        .unwrap();
    let scope = CommandScope {
        conversation_id: Some(cid.clone()),
        ..Default::default()
    };
    let status = http
        .operation_get("select-remote", scope.clone())
        .await
        .unwrap();
    let credential = f
        .server
        .store
        .authenticate(&f.client.credential)
        .unwrap()
        .credential_id;
    let durable = f
        .server
        .store
        .execute_operation_action(
            &credential,
            connect_protocol::transitions::OperationAction::Get {
                operation_id: "select-remote".into(),
            },
        )
        .unwrap();
    assert_eq!(durable.effects.len(), 1);
    assert_eq!(status.result.as_ref().unwrap()["stream_revision"], 2);
    assert!(stream_control(
        &mut target,
        &cid,
        "stream-target",
        "select-remote",
        1,
        "stream.subscribe",
        json!({"subscriptions":[{"strm_id":"remote-voice"}],"future_extension":{"retain":"exact"}})
    )
    .await
    .is_none());
    assert_eq!(
        serde_json::to_value(http.operation_get("select-remote", scope).await.unwrap()).unwrap(),
        serde_json::to_value(status).unwrap()
    );
    // A changed extension is a different intent under the same logical ID.
    target.send_value("stream.subscribe",json!({"subscriptions":[{"strm_id":"remote-voice"}],"future_extension":{"retain":"changed"}}),CommandScope{conversation_id:Some(cid.clone()),session_id:Some("native-session".into()),connection_id:Some("stream-target".into()),operation_id:Some("select-remote".into()),expected_revision:Some(1)}).await.unwrap();
    assert_eq!(target.receive_control().await.unwrap().kind(), "error");
    // The same Device/key on another physical peer can recover metadata but
    // cannot borrow the existing route's original physical custody.
    let mut foreign = CanonicalClient::open(f.client.clone()).await.unwrap();
    foreign
        .send(
            payload(
                "stream.subscribe",
                json!({"subscriptions":[{"strm_id":"remote-voice"}]}),
            ),
            CommandScope {
                conversation_id: Some(cid.clone()),
                session_id: Some("native-session".into()),
                connection_id: Some("stream-target".into()),
                operation_id: Some("foreign-physical-control".into()),
                expected_revision: Some(2),
            },
        )
        .await
        .unwrap();
    let refused = foreign.receive_control().await.unwrap();
    assert_eq!(refused.kind(), "error");
    // Physical denial precedes durable admission, so it cannot reserve an ID
    // or fail an accepted original root owned by another physical peer.
    let refused = f
        .server
        .store
        .execute_operation_action(
            &credential,
            connect_protocol::transitions::OperationAction::Get {
                operation_id: "foreign-physical-control".into(),
            },
        )
        .unwrap_err();
    assert_eq!(refused.code, "forbidden");
    assert_eq!(
        rusqlite::Connection::open(&f.config.database)
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM operations WHERE operation_id='foreign-physical-control'",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        f.server
            .store
            .execute_operation_action(
                &credential,
                connect_protocol::transitions::OperationAction::Get {
                    operation_id: "select-remote".into()
                }
            )
            .unwrap(),
        durable
    );
    drop(foreign);
    let mut clients = vec![source, target];
    assert_audio_reaches_other_participant(&mut clients, source_handle, &[target_handle], 9000)
        .await;
    wait_slots(&f, 2).await;
    assert!(stream_control(
        &mut clients[1],
        &cid,
        "stream-target",
        "remove-remote",
        2,
        "stream.unsubscribe",
        json!({"strm_ids":["remote-voice"]})
    )
    .await
    .is_none());
    assert_audio_is_silent_after_unsubscribe(&mut clients, source_handle, target_handle).await;
    wait_slots(&f, 2).await;
    assert!(stream_control(
        &mut clients[1],
        &cid,
        "stream-target",
        "select-remote-again",
        3,
        "stream.subscribe",
        json!({"subscriptions":[{"strm_id":"remote-voice"}]})
    )
    .await
    .is_none());
    assert_audio_reaches_other_participant(&mut clients, source_handle, &[target_handle], 9400)
        .await;
    drop(clients);
    f.server.shutdown().await.unwrap();
}

async fn assert_audio_is_silent_after_unsubscribe(
    clients: &mut [CanonicalClient],
    source_handle: u16,
    target_handle: u16,
) {
    use rvoip_uctp::substrate::{
        pack_rtp_datagram, unpack_rtp_datagram, RtpDatagram, RtpMediaPayload,
    };
    let (source, receivers) = clients.split_at_mut(1);
    let sending = async {
        for seq in 9200..9300u32 {
            source[0]
                .send_native_rtp_datagram(
                    &pack_rtp_datagram(&RtpDatagram {
                        flags: 0,
                        stream_local_id: source_handle,
                        seq,
                        rtp: RtpMediaPayload {
                            payload: vec![0x80; 160].into(),
                            payload_type: 0,
                            sequence_number: seq as u16,
                            timestamp: seq * 160,
                            ssrc: 0x4736_0001,
                        },
                    })
                    .unwrap(),
                )
                .unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    let receiving = async {
        let start = tokio::time::Instant::now();
        while start.elapsed() < Duration::from_millis(1300) {
            // Drain datagrams emitted before the committed unsubscribe. Atomic
            // datagram timeouts preserve the physical connection.
            match tokio::time::timeout(
                Duration::from_millis(50),
                receivers[0].receive_native_rtp_datagram(),
            )
            .await
            {
                Ok(Ok(bytes)) => {
                    let packet = unpack_rtp_datagram(&bytes).unwrap();
                    assert_eq!(packet.stream_local_id, target_handle);
                    if start.elapsed() >= Duration::from_millis(300) {
                        assert!(
                            packet.rtp.payload.iter().all(|v| *v == 0xff || *v == 0x7f),
                            "removed source still reached receiver"
                        );
                    }
                }
                Ok(Err(error)) => panic!("unsubscribe ended physical receiver: {error:?}"),
                Err(_) => (),
            }
        }
    };
    tokio::join!(sending, receiving);
}

fn av_capabilities() -> serde_json::Value {
    json!({"audio_codecs":[{"name":"pcmu","params":{"sample_rate":8000,"channels":1}}],"video_codecs":[{"name":"vp8","params":{"sample_rate":90000,"channels":0}}],"max_streams_per_connection":8,"identity_assurance_offered":"identified"})
}
async fn negotiate_av(
    client: &mut CanonicalClient,
    cid: &str,
    operation: &str,
    connection: &str,
    streams: serde_json::Value,
) -> Envelope {
    negotiate_media(
        client,
        cid,
        operation,
        connection,
        streams,
        av_capabilities(),
    )
    .await
}
async fn negotiate_media(
    client: &mut CanonicalClient,
    cid: &str,
    operation: &str,
    connection: &str,
    streams: serde_json::Value,
    capabilities: serde_json::Value,
) -> Envelope {
    negotiate_media_payload(
        client,
        cid,
        operation,
        connection,
        json!({"substrate":"quic","capabilities":capabilities,"streams_offered":streams}),
    )
    .await
}
async fn negotiate_media_payload(
    client: &mut CanonicalClient,
    cid: &str,
    operation: &str,
    connection: &str,
    offer: Value,
) -> Envelope {
    let capabilities = offer["capabilities"].clone();
    let scope = CommandScope {
        conversation_id: Some(cid.into()),
        session_id: Some("native-session".into()),
        connection_id: Some(connection.into()),
        operation_id: Some(operation.into()),
        expected_revision: Some(0),
    };
    client
        .send_value("connection.offer", offer, scope.clone())
        .await
        .unwrap();
    let answer = client.receive_control().await.unwrap();
    assert_eq!(
        answer.kind(),
        "connection.answer",
        "{:?}",
        answer.original()
    );
    let callback = client.receive_control().await.unwrap();
    assert_eq!(
        callback.kind(),
        "connection.offer",
        "{:?}",
        callback.original()
    );
    let p = &callback.original()["payload"];
    let streams=p["substrate_setup"]["streams"].as_array().unwrap().iter().map(|s|json!({"id":s["strm_id"],"kind":s["kind"],"direction":match s["direction"].as_str().unwrap(){"sendonly"=>"recvonly","recvonly"=>"sendonly",_=>"sendrecv"},"codec":s["codec"]})).collect::<Vec<_>>();
    client.answer_connection_offer(&callback,serde_json::from_value(json!({"by_participant":p["by_participant"],"substrate":"quic","capabilities":capabilities.clone(),"streams_answered":streams,"substrate_setup":p["substrate_setup"]})).unwrap(),&scope).await.unwrap();
    answer
}
async fn wait_native_operation(f: &Fixture, credential: &str, cid: &str, id: &str) {
    let mut config = canonical_http_config(f);
    config.credential = credential.into();
    let mut http = CanonicalHttpClient::open(config).await.unwrap();
    let end = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let r = http
            .operation_get(
                id,
                CommandScope {
                    conversation_id: Some(cid.into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        if r.state == rvoip_uctp::v2::payloads::OperationState::Completed {
            return;
        }
        assert!(
            tokio::time::Instant::now() < end,
            "operation did not complete: {:?}",
            serde_json::to_value(r).unwrap()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
async fn assert_vp8_routes(
    source: &CanonicalClient,
    target: &mut CanonicalClient,
    source_handle: u16,
    target_handle: u16,
) {
    assert_vp8_routes_from(source, target, source_handle, target_handle, 1).await;
}
async fn assert_vp8_routes_from(
    source: &CanonicalClient,
    target: &mut CanonicalClient,
    source_handle: u16,
    target_handle: u16,
    first_sequence: u16,
) {
    use rvoip_uctp::substrate::{observe_rtp_datagram, pack, MediaDatagram};
    let frame = include_bytes!("../fixtures/gate6-vp8-black-640x360.vp8");
    let sending = async {
        for sequence in first_sequence..first_sequence + 29 {
            let mut rtp = vec![0x80, 0x80 | 96];
            rtp.extend_from_slice(&sequence.to_be_bytes());
            rtp.extend_from_slice(&(u32::from(sequence) * 9000).to_be_bytes());
            rtp.extend_from_slice(&0x6476_7001u32.to_be_bytes());
            rtp.push(0x10);
            rtp.extend_from_slice(frame);
            source
                .send_native_rtp_datagram(&pack(&MediaDatagram {
                    flags: 0,
                    stream_local_id: source_handle,
                    seq: u32::from(sequence),
                    payload: rtp.into(),
                }))
                .unwrap();
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    };
    let receiving = tokio::time::timeout(Duration::from_secs(3), async {
        let mut assembler = connect_client::VideoFrameAssembler::default();
        loop {
            let bytes = target.receive_native_rtp_datagram().await.unwrap();
            let packet = observe_rtp_datagram(&bytes).unwrap();
            if packet.stream_local_id != target_handle {
                continue;
            }
            let raw = packet.rtp.packet().serialize().unwrap().to_vec();
            let packet = connect_client::NativeVideoPacket::parse(raw).unwrap();
            assert_eq!(packet.payload_type, 96);
            assert!(packet.marker);
            if let Some(received) = assembler.push(&packet).unwrap() {
                assert_eq!(received.encoded.as_slice(), frame);
                assert!(received.keyframe);
                if let Some(directory) = std::env::var_os("CONNECT_GATE6_VIDEO_SAMPLES") {
                    let directory = std::path::PathBuf::from(directory);
                    std::fs::create_dir_all(&directory).unwrap();
                    std::fs::write(directory.join("received.vp8"), &received.encoded).unwrap();
                }
                break;
            }
        }
    });
    let (_, result) = tokio::join!(sending, receiving);
    result.expect("VP8 must cross the actual selected native route");
}
#[tokio::test]
async fn production_native_video_subscription_negotiates_real_resources_and_routes_vp8() {
    native_video_selection_case(true, false).await;
}
#[tokio::test]
async fn production_native_video_claim_cannot_bypass_verified_audio_only_endpoint() {
    native_video_selection_case(false, false).await;
}
#[tokio::test]
async fn production_native_participant_video_selection_routes_real_vp8() {
    native_video_selection_case(true, true).await;
}
async fn native_video_selection_case(endpoint_video: bool, participant_selector: bool) {
    let f = Fixture::with_native_stream_scope("EdDSA", 12).await;
    ready_endpoint(&f).await;
    let source_config = party_with_capabilities(
        &f,
        &f.peer.identity_id,
        85,
        false,
        if endpoint_video {
            vec!["audio".into(), "video".into()]
        } else {
            vec!["audio".into()]
        },
    );
    let mut target = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut source = CanonicalClient::open(source_config.clone()).await.unwrap();
    let cid = session_with_media(
        &f,
        &mut target,
        "voice+video",
        json!({"audio":["g.711-mu"],"video":["vp8"]}),
    )
    .await;
    let target_answer=negotiate_av(&mut target,&cid,"video-target-offer","video-target",json!([{"id":"monitor-audio","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}])).await;
    let source_answer=negotiate_av(&mut source,&cid,"video-source-offer","video-source",if endpoint_video {json!([{"id":"source-audio","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]},{"id":"source-camera","kind":"video","direction":"sendonly","codec_preferences":["vp8"]}])}else{json!([{"id":"source-audio","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}])}).await;
    wait_native_operation(&f, &f.client.credential, &cid, "video-target-offer").await;
    wait_native_operation(&f, &source_config.credential, &cid, "video-source-offer").await;
    stream_control(
        &mut target,
        &cid,
        "video-target",
        "video-select-audio",
        1,
        "stream.subscribe",
        json!({"subscriptions":[{"strm_id":"source-audio"}]}),
    )
    .await;
    let source_audio_handle = source_answer.original()["payload"]["substrate_setup"]["streams"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["strm_id"] == "source-audio")
        .unwrap()["stream_local_id"]
        .as_u64()
        .unwrap() as u16;
    let target_audio_handle = target_answer.original()["payload"]["substrate_setup"]["streams"][0]
        ["stream_local_id"]
        .as_u64()
        .unwrap() as u16;
    let mut clients = vec![source, target];
    assert_audio_reaches_other_participant(
        &mut clients,
        source_audio_handle,
        &[target_audio_handle],
        18000,
    )
    .await;
    if !endpoint_video {
        clients[0].send_value("connection.offer",json!({"substrate":"quic","capabilities":av_capabilities(),"streams_offered":[{"id":"denied-camera","kind":"video","direction":"sendonly","codec_preferences":["vp8"]}]}),CommandScope{conversation_id:Some(cid.clone()),session_id:Some("native-session".into()),connection_id:Some("denied-video-slot".into()),operation_id:Some("denied-video-claim".into()),expected_revision:Some(0)}).await.unwrap();
        let failed = clients[0].receive_control().await.unwrap();
        assert_eq!(failed.kind(), "operation.updated");
        assert_eq!(failed.original()["payload"]["state"], "failed");
        assert_eq!(clients[0].receive_control().await.unwrap().kind(), "ack");
        let credential = f
            .server
            .store
            .authenticate(&source_config.credential)
            .unwrap()
            .credential_id;
        let record = f
            .server
            .store
            .execute_operation_action(
                &credential,
                connect_protocol::transitions::OperationAction::Get {
                    operation_id: "denied-video-claim".into(),
                },
            )
            .unwrap();
        assert!(record.effects.is_empty());
        wait_slots(&f, 2).await;
        assert_audio_reaches_other_participant(
            &mut clients,
            source_audio_handle,
            &[target_audio_handle],
            18200,
        )
        .await;
        drop(clients);
        f.server.shutdown().await.unwrap();
        return;
    }
    let scope = CommandScope {
        conversation_id: Some(cid.clone()),
        session_id: Some("native-session".into()),
        connection_id: Some("video-target".into()),
        operation_id: Some("select-camera".into()),
        expected_revision: Some(2),
    };
    clients[1]
        .send_value(
            "stream.subscribe",
            if participant_selector {
                json!({"subscriptions":[{"from_participant":source_answer.original()["payload"]["by_participant"],"kinds":["video"]}]})
            } else {
                json!({"subscriptions":[{"strm_id":"source-camera"}]})
            },
            scope.clone(),
        )
        .await
        .unwrap();
    let callback = clients[1].receive_control().await.unwrap();
    assert_eq!(
        callback.kind(),
        "connection.offer",
        "{:?}",
        callback.original()
    );
    let p = &callback.original()["payload"];
    let streams=p["substrate_setup"]["streams"].as_array().unwrap().iter().map(|s|json!({"id":s["strm_id"],"kind":s["kind"],"direction":match s["direction"].as_str().unwrap(){"sendonly"=>"recvonly","recvonly"=>"sendonly",_=>"sendrecv"},"codec":s["codec"]})).collect::<Vec<_>>();
    let correct = json!({"by_participant":p["by_participant"],"substrate":"quic","capabilities":av_capabilities(),"streams_answered":streams,"substrate_setup":p["substrate_setup"]});
    let mut stale = correct.clone();
    stale["substrate_setup"]["revision"] = json!(4);
    clients[1]
        .answer_connection_offer(&callback, serde_json::from_value(stale).unwrap(), &scope)
        .await
        .unwrap();
    assert_eq!(clients[1].receive_control().await.unwrap().kind(), "error");
    let credential = f
        .server
        .store
        .authenticate(&f.client.credential)
        .unwrap()
        .credential_id;
    let preparing = f
        .server
        .store
        .execute_operation_action(
            &credential,
            connect_protocol::transitions::OperationAction::Get {
                operation_id: "select-camera".into(),
            },
        )
        .unwrap();
    assert_eq!(
        preparing.state,
        connect_protocol::transitions::OperationState::Preparing
    );
    assert_eq!(preparing.effects.len(), 1);
    // The invalid signed confirmation cannot commit new resources or disturb
    // already authorized audio while the same negotiation remains pending.
    assert_audio_reaches_other_participant(
        &mut clients,
        source_audio_handle,
        &[target_audio_handle],
        18200,
    )
    .await;
    clients[1]
        .answer_connection_offer(&callback, serde_json::from_value(correct).unwrap(), &scope)
        .await
        .unwrap();
    let completed = clients[1].receive_control().await.unwrap();
    assert_eq!(completed.kind(), "operation.updated");
    assert_eq!(completed.original()["payload"]["state"], "completed");
    assert_eq!(clients[1].receive_control().await.unwrap().kind(), "ack");
    let after = f
        .server
        .store
        .execute_operation_action(
            &credential,
            connect_protocol::transitions::OperationAction::Get {
                operation_id: "select-camera".into(),
            },
        )
        .unwrap();
    assert_eq!(after.effects[0].id, preparing.effects[0].id);
    assert_eq!(after.result.as_ref().unwrap()["resources_changed"], true);
    assert_eq!(after.result.as_ref().unwrap()["stream_revision"], 3);
    let target_handle = callback.original()["payload"]["substrate_setup"]["streams"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["strm_id"] == "source-camera")
        .unwrap()["stream_local_id"]
        .as_u64()
        .unwrap() as u16;
    let source_handle = source_answer.original()["payload"]["substrate_setup"]["streams"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["strm_id"] == "source-camera")
        .unwrap()["stream_local_id"]
        .as_u64()
        .unwrap() as u16;
    let (sources, targets) = clients.split_at_mut(1);
    assert_vp8_routes(&sources[0], &mut targets[0], source_handle, target_handle).await;
    assert_audio_reaches_other_participant(
        &mut clients,
        source_audio_handle,
        &[target_audio_handle],
        18400,
    )
    .await;
    wait_slots(&f, 2).await;
    drop(clients);
    f.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn production_accepted_live_video_upgrade_adds_connections_and_preserves_existing_audio() {
    live_video_upgrade_and_connection_end(false).await;
}
#[tokio::test]
async fn production_native_connection_end_captured_outcome_recovers_after_recording_failure_and_cancel(
) {
    live_video_upgrade_and_connection_end(true).await;
}
async fn live_video_upgrade_and_connection_end(recording_failure: bool) {
    let f = Fixture::with_native_end_scope("EdDSA", 12).await;
    ready_endpoint(&f).await;
    let source_config = party_with_capabilities(
        &f,
        &f.peer.identity_id,
        86,
        true,
        vec!["audio".into(), "video".into()],
    );
    let mut target = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut source = CanonicalClient::open(source_config.clone()).await.unwrap();
    let cid = session_with_media(
        &f,
        &mut target,
        "voice",
        json!({"audio":["g.711-mu"],"video":["vp8"]}),
    )
    .await;
    let target_audio=negotiate_av(&mut target,&cid,"upgrade-target-audio-offer","upgrade-target-audio",json!([{"id":"upgrade-monitor-audio","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}])).await;
    let source_audio=negotiate_av(&mut source,&cid,"upgrade-source-audio-offer","upgrade-source-audio",json!([{"id":"upgrade-source-audio","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}])).await;
    wait_native_operation(&f, &f.client.credential, &cid, "upgrade-target-audio-offer").await;
    wait_native_operation(
        &f,
        &source_config.credential,
        &cid,
        "upgrade-source-audio-offer",
    )
    .await;
    stream_control(
        &mut target,
        &cid,
        "upgrade-target-audio",
        "upgrade-select-audio",
        1,
        "stream.subscribe",
        json!({"subscriptions":[{"strm_id":"upgrade-source-audio"}]}),
    )
    .await;
    stream_control(
        &mut source,
        &cid,
        "upgrade-source-audio",
        "upgrade-select-return",
        1,
        "stream.subscribe",
        json!({"subscriptions":[{"strm_id":"upgrade-monitor-audio"}]}),
    )
    .await;
    let source_audio_handle = source_audio.original()["payload"]["substrate_setup"]["streams"][0]
        ["stream_local_id"]
        .as_u64()
        .unwrap() as u16;
    let target_audio_handle = target_audio.original()["payload"]["substrate_setup"]["streams"][0]
        ["stream_local_id"]
        .as_u64()
        .unwrap() as u16;
    let mut clients = vec![source, target];
    assert_audio_reaches_other_participant(
        &mut clients,
        source_audio_handle,
        &[target_audio_handle],
        13000,
    )
    .await;
    // This offer is duplex: exercise the return direction before asserting
    // readiness-gated Active, then retain that phase across the additive upgrade.
    clients.swap(0, 1);
    assert_audio_reaches_other_participant(
        &mut clients,
        target_audio_handle,
        &[source_audio_handle],
        13500,
    )
    .await;
    clients.swap(0, 1);
    let until = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let snapshot = clients[1]
            .conversation_snapshot(&cid, serde_json::from_value(json!({})).unwrap())
            .await
            .unwrap();
        if snapshot
            .items
            .iter()
            .any(|item| item["kind"] == "session" && item["data"]["state"] == "active")
        {
            break;
        }
        assert!(
            tokio::time::Instant::now() < until,
            "actual duplex did not activate the Session"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let upgrade = Request {
        operation_id: "accepted-live-video-upgrade".into(),
        conversation_id: cid.clone(),
        session_id: "native-session".into(),
        by_participant: None,
        expected_revision: Some(1),
        action: Action::Upgrade {
            media: vec!["video".into()],
        },
    };
    let outcome = clients[1].session_command(upgrade.clone()).await.unwrap();
    let Outcome::SessionControl(outcome) = outcome else {
        panic!()
    };
    assert_eq!(outcome.session.media, vec!["audio", "video"]);
    assert_eq!(outcome.session.state, "active");
    // Desired Session media is distinct from actual new Connection readiness.
    // Existing audio ports, operation identities and stream revision stay live.
    wait_slots(&f, 2).await;
    assert_audio_reaches_other_participant(
        &mut clients,
        source_audio_handle,
        &[target_audio_handle],
        13200,
    )
    .await;
    let source_video=negotiate_av(&mut clients[0],&cid,"upgrade-source-video-offer","upgrade-source-video",json!([{"id":"upgrade-camera","kind":"video","direction":"sendonly","codec_preferences":["vp8"]}])).await;
    negotiate_av(&mut clients[1],&cid,"upgrade-target-video-offer","upgrade-target-video",json!([{"id":"upgrade-monitor-video","kind":"video","direction":"recvonly","codec_preferences":["vp8"]}])).await;
    wait_native_operation(
        &f,
        &source_config.credential,
        &cid,
        "upgrade-source-video-offer",
    )
    .await;
    wait_native_operation(&f, &f.client.credential, &cid, "upgrade-target-video-offer").await;
    let callback = stream_control(
        &mut clients[1],
        &cid,
        "upgrade-target-video",
        "upgrade-select-camera",
        1,
        "stream.subscribe",
        json!({"subscriptions":[{"strm_id":"upgrade-camera"}]}),
    )
    .await
    .unwrap();
    let target_handle = callback.original()["payload"]["substrate_setup"]["streams"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["strm_id"] == "upgrade-camera")
        .unwrap()["stream_local_id"]
        .as_u64()
        .unwrap() as u16;
    let source_handle = source_video.original()["payload"]["substrate_setup"]["streams"][0]
        ["stream_local_id"]
        .as_u64()
        .unwrap() as u16;
    let (sources, targets) = clients.split_at_mut(1);
    assert_vp8_routes(&sources[0], &mut targets[0], source_handle, target_handle).await;
    assert_audio_reaches_other_participant(
        &mut clients,
        source_audio_handle,
        &[target_audio_handle],
        13400,
    )
    .await;
    wait_slots(&f, 4).await;
    assert_eq!(
        clients[1].session_command(upgrade).await.unwrap(),
        Outcome::SessionControl(outcome)
    );
    wait_slots(&f, 4).await;
    let mut http = CanonicalHttpClient::open(canonical_http_config(&f))
        .await
        .unwrap();
    let unsupported = http
        .request(
            payload(
                "connection.end",
                json!({"reason_code":1000,"reason":"finished"}),
            ),
            CommandScope {
                conversation_id: Some(cid.clone()),
                session_id: Some("native-session".into()),
                connection_id: Some("upgrade-target-video".into()),
                operation_id: Some("http-cannot-end-video".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(unsupported.frames[0].original()["payload"]["code"], 501);
    assert_eq!(
        unsupported.frames[0].original()["payload"]["reason"],
        "unsupported_feature"
    );
    wait_slots(&f, 4).await;
    // A second physical peer with identical Device/key/credential can read
    // metadata but cannot end the original peer's owned native route.
    let mut foreign = CanonicalClient::open(f.client.clone()).await.unwrap();
    foreign
        .send_value(
            "connection.end",
            json!({"reason_code":1000,"reason":"finished"}),
            CommandScope {
                conversation_id: Some(cid.clone()),
                session_id: Some("native-session".into()),
                connection_id: Some("upgrade-target-video".into()),
                operation_id: Some("foreign-end-video".into()),
                expected_revision: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(foreign.receive_control().await.unwrap().kind(), "error");
    let credential = f
        .server
        .store
        .authenticate(&f.client.credential)
        .unwrap()
        .credential_id;
    let refused = f
        .server
        .store
        .execute_operation_action(
            &credential,
            connect_protocol::transitions::OperationAction::Get {
                operation_id: "foreign-end-video".into(),
            },
        )
        .unwrap_err();
    assert_eq!(refused.code, "forbidden");
    assert_eq!(
        rusqlite::Connection::open(&f.config.database)
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM operations WHERE operation_id='foreign-end-video'",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
    wait_slots(&f, 4).await;
    drop(foreign);
    if recording_failure {
        rusqlite::Connection::open(&f.config.database).unwrap().execute_batch("CREATE TRIGGER isolated_end_recording_failure BEFORE INSERT ON journal WHEN json_extract(NEW.data_json,'$.operation_id')='end-target-video' AND json_extract(NEW.data_json,'$.state') IN ('committing','completed') BEGIN SELECT RAISE(ABORT,'isolated captured end recording failure'); END;").unwrap();
        end_native_connection(
            &mut clients[1],
            &cid,
            "upgrade-target-video",
            "end-target-video",
            None,
            "preparing",
        )
        .await;
        wait_slots(&f, 3).await;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            if let Ok(held) = f.server.store.try_acquire_authority(&[
                connect_store::AuthorityScope::conversation(
                    &cid,
                    connect_store::AuthorityMode::Write,
                )
                .unwrap(),
            ]) {
                drop(held);
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "captured end retained command readers"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let credential = f
            .server
            .store
            .authenticate(&f.client.credential)
            .unwrap()
            .credential_id;
        let get = || {
            f.server
                .store
                .execute_operation_action(
                    &credential,
                    connect_protocol::transitions::OperationAction::Get {
                        operation_id: "end-target-video".into(),
                    },
                )
                .unwrap()
        };
        let before = get();
        assert_eq!(
            before.state,
            connect_protocol::transitions::OperationState::Preparing
        );
        assert_eq!(before.effects.len(), 1);
        let mut permission = Scope::collaboration(vec!["*".into()]);
        permission
            .operations
            .extend(["connection.end".into(), "operation.cancel".into()]);
        let cancellation = f
            .server
            .store
            .issue(
                &f.client.identity_id,
                permission,
                chrono::Utc::now().timestamp() + 600,
            )
            .unwrap();
        let mut config = canonical_http_config(&f);
        config.credential = cancellation.token;
        let mut http = CanonicalHttpClient::open(config).await.unwrap();
        http.request(
            payload(
                "operation.cancel",
                json!({"operation_id":"end-target-video"}),
            ),
            CommandScope {
                conversation_id: Some(cid.clone()),
                operation_id: Some("late-end-cancel".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            get().state,
            connect_protocol::transitions::OperationState::Unknown
        );
        rusqlite::Connection::open(&f.config.database)
            .unwrap()
            .execute_batch("DROP TRIGGER isolated_end_recording_failure")
            .unwrap();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let after = get();
            assert_eq!(after.record_id, before.record_id);
            assert_eq!(after.effects[0].id, before.effects[0].id);
            if after.state == connect_protocol::transitions::OperationState::Completed {
                assert!(after.cancel_requested);
                break;
            }
            assert!(tokio::time::Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    let ended = end_native_connection(
        &mut clients[1],
        &cid,
        "upgrade-target-video",
        "end-target-video",
        None,
        "completed",
    )
    .await;
    assert_eq!(
        ended["result"],
        json!({"connection_id":"upgrade-target-video","ended":true})
    );
    wait_slots(&f, 3).await;
    let replay = end_native_connection(
        &mut clients[1],
        &cid,
        "upgrade-target-video",
        "end-target-video",
        None,
        "completed",
    )
    .await;
    assert_eq!(ended, replay);
    wait_slots(&f, 3).await;
    assert_audio_reaches_other_participant(
        &mut clients,
        source_audio_handle,
        &[target_audio_handle],
        13600,
    )
    .await;
    // The same physical peer can still use the Session and negotiate a fresh
    // Connection after ending its previous video Connection.
    negotiate_av(&mut clients[1], &cid, "replacement-video-offer", "replacement-video", json!([{"id":"replacement-monitor","kind":"video","direction":"recvonly","codec_preferences":["vp8"]}])).await;
    wait_native_operation(&f, &f.client.credential, &cid, "replacement-video-offer").await;
    wait_slots(&f, 4).await;
    assert_audio_reaches_other_participant(
        &mut clients,
        source_audio_handle,
        &[target_audio_handle],
        13800,
    )
    .await;
    drop(clients);
    f.server.shutdown().await.unwrap();
}

async fn end_native_connection(
    client: &mut CanonicalClient,
    cid: &str,
    conn: &str,
    operation: &str,
    revision: Option<u64>,
    state: &str,
) -> Value {
    client
        .send_value(
            "connection.end",
            json!({"reason_code":1000,"reason":"finished","future":{"keep":true}}),
            CommandScope {
                conversation_id: Some(cid.into()),
                session_id: Some("native-session".into()),
                connection_id: Some(conn.into()),
                operation_id: Some(operation.into()),
                expected_revision: revision,
            },
        )
        .await
        .unwrap();
    let mut updated = None;
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(8), client.receive_control())
            .await
            .unwrap()
            .unwrap();
        match frame.kind() {
            "operation.updated" => {
                assert_eq!(
                    frame.original()["payload"]["state"],
                    state,
                    "{:?}",
                    frame.original()
                );
                updated = Some(frame.original()["payload"].clone());
            }
            "ack" => return updated.unwrap(),
            _ => panic!(
                "Unexpected native termination reply: {:?}",
                frame.original()
            ),
        }
    }
}

#[tokio::test]
async fn production_native_screen_upgrade_routes_distinct_vp8_and_closes_without_losing_audio() {
    native_screen_case(0, false).await;
}
#[tokio::test]
async fn production_native_screen_cannot_borrow_verified_video_endpoint_permission() {
    native_screen_case(1, false).await;
}
#[tokio::test]
async fn production_native_screen_cannot_borrow_a_participants_accepted_camera_capability() {
    native_screen_case(2, false).await;
}
#[tokio::test]
async fn production_native_participant_screen_selection_routes_real_vp8() {
    native_screen_case(0, true).await;
}
async fn native_screen_case(refusal: u8, participant_selector: bool) {
    let f = Fixture::with_native_end_scope("EdDSA", 12).await;
    ready_endpoint_with_capabilities(&f, vec!["audio".into(), "screen".into()]).await;
    let source_config = party_with_capabilities(
        &f,
        &f.peer.identity_id,
        87,
        false,
        if refusal == 1 {
            vec!["audio".into(), "video".into()]
        } else {
            vec!["audio".into(), "screen".into()]
        },
    );
    let mut target = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut source = CanonicalClient::open(source_config.clone()).await.unwrap();
    let caps = json!({"audio":["g.711-mu"],"video":["vp8"],"screen":["vp8"]});
    let accepted = if refusal == 2 {
        json!({"audio":["g.711-mu"],"video":["vp8"]})
    } else {
        caps.clone()
    };
    let cid = session_with_acceptance(&f, &mut target, "voice", caps, accepted).await;
    let target_audio=negotiate_av(&mut target,&cid,"screen-target-audio-offer","screen-target-audio",json!([{"id":"screen-monitor-audio","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}])).await;
    let source_audio=negotiate_av(&mut source,&cid,"screen-source-audio-offer","screen-source-audio",json!([{"id":"screen-source-audio","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}])).await;
    wait_native_operation(&f, &f.client.credential, &cid, "screen-target-audio-offer").await;
    wait_native_operation(
        &f,
        &source_config.credential,
        &cid,
        "screen-source-audio-offer",
    )
    .await;
    stream_control(
        &mut target,
        &cid,
        "screen-target-audio",
        "screen-select-audio",
        1,
        "stream.subscribe",
        json!({"subscriptions":[{"strm_id":"screen-source-audio"}]}),
    )
    .await;
    let source_audio_handle = source_audio.original()["payload"]["substrate_setup"]["streams"][0]
        ["stream_local_id"]
        .as_u64()
        .unwrap() as u16;
    let target_audio_handle = target_audio.original()["payload"]["substrate_setup"]["streams"][0]
        ["stream_local_id"]
        .as_u64()
        .unwrap() as u16;
    let mut clients = vec![source, target];
    assert_audio_reaches_other_participant(
        &mut clients,
        source_audio_handle,
        &[target_audio_handle],
        21000,
    )
    .await;
    let upgrade = Request {
        operation_id: "accepted-live-screen-upgrade".into(),
        conversation_id: cid.clone(),
        session_id: "native-session".into(),
        by_participant: None,
        expected_revision: Some(1),
        action: Action::Upgrade {
            media: vec!["screen".into()],
        },
    };
    let upgraded = clients[1].session_command(upgrade.clone()).await.unwrap();
    let Outcome::SessionControl(result) = &upgraded else {
        panic!()
    };
    assert_eq!(result.session.media, vec!["audio", "screen"]);
    wait_slots(&f, 2).await;
    assert_audio_reaches_other_participant(
        &mut clients,
        source_audio_handle,
        &[target_audio_handle],
        21200,
    )
    .await;
    if refusal != 0 {
        clients[0].send_value("connection.offer",json!({"substrate":"quic","capabilities":av_capabilities(),"streams_offered":[{"id":"denied-screen","kind":"screen","direction":"sendonly","codec_preferences":["vp8"]}]}),CommandScope {conversation_id:Some(cid.clone()),session_id:Some("native-session".into()),connection_id:Some("denied-screen-slot".into()),operation_id:Some("denied-screen-offer".into()),expected_revision:Some(0)}).await.unwrap();
        let failed = clients[0].receive_control().await.unwrap();
        assert_eq!(failed.kind(), "operation.updated");
        assert_eq!(failed.original()["payload"]["state"], "failed");
        assert_eq!(clients[0].receive_control().await.unwrap().kind(), "ack");
        let credential = f
            .server
            .store
            .authenticate(&source_config.credential)
            .unwrap()
            .credential_id;
        let failed = f
            .server
            .store
            .execute_operation_action(
                &credential,
                connect_protocol::transitions::OperationAction::Get {
                    operation_id: "denied-screen-offer".into(),
                },
            )
            .unwrap();
        assert!(failed.effects.is_empty());
        wait_slots(&f, 2).await;
        assert_audio_reaches_other_participant(
            &mut clients,
            source_audio_handle,
            &[target_audio_handle],
            21400,
        )
        .await;
        drop(clients);
        f.server.shutdown().await.unwrap();
        return;
    }
    let source_screen=negotiate_av(&mut clients[0],&cid,"screen-source-offer","screen-source",json!([{"id":"shared-desktop","kind":"screen","direction":"sendonly","codec_preferences":["vp8"]}])).await;
    let target_screen=negotiate_av(&mut clients[1],&cid,"screen-target-offer","screen-target",json!([{"id":"desktop-monitor","kind":"screen","direction":"recvonly","codec_preferences":["vp8"]}])).await;
    wait_native_operation(&f, &source_config.credential, &cid, "screen-source-offer").await;
    wait_native_operation(&f, &f.client.credential, &cid, "screen-target-offer").await;
    assert_eq!(
        source_screen.original()["payload"]["substrate_setup"]["streams"][0]["kind"],
        "screen"
    );
    assert_eq!(
        target_screen.original()["payload"]["substrate_setup"]["streams"][0]["kind"],
        "screen"
    );
    let callback = stream_control(
        &mut clients[1],
        &cid,
        "screen-target",
        "select-shared-desktop",
        1,
        "stream.subscribe",
        if participant_selector {
            json!({"subscriptions":[{"from_participant":source_screen.original()["payload"]["by_participant"],"kinds":["screen"]}]})
        } else {
            json!({"subscriptions":[{"strm_id":"shared-desktop"}]})
        },
    )
    .await
    .unwrap();
    let binding = callback.original()["payload"]["substrate_setup"]["streams"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["strm_id"] == "shared-desktop")
        .unwrap();
    assert_eq!(binding["kind"], "screen");
    let target_handle = binding["stream_local_id"].as_u64().unwrap() as u16;
    let source_handle = source_screen.original()["payload"]["substrate_setup"]["streams"][0]
        ["stream_local_id"]
        .as_u64()
        .unwrap() as u16;
    let (sources, targets) = clients.split_at_mut(1);
    assert_vp8_routes(&sources[0], &mut targets[0], source_handle, target_handle).await;
    wait_slots(&f, 4).await;
    assert_audio_reaches_other_participant(
        &mut clients,
        source_audio_handle,
        &[target_audio_handle],
        21600,
    )
    .await;
    assert_eq!(clients[1].session_command(upgrade).await.unwrap(), upgraded);
    end_native_connection(
        &mut clients[1],
        &cid,
        "screen-target",
        "end-screen-target",
        None,
        "completed",
    )
    .await;
    wait_slots(&f, 3).await;
    assert_audio_reaches_other_participant(
        &mut clients,
        source_audio_handle,
        &[target_audio_handle],
        21800,
    )
    .await;
    drop(clients);
    f.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn production_native_reliable_data_capability_cannot_grant_an_unknown_protocol_or_disturb_audio(
) {
    assert_eq!(
        connect_protocol::native_data::MAX_BODY_BYTES,
        rvoip_core::MAX_DATA_MESSAGE_BYTES
    );
    let f = Fixture::with_native_stream_scope("EdDSA", 12).await;
    ready_endpoint_with_capabilities(&f, vec!["audio".into(), "data".into()]).await;
    let source_config = party_with_capabilities(
        &f,
        &f.peer.identity_id,
        88,
        false,
        vec!["audio".into(), "data".into()],
    );
    let mut target = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut source = CanonicalClient::open(source_config.clone()).await.unwrap();
    let cid = session_with_media(
        &f,
        &mut target,
        "voice",
        json!({"audio":["g.711-mu"],"data":["reliable-ordered"]}),
    )
    .await;
    let target_audio=negotiate_av(&mut target,&cid,"data-target-audio-offer","data-target-audio",json!([{"id":"data-monitor-audio","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}])).await;
    let source_audio=negotiate_av(&mut source,&cid,"data-source-audio-offer","data-source-audio",json!([{"id":"data-source-audio","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}])).await;
    wait_native_operation(&f, &f.client.credential, &cid, "data-target-audio-offer").await;
    wait_native_operation(
        &f,
        &source_config.credential,
        &cid,
        "data-source-audio-offer",
    )
    .await;
    stream_control(
        &mut target,
        &cid,
        "data-target-audio",
        "data-select-audio",
        1,
        "stream.subscribe",
        json!({"subscriptions":[{"strm_id":"data-source-audio"}]}),
    )
    .await;
    let source_handle = source_audio.original()["payload"]["substrate_setup"]["streams"][0]
        ["stream_local_id"]
        .as_u64()
        .unwrap() as u16;
    let target_handle = target_audio.original()["payload"]["substrate_setup"]["streams"][0]
        ["stream_local_id"]
        .as_u64()
        .unwrap() as u16;
    let mut clients = vec![source, target];
    assert_audio_reaches_other_participant(&mut clients, source_handle, &[target_handle], 23000)
        .await;
    clients[1]
        .session_command(Request {
            operation_id: "accepted-live-data-upgrade".into(),
            conversation_id: cid.clone(),
            session_id: "native-session".into(),
            by_participant: None,
            expected_revision: Some(1),
            action: Action::Upgrade {
                media: vec!["data".into()],
            },
        })
        .await
        .unwrap();
    clients[0].send_value("connection.offer",json!({"substrate":"quic","capabilities":{"data_protocols":["not-an-installed-protocol"],"max_streams_per_connection":8,"identity_assurance_offered":"identified"},"streams_offered":[{"id":"application","kind":"data","direction":"sendonly","codec_preferences":["not-an-installed-protocol"]}]}),CommandScope {conversation_id:Some(cid.clone()),session_id:Some("native-session".into()),connection_id:Some("unknown-data-protocol".into()),operation_id:Some("unknown-data-protocol-offer".into()),expected_revision:Some(0)}).await.unwrap();
    let failed = clients[0].receive_control().await.unwrap();
    assert_eq!(failed.kind(), "operation.updated");
    assert_eq!(failed.original()["payload"]["state"], "failed");
    assert_eq!(clients[0].receive_control().await.unwrap().kind(), "ack");
    let credential = f
        .server
        .store
        .authenticate(&source_config.credential)
        .unwrap()
        .credential_id;
    let record = f
        .server
        .store
        .execute_operation_action(
            &credential,
            connect_protocol::transitions::OperationAction::Get {
                operation_id: "unknown-data-protocol-offer".into(),
            },
        )
        .unwrap();
    assert!(record.effects.is_empty());
    wait_slots(&f, 2).await;
    assert_audio_reaches_other_participant(&mut clients, source_handle, &[target_handle], 23200)
        .await;
    drop(clients);
    f.server.shutdown().await.unwrap();
}
