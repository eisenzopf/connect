//! Actual Connect Store authority, production native flow and signed SDK events.
use super::*;

pub(super) fn observe(algorithm: &str, event: &Envelope) {
    if let Some(path) = std::env::var_os("CONNECT_NATIVE_LIFECYCLE_OBSERVATIONS") {
        use std::io::Write;
        let path = std::path::PathBuf::from(path);
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(".local/gate-6");
        assert!(path.is_absolute() && path.starts_with(root));
        let mut bytes =
            serde_json::to_vec(&json!({"algorithm":algorithm,"frame":event.original()})).unwrap();
        bytes.push(b'\n');
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap()
            .write_all(&bytes)
            .unwrap();
    }
}

#[tokio::test]
async fn production_native_lifecycle_quality_reports_actual_flow_through_signed_sdk() {
    for algorithm in ["EdDSA", "ES256"] {
        let mut f = Fixture::with_native_end_scope(algorithm, 12).await;
        f.client.timeout_ms = 8000;
        let (mut scene, ()) =
            codecs::setup_started(f, (), "lifecycle-target-offer", "lifecycle-source-offer").await;
        // An admitted/answered native offer alone is not media readiness. The
        // heartbeat is a real reliable-stream barrier and keeps any prior facts.
        for client in &mut scene.clients {
            let id = client.keepalive().await.unwrap();
            let frame = client.receive().await.unwrap();
            assert_eq!(frame.kind(), "ack", "no flow yet: {:?}", frame.original());
            assert_eq!(frame.original()["in_reply_to"], id);
        }
        assert_audio_reaches_other_participant(&mut scene.clients, scene.sh, &[scene.th], 70000)
            .await;
        scene.clients.swap(0, 1);
        assert_audio_reaches_other_participant(&mut scene.clients, scene.th, &[scene.sh], 71000)
            .await;
        scene.clients.swap(0, 1);
        // Exercise the existing collector while lifecycle frames are already
        // pending on the actual reliable stream. They must remain in its inbox.
        for (index, operation) in ["lifecycle-source-offer", "lifecycle-target-offer"]
            .into_iter()
            .enumerate()
        {
            let outcome = scene.clients[index]
                .operation_get(
                    operation,
                    CommandScope {
                        conversation_id: Some(scene.cid.clone()),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            assert_eq!(
                outcome.state,
                rvoip_uctp::v2::payloads::OperationState::Completed
            );
        }
        for (index, (connection, stream, handle)) in [
            ("codec-source", "codec-voice", scene.sh),
            ("codec-target", "codec-monitor", scene.th),
        ]
        .into_iter()
        .enumerate()
        {
            let mut ready = false;
            let mut opened = false;
            let mut measured = false;
            let end = tokio::time::Instant::now() + Duration::from_secs(8);
            while !ready || !opened || !measured {
                assert!(tokio::time::Instant::now() < end);
                let e = scene.clients[index].receive().await.unwrap();
                if session_start(&e) {
                    continue;
                }
                observe(algorithm, &e);
                // receive() verifies the actual enrolled server signature under
                // each configured signing algorithm before returning this fact.
                assert_eq!(e.original()["cid"], scene.cid);
                assert_eq!(e.original()["sid"], "native-session");
                assert_eq!(e.original()["connid"], connection);
                assert!(e.original().get("in_reply_to").is_none());
                assert!(e.original().get("operation_id").is_none());
                let p = &e.original()["payload"];
                match e.kind() {
                    "connection.ready" => {
                        assert!(!ready);
                        ready = true;
                    }
                    "stream.opened" => {
                        assert!(ready && !opened);
                        assert_eq!(p["stream"]["strm_id"], stream);
                        assert_eq!(p["stream"]["stream_local_id"], handle);
                        opened = true;
                    }
                    "connection.quality" => {
                        assert!(p["interval_ms"].as_u64().unwrap() >= 5000);
                        let samples = p["streams"].as_array().unwrap();
                        assert_eq!(samples.len(), 1);
                        let sample = &samples[0];
                        assert_eq!(sample["strm_id"], stream);
                        assert!(sample["packets_received"].as_u64().unwrap() > 0);
                        assert!(sample["packets_sent"].as_u64().unwrap() > 0);
                        assert!(sample["bitrate_bps"].as_u64().unwrap() > 0);
                        for absent in ["loss_pct", "jitter_ms", "rtt_ms", "mos"] {
                            assert!(sample.get(absent).is_none(), "unmeasured {absent}");
                        }
                        measured = true;
                    }
                    _ => panic!("unexpected lifecycle fact: {:?}", e.original()),
                }
            }
        }
        // Revoke only the target Endpoint through the actual public admin SDK.
        // An idle lifecycle registration must not park its old permission reader.
        let (endpoint, revision): (String, u64) =
            rusqlite::Connection::open(&scene.f.config.database)
                .unwrap()
                .query_row(
                    "SELECT e.id,e.revision FROM endpoints e JOIN connections n ON n.endpoint_id=e.id WHERE n.id='codec-target' AND e.device_id=?1",
                    [&scene.f.client.device_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
        let mut admin_config = canonical_http_config(&scene.f);
        admin_config.credential = scene.f.admin.clone();
        let mut admin = connect_client::application::ApplicationClient::open(admin_config)
            .await
            .unwrap();
        tokio::time::timeout(
            Duration::from_secs(3),
            admin.provision(ProvisioningCommand::RevokeEndpoint {
                operation_id: "revoke-lifecycle-target".into(),
                endpoint_id: endpoint,
                expected_revision: revision,
            }),
        )
        .await
        .unwrap()
        .unwrap();
        let revoked_at = chrono::Utc::now();
        assert!(scene
            .f
            .server
            .store
            .authenticate(&scene.f.client.credential)
            .is_ok());
        let end = tokio::time::Instant::now() + Duration::from_millis(6200);
        while tokio::time::Instant::now() < end {
            let id = scene.clients[1].keepalive().await.unwrap();
            loop {
                let e = scene.clients[1].receive().await.unwrap();
                if e.kind() == "ack" {
                    assert_eq!(e.original()["in_reply_to"], id);
                    break;
                }
                // Session metadata has independent current read authority;
                // endpoint revocation does not revoke that permission.
                if session_start(&e) {
                    continue;
                }
                // Bytes admitted before revocation can still be in the peer's
                // receive queue. They cannot establish a later current grant.
                assert_eq!(e.original()["connid"], "codec-target");
                let at = chrono::DateTime::parse_from_rfc3339(e.original()["ts"].as_str().unwrap())
                    .unwrap();
                assert!(at <= revoked_at, "revoked source emitted a fresh event");
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        // The other physical peer still receives its own freshly authorized
        // quality interval, even though its former recipient was revoked.
        let fresh = loop {
            let frame = scene.clients[0].receive().await.unwrap();
            if session_start(&frame) {
                continue;
            }
            break frame;
        };
        observe(algorithm, &fresh);
        assert_eq!(fresh.kind(), "connection.quality");
        assert_eq!(fresh.original()["connid"], "codec-source");
        assert!(
            chrono::DateTime::parse_from_rfc3339(fresh.original()["ts"].as_str().unwrap()).unwrap()
                > revoked_at
        );
        for client in scene.clients {
            client.shutdown().await.unwrap();
        }
        scene.f.server.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn production_native_lifecycle_closes_original_codec_binding_and_opens_only_observed_replacement(
) {
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_native_end_scope(algorithm, 12).await;
        let (mut scene, ()) =
            codecs::setup_started(f, (), "lifecycle-target-offer", "lifecycle-source-offer").await;
        assert_audio_reaches_other_participant(&mut scene.clients, scene.sh, &[scene.th], 74000)
            .await;
        scene.clients.swap(0, 1);
        assert_audio_reaches_other_participant(&mut scene.clients, scene.th, &[scene.sh], 75000)
            .await;
        scene.clients.swap(0, 1);
        loop {
            let e = scene.clients[1].receive().await.unwrap();
            if session_start(&e) {
                continue;
            }
            observe(algorithm, &e);
            assert_eq!(e.original()["connid"], "codec-target");
            if e.kind() == "stream.opened" {
                assert_eq!(
                    e.original()["payload"]["stream"]["stream_local_id"],
                    scene.th
                );
                break;
            }
            assert!(matches!(
                e.kind(),
                "connection.ready" | "connection.quality"
            ));
        }
        let scope = CommandScope {
            conversation_id: Some(scene.cid.clone()),
            session_id: Some("native-session".into()),
            connection_id: Some("codec-target".into()),
            operation_id: Some("lifecycle-pcma".into()),
            expected_revision: Some(2),
        };
        scene.clients[1].send_value("connection.update", json!({"action":"codec-renegotiate","streams":["codec-monitor"],"codec_preferences":["pcma"]}), scope.clone()).await.unwrap();
        let callback = loop {
            let e = scene.clients[1].receive().await.unwrap();
            if session_start(&e) {
                continue;
            }
            if e.kind() == "connection.offer" {
                break e;
            }
            assert_eq!(e.kind(), "connection.quality");
        };
        let binding = &callback.original()["payload"]["substrate_setup"]["streams"][0];
        assert_eq!(binding["codec"]["name"], "pcma");
        let changed = binding["stream_local_id"].as_u64().unwrap() as u16;
        assert_ne!(changed, scene.th);
        codecs::answer(&mut scene.clients[1], &callback, &scope, false).await;
        let mut events = Vec::new();
        loop {
            let e = scene.clients[1].receive().await.unwrap();
            match e.kind() {
                "operation.updated" => assert_eq!(e.original()["payload"]["state"], "completed"),
                "ack" => break,
                "stream.closed" | "stream.opened" | "connection.quality" => events.push(e),
                _ => panic!("unexpected replacement reply: {:?}", e.original()),
            }
        }
        // A committed parameter change and the old binding's historical flow do
        // not publish half a replacement pair. A reliable barrier observes idle
        // progress without supplying any packet on the new binding.
        tokio::time::sleep(Duration::from_millis(200)).await;
        let barrier = scene.clients[1].keepalive().await.unwrap();
        loop {
            let e = scene.clients[1].receive().await.unwrap();
            if e.kind() == "ack" {
                assert_eq!(e.original()["in_reply_to"], barrier);
                break;
            }
            assert_eq!(
                e.kind(),
                "connection.quality",
                "premature replacement: {:?}",
                e.original()
            );
            events.push(e);
        }
        assert!(
            events.iter().all(|e| e.kind() == "connection.quality"),
            "replacement emitted before new flow"
        );
        let fresh_flow_after = chrono::Utc::now();
        codecs::decoded_pcma(&mut scene.clients, scene.sh, changed, 76000).await;
        codecs::decoded_return(&mut scene.clients, scene.sh, changed, 77000).await;
        let mut closed = false;
        let mut opened = false;
        let mut closed_publication = None;
        let until = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            if closed && opened {
                break;
            }
            assert!(tokio::time::Instant::now() < until);
            let e = if events.is_empty() {
                scene.clients[1].receive().await.unwrap()
            } else {
                events.remove(0)
            };
            if closed && !opened {
                assert_eq!(
                    e.kind(),
                    "stream.opened",
                    "replacement batch was interleaved"
                );
            }
            if session_start(&e) {
                continue;
            }
            observe(algorithm, &e);
            assert_eq!(e.original()["cid"], scene.cid);
            assert_eq!(e.original()["sid"], "native-session");
            assert_eq!(e.original()["connid"], "codec-target");
            assert!(e.original().get("in_reply_to").is_none());
            let p = &e.original()["payload"];
            match e.kind() {
                "stream.closed" => {
                    assert!(!closed && !opened);
                    assert_eq!(p["strm_id"], "codec-monitor");
                    assert_eq!(p["reason"], "codec-renegotiated");
                    let retired =
                        chrono::DateTime::parse_from_rfc3339(p["closed_at"].as_str().unwrap())
                            .unwrap();
                    assert!(
                        retired < fresh_flow_after,
                        "original retirement time must survive deferred publication"
                    );
                    let publication =
                        chrono::DateTime::parse_from_rfc3339(e.original()["ts"].as_str().unwrap())
                            .unwrap();
                    assert!(publication >= fresh_flow_after);
                    closed_publication = Some(e.original()["ts"].clone());
                    closed = true;
                }
                "stream.opened" => {
                    assert!(closed && !opened);
                    assert_eq!(closed_publication.as_ref(), Some(&e.original()["ts"]));
                    assert_eq!(p["stream"]["strm_id"], "codec-monitor");
                    assert_eq!(p["stream"]["stream_local_id"], changed);
                    assert_eq!(p["stream"]["codec"]["name"], "pcma");
                    opened = true;
                }
                "connection.quality" => (),
                _ => panic!("unexpected replacement event: {:?}", e.original()),
            }
        }
        for client in scene.clients {
            client.shutdown().await.unwrap();
        }
        scene.f.server.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn production_owned_native_audio_selection_replay_preserves_pending_lifecycle_events() {
    owned_audio_signaling_case("EdDSA", false).await;
}
#[tokio::test]
async fn production_owned_audio_continuously_reads_signed_capability_bursts_while_preserving_duplex_audio_and_selector_replay(
) {
    for algorithm in ["EdDSA", "ES256"] {
        owned_audio_signaling_case(algorithm, true).await;
    }
}
async fn owned_audio_signaling_case(algorithm: &str, burst: bool) {
    use rvoip_uctp::substrate::{
        pack_rtp_datagram, unpack_rtp_datagram, RtpDatagram, RtpMediaPayload,
    };
    let f = Fixture::with_native_end_scope(algorithm, 12).await;
    ready_endpoint(&f).await;
    let source_config =
        party_with_capabilities(&f, &f.peer.identity_id, 106, true, vec!["audio".into()]);
    let mut target = CanonicalClient::open(f.client.clone()).await.unwrap();
    let cid = session(&f, &mut target).await;
    let mut source = CanonicalClient::open(source_config.clone()).await.unwrap();
    let answer = negotiate_named(
        &mut source,
        &cid,
        "owned-life-source-offer",
        "owned-life-source",
        "owned-life-voice",
    )
    .await;
    wait_native_operation(
        &f,
        &source_config.credential,
        &cid,
        "owned-life-source-offer",
    )
    .await;
    let source_handle = answer.original()["payload"]["substrate_setup"]["streams"][0]
        ["stream_local_id"]
        .as_u64()
        .unwrap() as u16;
    let scope = CommandScope {
        conversation_id: Some(cid.clone()),
        session_id: Some("native-session".into()),
        connection_id: Some("owned-life-target".into()),
        operation_id: Some("owned-life-target-offer".into()),
        expected_revision: Some(0),
    };
    let offer = json!({"substrate":"quic","capabilities":{"audio_codecs":[{"name":"pcmu","params":{"sample_rate":8000,"channels":1}}],"max_streams_per_connection":8,"identity_assurance_offered":"identified"},"streams_offered":[{"id":"owned-life-monitor","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}]});
    let mut owned = target
        .prepare_public_native_audio(scope, offer)
        .await
        .unwrap();
    let selection = vec![serde_json::from_value(json!({"strm_id":"owned-life-voice"})).unwrap()];
    assert_eq!(
        owned
            .subscribe_audio("owned-life-select", 1, selection.clone())
            .await
            .unwrap(),
        2
    );
    let callback = stream_control(
        &mut source,
        &cid,
        "owned-life-source",
        "owned-life-source-select",
        1,
        "stream.subscribe",
        json!({"subscriptions":[{"strm_id":"owned-life-monitor"}]}),
    )
    .await;
    let source_handle = callback.as_ref().map_or(source_handle, |e| {
        e.original()["payload"]["substrate_setup"]["streams"][0]["stream_local_id"]
            .as_u64()
            .unwrap() as u16
    });
    let wave = (0..160)
        .map(|n| if n % 16 < 8 { 0x80 } else { 0x00 })
        .collect::<Vec<u8>>();
    let source_clock = tokio::time::Instant::now();
    for pass in 0..if burst { 2 } else { 1 } {
        let mut heard_owned = false;
        let mut heard_source = false;
        let mut cadence = tokio::time::interval(Duration::from_millis(20));
        for seq in pass * 60..pass * 60 + 60u32 {
            cadence.tick().await;
            owned.send_pcmu(&wave).unwrap();
            source
                .send_native_rtp_datagram(
                    &pack_rtp_datagram(&RtpDatagram {
                        flags: 0,
                        stream_local_id: source_handle,
                        seq,
                        rtp: RtpMediaPayload {
                            payload: wave.clone().into(),
                            payload_type: 0,
                            sequence_number: seq as u16,
                            timestamp: ((source_clock.elapsed().as_millis() / 20) * 160) as u32,
                            ssrc: 0x6c69_6665,
                        },
                    })
                    .unwrap(),
                )
                .unwrap();
            // Atomic datagram probes may be cancelled without interrupting a
            // partially read reliable frame. Keep producing while the mixer warms.
            if let Ok(pcmu) =
                tokio::time::timeout(Duration::from_millis(5), owned.receive_pcmu()).await
            {
                heard_owned |= pcmu.unwrap().iter().any(|v| *v != 0xff && *v != 0x7f);
            }
            if let Ok(bytes) = tokio::time::timeout(
                Duration::from_millis(5),
                source.receive_native_rtp_datagram(),
            )
            .await
            {
                let bytes = bytes.unwrap();
                let packet = unpack_rtp_datagram(&bytes).unwrap();
                assert_eq!(packet.stream_local_id, source_handle);
                heard_source |= packet.rtp.payload.iter().any(|v| *v != 0xff && *v != 0x7f);
            }
        }
        assert!(
            heard_owned && heard_source,
            "both owned and raw native audio must be observed"
        );
        if burst && pass == 0 {
            let participant = capabilities::participant(&f, "owned-life-source");
            let mut descriptor = json!({"audio_codecs":[{"name":"pcmu","params":{}}],"future":{}});
            for i in 0..8 {
                descriptor["future"][format!("bounded_{i}")] = json!("x".repeat(1500));
            }
            for version in 0..220 {
                descriptor["future"]["burst_version"] = json!(version);
                let result = source
                    .advertise_native_capabilities(
                        CommandScope {
                            conversation_id: Some(cid.clone()),
                            session_id: Some("native-session".into()),
                            connection_id: Some("owned-life-source".into()),
                            operation_id: Some(format!("owned-capability-{version}")),
                            expected_revision: Some(2),
                        },
                        &participant,
                        descriptor.clone(),
                        "endpoint-change",
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    result.state,
                    rvoip_uctp::v2::payloads::OperationState::Completed,
                    "{result:?}"
                );
                tokio::time::sleep(Duration::from_millis(6)).await;
            }
            loop {
                let frame = owned.receive_signaling().await.unwrap();
                match frame.kind() {
                    "capability.advertise" => {
                        assert_eq!(frame.original()["connid"], "owned-life-source");
                        assert_eq!(frame.original()["payload"]["by_participant"], participant);
                        if frame.original()["payload"]["capabilities"]["future"]["burst_version"]
                            == 219
                        {
                            assert_eq!(frame.original()["payload"]["capabilities"], descriptor);
                            observe(algorithm, &frame);
                            break;
                        }
                    }
                    "session.started"
                    | "session.participant.joined"
                    | "session.participant.left" => {
                        assert!(session_start(&frame));
                    }
                    "connection.ready" | "stream.opened" | "stream.closed"
                    | "connection.quality" => {}
                    _ => panic!("unexpected owned signaling: {:?}", frame.original()),
                }
            }
        }
    }
    // Leave the owned peer's signaling unread through one full quality cadence.
    // Its next selector replay must preserve unrelated signed frames instead
    // of accepting them as a result, failing protocol validation or closing IO.
    tokio::time::sleep(Duration::from_millis(5200)).await;
    assert_eq!(
        owned
            .subscribe_audio("owned-life-select", 1, selection)
            .await
            .unwrap(),
        2
    );
    assert_eq!(owned.stream_revision(), 2);
    owned.send_pcmu(&wave).unwrap();
    owned.shutdown().await.unwrap();
    source.shutdown().await.unwrap();
    f.server.shutdown().await.unwrap();
}

pub(super) fn session_start(e: &Envelope) -> bool {
    if !matches!(
        e.kind(),
        "session.started"
            | "recording.vcon-ready"
            | "session.ended"
            | "session.participant.joined"
            | "session.participant.left"
    ) {
        return false;
    }
    for absent in ["connid", "operation_id", "in_reply_to"] {
        assert!(e.original().get(absent).is_none());
    }
    assert_eq!(e.original()["sid"], "native-session");
    let payload = &e.original()["payload"];
    match e.kind() {
        "session.started" => assert!(!payload["active_connections"].as_array().unwrap().is_empty()),
        "session.participant.joined" => {
            assert!(payload["participant"]["participant_id"].is_string());
            assert!(payload["via_connection"].is_string());
        }
        "session.participant.left" => {
            assert!(payload["participant_id"].is_string());
            assert!([
                "explicit-leave",
                "all-connections-ended",
                "kicked",
                "transferred-out"
            ]
            .contains(&payload["reason"].as_str().unwrap()));
        }
        "session.ended" => {
            assert!(payload["ended_at"].is_string());
            assert!(payload["reason_code"].is_u64());
        }
        "recording.vcon-ready" => {
            let handle = &payload["vcon_handle"];
            assert!(uuid::Uuid::parse_str(handle["uuid"].as_str().unwrap()).is_ok());
            assert!(handle["revision"].as_u64().is_some_and(|v| v > 0));
            assert!(payload.get("vcon").is_none());
        }
        _ => unreachable!(),
    }
    true
}
