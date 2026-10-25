//! Public native SDK and independent HTTP3 media on the same production Core.
use super::canonical_webtransport_media::{tone_decoded, tone_packet};
use super::*;
use connect_client::canonical::CommandScope;
fn caps() -> Value {
    json!({"audio_codecs":[{"name":"pcmu","params":{"sample_rate":8000,"channels":1}}],"max_streams_per_connection":8,"identity_assurance_offered":"identified"})
}
fn scope(cid: &str, operation: &str, revision: Option<u64>) -> CommandScope {
    CommandScope {
        conversation_id: Some(cid.into()),
        session_id: Some("http3-media-session".into()),
        connection_id: Some("mixed-native-connection".into()),
        operation_id: Some(operation.into()),
        expected_revision: revision,
    }
}
async fn next(client: &mut CanonicalClient) -> Envelope {
    for _ in 0..128 {
        let frame = client.receive().await.unwrap();
        if !matches!(
            frame.kind(),
            "session.started"
                | "session.ended"
                | "session.participant.joined"
                | "session.participant.left"
                | "connection.ready"
                | "stream.opened"
                | "stream.closed"
                | "connection.quality"
                | "recording.vcon-ready"
        ) {
            return frame;
        }
    }
    panic!("bounded native control receive count");
}
async fn answer(
    client: &mut CanonicalClient,
    frame: &Envelope,
    root: &CommandScope,
    id: &str,
) -> u16 {
    for (field, expected) in [
        ("cid", root.conversation_id.as_ref().unwrap()),
        ("sid", root.session_id.as_ref().unwrap()),
        ("connid", root.connection_id.as_ref().unwrap()),
        ("operation_id", root.operation_id.as_ref().unwrap()),
    ] {
        assert_eq!(frame.original()[field], *expected);
    }
    let p = &frame.original()["payload"];
    assert_eq!(p["substrate"], "quic");
    assert_eq!(p["substrate_setup"]["offer_id"], id);
    let setup = p["substrate_setup"]["streams"].as_array().unwrap();
    let local = setup.iter().find(|s| s["kind"] == "audio").unwrap()["stream_local_id"]
        .as_u64()
        .unwrap() as u16;
    let streams=setup.iter().map(|s|json!({"id":s["strm_id"],"kind":s["kind"],"direction":match s["direction"].as_str().unwrap(){"sendonly"=>"recvonly","recvonly"=>"sendonly",_=>"sendrecv"},"codec":s["codec"]})).collect::<Vec<_>>();
    client.answer_connection_offer(frame,serde_json::from_value(json!({"by_participant":p["by_participant"],"substrate":"quic","capabilities":caps(),"streams_answered":streams,"substrate_setup":p["substrate_setup"]})).unwrap(),root).await.unwrap();
    local
}
async fn offer(client: &mut CanonicalClient, cid: &str) -> u16 {
    let root = scope(cid, "mixed-native-offer", Some(0));
    let id=client.send_value("connection.offer",json!({"substrate":"quic","capabilities":caps(),"streams_offered":[{"id":"mixed-native-voice","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}]}),root.clone()).await.unwrap();
    let response = next(client).await;
    assert_eq!(response.kind(), "connection.answer");
    assert_eq!(response.original()["in_reply_to"], id);
    assert_eq!(response.original()["payload"]["substrate"], "quic");
    let callback = next(client).await;
    assert_eq!(callback.kind(), "connection.offer");
    let local = answer(client, &callback, &root, &id).await;
    let until = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let current = client
            .operation_get(
                "mixed-native-offer",
                CommandScope {
                    conversation_id: Some(cid.into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        if serde_json::to_value(current).unwrap()["state"] == "completed" {
            return local;
        }
        assert!(tokio::time::Instant::now() < until);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
async fn subscribe(client: &mut CanonicalClient, cid: &str, source: &str, mut handle: u16) -> u16 {
    let root = scope(cid, "mixed-native-subscribe", Some(1));
    let id = client
        .send_value(
            "stream.subscribe",
            json!({"subscriptions":[{"from_participant":source,"kinds":["audio"]}]}),
            root.clone(),
        )
        .await
        .unwrap();
    let mut completed = false;
    for _ in 0..128 {
        let frame = next(client).await;
        assert_ne!(frame.kind(), "error", "{:?}", frame.original());
        if frame.kind() == "connection.offer" {
            handle = answer(client, &frame, &root, &id).await;
        } else if frame.original()["in_reply_to"] == id {
            if frame.kind() == "operation.updated" {
                assert_eq!(frame.original()["payload"]["state"], "completed");
                completed = true;
            }
            if frame.kind() == "ack" {
                assert!(completed);
                return handle;
            }
        }
    }
    panic!("bounded native signed subscription reply count");
}
async fn wt_to_native(
    wt: &Http3Peer,
    native: &mut CanonicalClient,
    source: u16,
    target: u16,
    start: u32,
) {
    let send = async {
        for seq in start..start + 80 {
            wt._session
                .send_datagram(tone_packet(source, seq, 16).into())
                .unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    let receive = tokio::time::timeout(Duration::from_secs(4), async {
        for _ in 0..256 {
            let packet = native.receive_native_rtp_datagram().await.unwrap();
            if tone_decoded(&packet, target, 16) {
                return;
            }
        }
        panic!("no independently decoded WT-to-native tone");
    });
    let (_, result) = tokio::join!(send, receive);
    result.unwrap();
}
async fn native_to_wt(
    native: &CanonicalClient,
    wt: &Http3Peer,
    source: u16,
    target: u16,
    start: u32,
) {
    let send = async {
        for seq in start..start + 80 {
            native
                .send_native_rtp_datagram(&tone_packet(source, seq, 32))
                .unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    let receive = tokio::time::timeout(Duration::from_secs(4), async {
        for _ in 0..256 {
            let packet = wt._session.read_datagram().await.unwrap();
            if tone_decoded(&packet, target, 32) {
                return;
            }
        }
        panic!("no independently decoded native-to-WT tone");
    });
    let (_, result) = tokio::join!(send, receive);
    result.unwrap();
}
async fn mixed(algorithm: &str) {
    let f = fixture(algorithm).await;
    super::canonical_webtransport_media::endpoint(&f, &f.client, 0);
    let other = super::canonical_webtransport_media::other(&f);
    let admin = f.server.store.authenticate(&f.admin).unwrap().credential_id;
    let ProvisioningOutcome::Endpoint { endpoint_id, .. } = f
        .server
        .store
        .execute_provisioning(
            &admin,
            ProvisioningCommand::Endpoint {
                operation_id: "mixed-native-endpoint".into(),
                identity_id: other.identity_id.clone(),
                device: DeviceSpec {
                    id: other.device_id.clone(),
                    kind: other.device_kind.clone(),
                    platform: other.device_platform.clone(),
                },
                transport: EndpointTransport::Quic,
                address: format!("quic://localhost:{}/mixed-native", f.server.address.port()),
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
                operation_id: "mixed-native-verify".into(),
                endpoint_id,
                expected_revision: 0,
                evidence_ref: "designated-actual-mixed-transport-fixture".into(),
            },
        )
        .unwrap();
    let (cid, initiator, recipient) =
        super::canonical_webtransport_media::accepted(&f, &other).await;
    let mut wt = Http3Peer::open(&f).await;
    let mut native = CanonicalClient::open(other).await.unwrap();
    let a = wt.offer(&cid, 0).await;
    let b = offer(&mut native, &cid).await;
    let a = wt.subscribe(&cid, 0, &recipient, a).await;
    let b = subscribe(&mut native, &cid, &initiator, b).await;
    wt_to_native(&wt, &mut native, a, b, 1).await;
    native_to_wt(&native, &wt, b, a, 1001).await;
    // A third authenticated physical peer cannot borrow another peer's live
    // numeric stream handle. Require actual ingress rejection before proving
    // that the two original authorized routes still decode both directions.
    let attacker = Http3Peer::open(&f).await;
    let before = f
        .server
        .service
        .canonical_runtime_snapshot()
        .unwrap()
        .media_datagrams_rejected;
    attacker
        ._session
        .send_datagram(tone_packet(a, 2001, 8).into())
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let s = f.server.service.canonical_runtime_snapshot().unwrap();
            assert_eq!(s.connections, 2);
            if s.media_datagrams_rejected > before {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    wt_to_native(&wt, &mut native, a, b, 3001).await;
    native_to_wt(&native, &wt, b, a, 4001).await;
    let (started, _) = f
        .server
        .store
        .session_started_template(&cid, "http3-media-session")
        .unwrap();
    assert_eq!(started.active_connections.len(), 2);
    assert_eq!(
        started
            .active_connections
            .iter()
            .map(|c| c.transport.as_str())
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from(["quic", "webtransport"])
    );
    attacker.close().await;
    drop(attacker);
    wt.close().await;
    drop(wt);
    native.shutdown().await.unwrap();
    super::canonical_webtransport_lifecycle::drained(&f).await;
    f.server.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_mixed_webtransport_and_native_duplex_survives_foreign_physical_handle_rejection_eddsa(
) {
    mixed("EdDSA").await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_mixed_webtransport_and_native_duplex_survives_foreign_physical_handle_rejection_es256(
) {
    mixed("ES256").await;
}
