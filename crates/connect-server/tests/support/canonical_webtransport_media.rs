//! Independent canonical WT media against actual Store/adapter/Core bindings.
use super::*;
use connect_client::{
    canonical::CanonicalHttpClient,
    session_control::{Action, Request},
};
use rvoip_uctp::substrate::{pack_rtp_datagram, unpack_rtp_datagram, RtpDatagram, RtpMediaPayload};

pub(super) fn endpoint(f: &Fixture, config: &CanonicalClientConfig, index: usize) {
    endpoint_caps(f, config, index, vec!["audio".into()]);
}
pub(super) fn endpoint_caps(
    f: &Fixture,
    config: &CanonicalClientConfig,
    index: usize,
    capabilities: Vec<String>,
) {
    let admin = f.server.store.authenticate(&f.admin).unwrap().credential_id;
    let ProvisioningOutcome::Endpoint { endpoint_id, .. } = f
        .server
        .store
        .execute_provisioning(
            &admin,
            ProvisioningCommand::Endpoint {
                operation_id: format!("wt-endpoint-{index}"),
                identity_id: config.identity_id.clone(),
                device: DeviceSpec {
                    id: config.device_id.clone(),
                    kind: config.device_kind.clone(),
                    platform: config.device_platform.clone(),
                },
                transport: EndpointTransport::Webtransport,
                // Logical Device endpoint URIs are unique; the actual physical
                // signaling contact remains the configured /connect mount.
                address: format!(
                    "{}/device-{index}",
                    f.server.webtransport_url.as_ref().unwrap()
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
                operation_id: format!("wt-verify-{index}"),
                endpoint_id,
                expected_revision: 0,
                evidence_ref: "designated-local-actual-http3-fixture".into(),
            },
        )
        .unwrap();
}
pub(super) fn other(f: &Fixture) -> CanonicalClientConfig {
    other_caps(f, vec!["audio".into()])
}
pub(super) fn other_caps(f: &Fixture, capabilities: Vec<String>) -> CanonicalClientConfig {
    let admin = f.server.store.authenticate(&f.admin).unwrap().credential_id;
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope
        .operations
        .extend(["stream.subscribe".into(), "stream.unsubscribe".into()]);
    let issued = f
        .server
        .store
        .issue(
            &f.peer.identity_id,
            scope,
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let mut config = f.client.clone();
    config.identity_id = f.peer.identity_id.clone();
    config.credential = issued.token;
    config.device_id = "http3-other-device".into();
    config.signing_key_id = "http3-other-key".into();
    config.signing_private_key = f.temp.path().join("http3-other-key.der");
    let public = signing(&config.signing_private_key, &config.signing_algorithm);
    endpoint_caps(f, &config, 1, capabilities);
    f.server
        .store
        .execute_provisioning(
            &admin,
            ProvisioningCommand::SigningKey {
                operation_id: "http3-other-enrollment".into(),
                identity_id: config.identity_id.clone(),
                credential_id: issued.id,
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
pub(super) async fn accepted(
    f: &Fixture,
    peer: &CanonicalClientConfig,
) -> (String, String, String) {
    accepted_caps(f, peer, "voice", json!({"audio":["g.711-mu"]})).await
}
pub(super) async fn accepted_caps(
    f: &Fixture,
    peer: &CanonicalClientConfig,
    medium: &str,
    capabilities: Value,
) -> (String, String, String) {
    let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut input =
        connect_client::conversations::CreateRequest::persistent(vec![peer.identity_id.clone()]);
    input.tenant_id = Some("tnt_default".into());
    let room = client
        .create_conversation("http3-media-room", input)
        .await
        .unwrap();
    let party = room
        .participants
        .iter()
        .find(|p| p.identity_id == peer.identity_id)
        .unwrap()
        .participant_id
        .clone();
    client
        .session_command(Request {
            operation_id: "http3-media-invite".into(),
            conversation_id: room.id.clone(),
            session_id: "http3-media-session".into(),
            by_participant: None,
            expected_revision: None,
            action: Action::Invite {
                participants: vec![party],
                medium: medium.into(),
                intent: "conference".into(),
                capabilities: capabilities.clone(),
            },
        })
        .await
        .unwrap();
    let mut http = canonical_http_config(f);
    http.credential = peer.credential.clone();
    CanonicalHttpClient::open(http)
        .await
        .unwrap()
        .session_command(Request {
            operation_id: "http3-media-accept".into(),
            conversation_id: room.id.clone(),
            session_id: "http3-media-session".into(),
            by_participant: None,
            expected_revision: Some(0),
            action: Action::Accept { capabilities },
        })
        .await
        .unwrap();
    let initiator = room
        .participants
        .iter()
        .find(|p| p.identity_id == f.client.identity_id)
        .unwrap()
        .participant_id
        .clone();
    let recipient = room
        .participants
        .iter()
        .find(|p| p.identity_id == peer.identity_id)
        .unwrap()
        .participant_id
        .clone();
    client.shutdown().await.unwrap();
    (room.id, initiator, recipient)
}
fn scopes(frame: &mut Value, cid: &str, operation: &str, connection: &str) {
    frame["cid"] = json!(cid);
    frame["sid"] = json!("http3-media-session");
    frame["connid"] = json!(connection);
    frame["operation_id"] = json!(operation);
}
fn caps() -> Value {
    json!({"audio_codecs":[{"name":"pcmu","params":{"sample_rate":8000,"channels":1}}],"max_streams_per_connection":8,"identity_assurance_offered":"identified"})
}
impl Http3Peer {
    pub(super) async fn completed(&mut self, cid: &str, operation: &str) {
        let until = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let mut request = envelope("operation.get", json!({"operation_id": operation}));
            request["cid"] = json!(cid);
            let id = self.send(request, true).await;
            let mut state = None;
            let mut done = false;
            for _ in 0..128 {
                let frame = self.read().await;
                if frame.original()["in_reply_to"] != id {
                    continue;
                }
                assert_ne!(frame.kind(), "error", "{:?}", frame.original());
                if frame.kind() == "operation.updated" {
                    assert_eq!(frame.original()["payload"]["operation_id"], operation);
                    state = frame.original()["payload"]["state"]
                        .as_str()
                        .map(str::to_owned);
                }
                if frame.kind() == "ack" {
                    done = true;
                    break;
                }
            }
            assert!(done, "bounded current operation reply must end in ACK");
            if state.as_deref() == Some("completed") {
                return;
            }
            assert!(
                tokio::time::Instant::now() < until,
                "native operation did not complete: {state:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    pub(super) async fn subscribe(
        &mut self,
        cid: &str,
        index: usize,
        source: &str,
        mut handle: u16,
    ) -> u16 {
        let operation = format!("http3-subscribe-{index}");
        let connection = format!("http3-media-connection-{index}");
        let mut request = envelope(
            "stream.subscribe",
            json!({"subscriptions":[{"from_participant":source,"kinds":["audio"]}]}),
        );
        scopes(&mut request, cid, &operation, &connection);
        request["expected_revision"] = json!(1);
        let id = self.send(request, true).await;
        let mut completed = false;
        for _ in 0..128 {
            let frame = self.read().await;
            assert_ne!(frame.kind(), "error", "{:?}", frame.original());
            if frame.kind() == "connection.offer" {
                assert_eq!(frame.original()["cid"], cid);
                assert_eq!(frame.original()["sid"], "http3-media-session");
                assert_eq!(frame.original()["connid"], connection);
                assert_eq!(frame.original()["operation_id"], operation);
                let p = &frame.original()["payload"];
                assert_eq!(p["substrate"], "webtransport");
                assert_eq!(p["substrate_setup"]["offer_id"], id);
                let setup = p["substrate_setup"]["streams"].as_array().unwrap();
                handle = setup.iter().find(|s| s["kind"] == "audio").unwrap()["stream_local_id"]
                    .as_u64()
                    .unwrap() as u16;
                let streams = setup.iter().map(|s|json!({"id":s["strm_id"],"kind":s["kind"],"direction":match s["direction"].as_str().unwrap(){"sendonly"=>"recvonly","recvonly"=>"sendonly",_=>"sendrecv"},"codec":s["codec"]})).collect::<Vec<_>>();
                let mut response = envelope(
                    "connection.answer",
                    json!({"by_participant":p["by_participant"],"substrate":"webtransport","capabilities":caps(),"streams_answered":streams,"substrate_setup":p["substrate_setup"]}),
                );
                scopes(&mut response, cid, &operation, &connection);
                response["in_reply_to"] = json!(frame.id());
                self.send(response, true).await;
            } else if frame.original()["in_reply_to"] == id {
                if frame.kind() == "operation.updated" {
                    assert_eq!(frame.original()["payload"]["operation_id"], operation);
                    assert_eq!(frame.original()["payload"]["state"], "completed");
                    completed = true;
                }
                if frame.kind() == "ack" {
                    assert!(completed);
                    return handle;
                }
            }
        }
        panic!("bounded signed subscription reply did not complete");
    }
    pub(super) async fn offer(&mut self, cid: &str, index: usize) -> u16 {
        let operation = format!("http3-media-offer-{index}");
        let connection = format!("http3-media-connection-{index}");
        let mut offer = envelope(
            "connection.offer",
            json!({"substrate":"webtransport","capabilities":caps(),"streams_offered":[{"id":format!("http3-voice-{index}"),"kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}]}),
        );
        scopes(&mut offer, cid, &operation, &connection);
        offer["expected_revision"] = json!(0);
        let request = self.send(offer, true).await;
        eprintln!("HTTP3 media fixture stage: sent offer {index}");
        let mut local = None;
        for _ in 0..128 {
            let frame = self.read().await;
            eprintln!(
                "HTTP3 media fixture stage: offer {index} received {}",
                frame.kind()
            );
            assert_ne!(frame.kind(), "error", "{:?}", frame.original());
            match frame.kind() {
                "connection.answer" if frame.original()["in_reply_to"] == request => {
                    assert_eq!(frame.original()["payload"]["substrate"], "webtransport");
                    local = Some(
                        frame.original()["payload"]["substrate_setup"]["streams"][0]
                            ["stream_local_id"]
                            .as_u64()
                            .unwrap() as u16,
                    );
                }
                "connection.offer" => {
                    assert_eq!(frame.original()["cid"], cid);
                    assert_eq!(frame.original()["sid"], "http3-media-session");
                    assert_eq!(frame.original()["connid"], connection);
                    assert_eq!(frame.original()["operation_id"], operation);
                    let p = &frame.original()["payload"];
                    assert_eq!(p["substrate"], "webtransport");
                    let streams=p["substrate_setup"]["streams"].as_array().unwrap().iter().map(|s|json!({"id":s["strm_id"],"kind":s["kind"],"direction":match s["direction"].as_str().unwrap(){"sendonly"=>"recvonly","recvonly"=>"sendonly",_=>"sendrecv"},"codec":s["codec"]})).collect::<Vec<_>>();
                    let mut answer = envelope(
                        "connection.answer",
                        json!({"by_participant":p["by_participant"],"substrate":"webtransport","capabilities":caps(),"streams_answered":streams,"substrate_setup":p["substrate_setup"]}),
                    );
                    scopes(&mut answer, cid, &operation, &connection);
                    answer["in_reply_to"] = json!(frame.id());
                    self.send(answer, true).await;
                    // connection.answer terminates the original wire offer;
                    // native completion is proven by its current operation,
                    // not by inventing an additional root ACK.
                    self.completed(cid, &operation).await;
                    return local.expect("missing negotiated native handle");
                }
                _ => {}
            }
        }
        panic!("bounded independent HTTP3 media exchange exceeded its reply count")
    }
}
// Independent G.711 decoder: do not count upstream packet parsing alone as
// decoded output. Both signs of the injected square wave must survive mixing.
fn decode_ulaw(byte: u8) -> i32 {
    let value = !byte;
    let magnitude = (((value & 15) as i32 * 8 + 132) << ((value >> 4) & 7)) - 132;
    if value & 128 != 0 {
        -magnitude
    } else {
        magnitude
    }
}
pub(super) fn tone_packet(source: u16, seq: u32, period: usize) -> Vec<u8> {
    let payload = (0..160)
        .map(|n| if n % period < period / 2 { 0x80 } else { 0x00 })
        .collect::<Vec<_>>();
    pack_rtp_datagram(&RtpDatagram {
        flags: 0,
        stream_local_id: source,
        seq,
        rtp: RtpMediaPayload {
            payload: payload.into(),
            payload_type: 0,
            sequence_number: seq as u16,
            timestamp: seq * 160,
            ssrc: 0x4736_0049,
        },
    })
    .unwrap()
    .to_vec()
}
pub(super) fn tone_decoded(bytes: &[u8], target: u16, period: usize) -> bool {
    let frame = unpack_rtp_datagram(bytes).unwrap();
    assert_eq!(frame.stream_local_id, target);
    assert_eq!(frame.rtp.payload_type, 0);
    let pcm = frame
        .rtp
        .payload
        .iter()
        .map(|b| decode_ulaw(*b))
        .collect::<Vec<_>>();
    let energetic = pcm
        .iter()
        .filter(|sample| sample.abs() > 500)
        .copied()
        .collect::<Vec<_>>();
    let changes = energetic
        .windows(2)
        .filter(|p| p[0].is_positive() != p[1].is_positive())
        .count();
    let rate = changes as f64 / energetic.len().max(1) as f64;
    let expected = 2.0 / period as f64;
    pcm.iter().any(|s| *s > 1000)
        && pcm.iter().any(|s| *s < -1000)
        && energetic.len() as f64 >= pcm.len() as f64 * 0.85
        && rate >= expected * 0.8
        && rate <= expected * 1.2
}
async fn decoded(
    from: &Http3Peer,
    to: &Http3Peer,
    source: u16,
    target: u16,
    start: u32,
    period: usize,
) {
    let send = async {
        for seq in start..start + 80 {
            from._session
                .send_datagram(tone_packet(source, seq, period).into())
                .unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    let receive = tokio::time::timeout(Duration::from_secs(4), async {
        for _ in 0..256 {
            let bytes = to._session.read_datagram().await.unwrap();
            if tone_decoded(&bytes, target, period) {
                return;
            }
        }
        panic!("no independently decoded bipolar audio in bounded received frames");
    });
    let (_, result) = tokio::join!(send, receive);
    result.expect("actual authorized decoded WT duplex must reach the other Participant");
}
async fn duplex(algorithm: &str) {
    duplex_mode(algorithm, None).await;
}
async fn duplex_mode(algorithm: &str, server_end: Option<bool>) {
    static LOGS: std::sync::Once = std::sync::Once::new();
    LOGS.call_once(|| {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "connect_service=debug,rvoip_uctp=debug".into()),
            )
            .with_test_writer()
            .try_init();
    });
    eprintln!("HTTP3 media fixture stage: start {algorithm}");
    let f = fixture(algorithm).await;
    endpoint(&f, &f.client, 0);
    let peer = other(&f);
    eprintln!("HTTP3 media fixture stage: endpoints and enrollment ready");
    let (cid, initiator, recipient) = accepted(&f, &peer).await;
    eprintln!("HTTP3 media fixture stage: Session accepted");
    let mut first = Http3Peer::open(&f).await;
    eprintln!("HTTP3 media fixture stage: first peer authenticated");
    let mut second = Http3Peer::open_config(&f, &peer).await;
    eprintln!("HTTP3 media fixture stage: second peer authenticated");
    let a = first.offer(&cid, 0).await;
    let b = second.offer(&cid, 1).await;
    let a = first.subscribe(&cid, 0, &recipient, a).await;
    let b = second.subscribe(&cid, 1, &initiator, b).await;
    decoded(&first, &second, a, b, 1, 16).await;
    decoded(&second, &first, b, a, 1001, 32).await;
    let (started, _) = f
        .server
        .store
        .session_started_template(&cid, "http3-media-session")
        .unwrap();
    assert_eq!(started.active_connections.len(), 2);
    assert!(started
        .active_connections
        .iter()
        .all(|c| c.transport == "webtransport"));
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let mut statement=db.prepare("SELECT e.transport,o.state,e.device_id FROM connections n JOIN endpoints e ON e.id=n.endpoint_id JOIN operations o ON o.connection_id=n.id WHERE o.command_type='connection.offer' ORDER BY e.device_id").unwrap();
    let rows = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows
        .iter()
        .all(|r| r.0 == "webtransport" && r.1 == "completed"));
    drop(statement);
    drop(db);
    if let Some(graceful) = server_end {
        let service = f.server.service.clone();
        if graceful {
            f.server.shutdown().await.unwrap();
        } else {
            drop(f.server);
        }
        tokio::time::timeout(Duration::from_secs(5), first.connection.closed())
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), second.connection.closed())
            .await
            .unwrap();
        drop(first);
        drop(second);
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let s = service.canonical_runtime_snapshot().unwrap();
                if s.physical_peers == 0 && s.connections == 0 && s.media_readers == 0 {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("owned server end must retire live decoded WebTransport routes");
        return;
    }
    first.close().await;
    second.close().await;
    drop(first);
    drop(second);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let s = f.server.service.canonical_runtime_snapshot().unwrap();
            if s.physical_peers == 0 && s.connections == 0 && s.media_readers == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    // New physical readers recover original durable media status; reading a
    // completed offer must not allocate a replacement route or relabel it.
    // Adapter retirement precedes the tracked Store worker's final commit.
    // Wait for authoritative cleanup, including the retired lifetime, before
    // comparing durable state across read-only status recovery.
    tokio::time::timeout(Duration::from_secs(5),async{loop{
        let c=rusqlite::Connection::open_with_flags(&f.config.database,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let retired:u64=c.query_row("SELECT count(*) FROM connections n JOIN native_connection_lifetimes life ON life.connection_id=n.id WHERE n.session_id='http3-media-session' AND n.state='retired' AND n.observed_runtime_id IS NULL AND life.retired=1",[],|r|r.get(0)).unwrap();
        if retired==2{return;}tokio::time::sleep(Duration::from_millis(20)).await;
    }}).await.expect("both original media lifetimes must commit retirement before status reads");
    let before = connect_store::Store::inspect_snapshot(&f.config.database).unwrap();
    let expected_record: String = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap()
    .query_row(
        "SELECT record_id FROM operations WHERE owner_id=?1 AND operation_id='http3-media-offer-0'",
        [&f.client.identity_id],
        |r| r.get(0),
    )
    .unwrap();
    let mut fresh = Http3Peer::open(&f).await;
    let mut request = envelope(
        "operation.get",
        json!({"operation_id":"http3-media-offer-0"}),
    );
    request["cid"] = json!(cid);
    let id = fresh.send(request, true).await;
    let mut observed = false;
    let mut done = false;
    for _ in 0..128 {
        let frame = fresh.read().await;
        if frame.original()["in_reply_to"] != id {
            continue;
        }
        assert_ne!(frame.kind(), "error", "{:?}", frame.original());
        if frame.kind() == "operation.updated" {
            assert_eq!(frame.original()["payload"]["record_id"], expected_record);
            assert_eq!(frame.original()["payload"]["state"], "completed");
            observed = true;
        }
        if frame.kind() == "ack" {
            done = true;
            break;
        }
    }
    assert!(done && observed);
    let mut native = CanonicalClient::open(f.client.clone()).await.unwrap();
    let retained = native
        .operation_get(
            "http3-media-offer-0",
            connect_client::canonical::CommandScope {
                conversation_id: Some(cid.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let retained = serde_json::to_value(retained).unwrap();
    assert_eq!(retained["record_id"], expected_record);
    assert_eq!(retained["state"], "completed");
    assert_eq!(
        f.server
            .service
            .canonical_runtime_snapshot()
            .unwrap()
            .connections,
        0
    );
    assert_eq!(
        connect_store::Store::inspect_snapshot(&f.config.database).unwrap(),
        before
    );
    native.shutdown().await.unwrap();
    fresh.close().await;
    drop(fresh);
    super::canonical_webtransport_lifecycle::drained(&f).await;
    f.server.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn independent_http3_actual_decoded_duplex_uses_webtransport_bindings_eddsa() {
    duplex("EdDSA").await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn independent_http3_actual_decoded_duplex_uses_webtransport_bindings_es256() {
    duplex("ES256").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_shutdown_with_live_decoded_webtransport_media_retires_all_physical_bindings() {
    duplex_mode("EdDSA", Some(true)).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_drop_with_live_decoded_webtransport_media_retires_all_physical_bindings() {
    duplex_mode("ES256", Some(false)).await;
}
