//! Independent canonical WT media against actual Store/adapter/Core bindings.
use super::*;
use connect_client::{
    canonical::CanonicalHttpClient,
    session_control::{Action, Request},
};
use rvoip_uctp::substrate::{pack_rtp_datagram, unpack_rtp_datagram, RtpDatagram, RtpMediaPayload};

fn endpoint(f: &Fixture, config: &CanonicalClientConfig, index: usize) {
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
                address: f.server.webtransport_url.clone().unwrap(),
                capabilities: vec!["audio".into()],
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
fn other(f: &Fixture) -> CanonicalClientConfig {
    let admin = f.server.store.authenticate(&f.admin).unwrap().credential_id;
    let issued = f
        .server
        .store
        .issue(
            &f.peer.identity_id,
            Scope::collaboration(vec!["*".into()]),
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
    endpoint(f, &config, 1);
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
async fn accepted(f: &Fixture, peer: &CanonicalClientConfig) -> String {
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
                medium: "voice".into(),
                intent: "conference".into(),
                capabilities: json!({"audio":["g.711-mu"]}),
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
            action: Action::Accept {
                capabilities: json!({"audio":["g.711-mu"]}),
            },
        })
        .await
        .unwrap();
    client.shutdown().await.unwrap();
    room.id
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
    async fn offer(&mut self, cid: &str, index: usize) -> u16 {
        let operation = format!("http3-media-offer-{index}");
        let connection = format!("http3-media-connection-{index}");
        let mut offer = envelope(
            "connection.offer",
            json!({"substrate":"webtransport","capabilities":caps(),"streams_offered":[{"id":"voice","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}]}),
        );
        scopes(&mut offer, cid, &operation, &connection);
        offer["expected_revision"] = json!(0);
        let request = self.send(offer, true).await;
        let mut local = None;
        let mut callback = false;
        for _ in 0..128 {
            let frame = self.read().await;
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
                    assert!(!callback);
                    callback = true;
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
                }
                "ack" if frame.original()["in_reply_to"] == request => {
                    assert!(callback);
                    return local.unwrap();
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
async fn decoded(from: &Http3Peer, to: &Http3Peer, source: u16, target: u16, start: u32) {
    let send = async {
        for seq in start..start + 80 {
            let payload = (0..160)
                .map(|n| if n % 16 < 8 { 0x80 } else { 0x00 })
                .collect::<Vec<_>>();
            let frame = pack_rtp_datagram(&RtpDatagram {
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
            .unwrap();
            from._session.send_datagram(frame.into()).unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    let receive = tokio::time::timeout(Duration::from_secs(4), async {
        for _ in 0..256 {
            let bytes = to._session.read_datagram().await.unwrap();
            let frame = unpack_rtp_datagram(&bytes).unwrap();
            assert_eq!(frame.stream_local_id, target);
            assert_eq!(frame.rtp.payload_type, 0);
            let pcm = frame
                .rtp
                .payload
                .iter()
                .map(|b| decode_ulaw(*b))
                .collect::<Vec<_>>();
            if pcm.iter().any(|s| *s > 1000) && pcm.iter().any(|s| *s < -1000) {
                return;
            }
        }
        panic!("no independently decoded bipolar audio in bounded received frames")
    });
    let (_, result) = tokio::join!(send, receive);
    result.expect("actual authorized decoded WT duplex must reach the other Participant");
}
async fn duplex(algorithm: &str) {
    let f = fixture(algorithm).await;
    endpoint(&f, &f.client, 0);
    let peer = other(&f);
    let cid = accepted(&f, &peer).await;
    let mut first = Http3Peer::open(&f).await;
    let mut second = Http3Peer::open_config(&f, &peer).await;
    let a = first.offer(&cid, 0).await;
    let b = second.offer(&cid, 1).await;
    decoded(&first, &second, a, b, 1).await;
    decoded(&second, &first, b, a, 1001).await;
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
