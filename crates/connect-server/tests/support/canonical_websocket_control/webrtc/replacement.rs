//! A staged RTC target owns a distinct reservation while decoded old audio continues.
use super::*;
use connect_client::canonical::{CanonicalHttpClient, CommandScope};
use connect_protocol::transitions::{EffectState, TransitionCommand, TransitionIntent};

async fn target(f: &Fixture) -> (CanonicalClientConfig, String) {
    let admin = f.server.store.authenticate(&f.admin).unwrap().credential_id;
    let mut scope = Scope::collaboration(vec!["*".into()]);
    // Recovery requires the original movement scope, and receiving retained
    // media requires the target's own current subscription permission.
    scope
        .operations
        .extend(["connection.replace".into(), "stream.subscribe".into()]);
    let issued = f
        .server
        .store
        .issue(
            &f.client.identity_id,
            scope,
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let mut c = f.client.clone();
    c.credential = issued.token;
    c.device_id = "rtc-replacement-device".into();
    c.signing_key_id = "rtc-replacement-key".into();
    c.signing_private_key = f.temp.path().join("rtc-replacement-key.der");
    let public = signing(&c.signing_private_key, &c.signing_algorithm);
    let endpoint = provision_config(f, &c, 2).await;
    f.server
        .store
        .execute_provisioning(
            &admin,
            ProvisioningCommand::SigningKey {
                operation_id: "rtc-replacement-enrollment".into(),
                identity_id: c.identity_id.clone(),
                credential_id: issued.id,
                device_id: c.device_id.clone(),
                key_id: c.signing_key_id.clone(),
                algorithm: SigningAlgorithm::EdDSA,
                public_key: URL_SAFE_NO_PAD.encode(public),
                not_before: chrono::Utc::now().timestamp() - 1,
                expires_at: chrono::Utc::now().timestamp() + 600,
                expected_revision: None,
            },
        )
        .unwrap();
    (c, endpoint)
}

/// The browser SDK observes the original source scope through the signed target
/// connection. Independent Store assertions cannot substitute for this path.
pub(super) async fn signed_original_status(target: &mut Peer, cid: &str, operation: &str) -> Value {
    let mut request = envelope("operation.get", json!({"operation_id":operation}));
    request["cid"] = json!(cid);
    request["sid"] = json!("http3-media-session");
    request["connid"] = json!("rtc-connection-0");
    let id = target.send(request, true).await;
    let mut status = None;
    for _ in 0..128 {
        let frame = target.read().await;
        assert_eq!(frame.original()["in_reply_to"], id);
        assert_eq!(frame.original()["cid"], cid);
        assert_eq!(frame.original()["sid"], "http3-media-session");
        assert_eq!(frame.original()["connid"], "rtc-connection-0");
        match frame.kind() {
            "operation.updated" => {
                assert!(status.is_none());
                let payload = frame.original()["payload"].clone();
                assert_eq!(payload["operation_id"], operation);
                assert_eq!(payload["command_type"], "connection.replace");
                status = Some(payload);
            }
            "ack" => return status.expect("target must observe its original replacement"),
            _ => panic!("unexpected target status reply: {:?}", frame.original()),
        }
    }
    panic!("target original status reply exceeded bound");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn signed_webrtc_replacement_reserves_once_preserves_decoded_old_audio_and_cancels_target() {
    let algorithm = "EdDSA";
    let f = configured_end_scope(algorithm, 8, None, true).await;
    let host = SignalingHost::new(opus::config(), 3).unwrap();
    f.server
        .service
        .install_canonical_webrtc(host.clone())
        .unwrap();
    let other = canonical_webtransport_media::other(&f);
    provision_config(&f, &f.client, 0).await;
    provision_config(&f, &other, 1).await;
    let (cid, initiator, recipient) =
        canonical_webtransport_media::accepted_caps(&f, &other, "voice", json!({"audio":["opus"]}))
            .await;
    let mut first = Peer::open(&f).await;
    let mut second = Peer::open_config(&f, &other).await;
    let mut peers = Vec::new();
    let mut senders = Vec::new();
    for (i, ws, credential) in [(0, &mut first, &f.client), (1, &mut second, &other)] {
        let peer = RvoipPeerConnection::new(&opus::config(), PeerRole::Offerer)
            .await
            .unwrap();
        let sdp = peer.create_offer_and_gather().await.unwrap();
        assert!(sdp.contains("opus/48000/2"));
        let mut request = scoped(
            &cid,
            i,
            "connection.offer",
            &format!("rtc-offer-{i}"),
            0,
            json!({
                "substrate":"websocket+webrtc", "capabilities":{"audio_codecs":[{"name":"opus","params":{"sample_rate":48000,"channels":1}}],"max_streams_per_connection":1,"identity_assurance_offered":"identified"},
                "streams_offered":[{"id":format!("rtc-audio-{i}"),"kind":"audio","direction":"sendrecv","codec_preferences":["opus"]}],
                "substrate_setup":{"kind":"websocket+webrtc","sdp":sdp}
            }),
        );
        request["id"] = json!(uuid::Uuid::new_v4().to_string());
        let id = ws.send(request, true).await;
        let answer = ws.read().await;
        assert_eq!(
            answer.kind(),
            "connection.answer",
            "{:?}",
            answer.original()
        );
        assert_eq!(answer.original()["in_reply_to"], id);
        let payload = &answer.original()["payload"];
        assert_eq!(
            payload["streams_answered"][0]["codec"]["params"]["channels"],
            1
        );
        assert_eq!(
            payload["streams_answered"][0]["codec"]["params"]["sample_rate"],
            48000
        );
        assert_eq!(
            payload["streams_answered"][0]["codec"]["params"]["payload_type"],
            111
        );
        let answer_sdp = payload["substrate_setup"]["sdp"].as_str().unwrap();
        assert!(answer_sdp.contains("opus/48000/2"));
        peer.set_remote_answer(answer_sdp).await.unwrap();
        peer.wait_connected(Duration::from_secs(5)).await.unwrap();
        senders.push(opus::producer(&peer, if i == 0 { 440.0 } else { 880.0 }));
        completed(&f, credential, &format!("rtc-offer-{i}")).await;
        peers.push(peer);
    }
    for peer in &peers {
        assert!(
            tokio::time::timeout(Duration::from_millis(300), peer.recv_remote_track())
                .await
                .is_err()
        );
    }
    for (i, ws, credential, participant) in [
        (0, &mut first, &f.client, &recipient),
        (1, &mut second, &other, &initiator),
    ] {
        command(
            ws,
            scoped(
                &cid,
                i,
                "stream.subscribe",
                &format!("opus-select-{i}"),
                1,
                json!({"subscriptions":[{"from_participant":participant,"kinds":["audio"]}]}),
            ),
        )
        .await;
        completed(&f, credential, &format!("opus-select-{i}")).await;
    }
    let (stream_a, mut a) = opus::receiving(&peers[0], 0).await;
    let (stream_b, mut b) = opus::receiving(&peers[1], 1).await;
    opus::tone(&mut a, 880.0, false).await;
    opus::tone(&mut b, 440.0, false).await;

    let (target_config, endpoint) = target(&f).await;
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope
        .operations
        .extend(["connection.replace".into(), "operation.cancel".into()]);
    let key = f
        .server
        .store
        .issue(
            &f.client.identity_id,
            scope,
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let mut controller_config = canonical_http_config(&f);
    controller_config.credential = key.token.clone();
    let mut controller = CanonicalHttpClient::open(controller_config).await.unwrap();
    let (revision, _) = session_role_snapshot(&mut first, &cid, &initiator).await;
    let operation = "rtc-staged-replacement";
    controller
        .transition(TransitionCommand {
            operation_id: operation.into(),
            conversation_id: cid.clone(),
            session_id: "http3-media-session".into(),
            connection_id: Some("rtc-connection-0".into()),
            expected_revision: revision,
            intent: TransitionIntent::Replace {
                endpoint_id: Some(endpoint),
                candidate_connid: None,
                retained_connid: "rtc-connection-1".into(),
                video: None,
            },
            payload_extensions: Default::default(),
        })
        .await
        .unwrap();
    let prepared = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let value = f
                .server
                .store
                .execute_operation_action(
                    &key.id,
                    OperationAction::Get {
                        operation_id: operation.into(),
                    },
                )
                .unwrap();
            if value.effects.iter().any(|e| {
                e.provider == "native-replacement-prepare" && e.state == EffectState::Confirmed
            }) {
                break value;
            }
            assert!(
                matches!(
                    value.state,
                    OperationState::Accepted | OperationState::Preparing
                ),
                "{value:?}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("original RTC replacement preparation");
    // A second actual peer exists, but no SDP answer or media activation has
    // been authorized. The old route must continue carrying real decoded PCM.
    let candidate = format!("replacement_{}", prepared.record_id);
    let replacement = RvoipPeerConnection::new(&opus::config(), PeerRole::Offerer)
        .await
        .unwrap();
    let sdp = replacement.create_offer_and_gather().await.unwrap();
    let mut target_peer = Peer::open_config(&f, &target_config).await;
    let mut offer = scoped(
        &cid,
        2,
        "connection.offer",
        operation,
        0,
        json!({
            "substrate":"websocket+webrtc", "capabilities":{"audio_codecs":[{"name":"opus","params":{"sample_rate":48000,"channels":1}}],"max_streams_per_connection":1,"identity_assurance_offered":"identified"},
            "streams_offered":[{"id":"replacement-audio","kind":"audio","direction":"sendrecv","codec_preferences":["opus"]}],
            "substrate_setup":{"kind":"websocket+webrtc","sdp":sdp},
            "replacement_preparation":{"profile":"connect-replacement-preparation-v1","operation_record_id":prepared.record_id}
        }),
    );
    offer["connid"] = json!(candidate);
    command(&mut target_peer, offer.clone()).await;
    let allocated = record(&f, &target_config, operation);
    let observed = signed_original_status(&mut target_peer, &cid, operation).await;
    assert_eq!(observed["record_id"], allocated.record_id);
    assert_eq!(observed["state"], "preparing");
    assert!(observed.get("result").is_none());
    assert_eq!(allocated.state, OperationState::Preparing);
    assert!(allocated.result.is_none());
    assert_eq!(
        allocated
            .effects
            .iter()
            .filter(|e| e.provider == "native-replacement-allocate"
                && e.state == EffectState::Confirmed
                && e.attempt == 1)
            .count(),
        1
    );
    assert_eq!(host.adapter().route_count(), 3);
    assert_eq!(host.adapter().total_allocations(), 3);
    assert_eq!(host.available_slots(), 0);
    let session = f
        .server
        .service
        .canonical_session_metadata_for_test("http3-media-session")
        .unwrap();
    assert_eq!(session.read().unwrap().connections.len(), 2);
    while a.try_recv().is_ok() {}
    while b.try_recv().is_ok() {}
    opus::tone(&mut a, 880.0, false).await;
    opus::tone(&mut b, 440.0, false).await;
    // Exact retry reads the same original receipt and cannot reserve again.
    command(&mut target_peer, offer).await;
    assert_eq!(record(&f, &target_config, operation), allocated);
    assert_eq!(
        signed_original_status(&mut target_peer, &cid, operation).await,
        observed
    );
    assert_eq!(host.adapter().total_allocations(), 3);
    controller
        .request_value(
            "operation.cancel",
            json!({"operation_id":operation,"reason":"fixture target preparation cancellation"}),
            CommandScope {
                conversation_id: Some(cid.clone()),
                operation_id: Some("rtc-replacement-cancel".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while host.adapter().route_count() != 2 || host.available_slots() != 1 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("only original replacement candidate must retire");
    while a.try_recv().is_ok() {}
    while b.try_recv().is_ok() {}
    opus::tone(&mut a, 880.0, false).await;
    opus::tone(&mut b, 440.0, false).await;
    assert_eq!(session.read().unwrap().connections.len(), 2);
    replacement.close().await.unwrap();
    target_peer.socket.close(None).await.unwrap();
    stream_a.close().await.unwrap();
    stream_b.close().await.unwrap();
    for (stop, sender) in senders {
        stop.cancel();
        assert!(sender.await.unwrap() > 20);
    }
    for peer in peers {
        peer.close().await.unwrap();
    }
    first.socket.close(None).await.unwrap();
    second.socket.close(None).await.unwrap();
    drop(first);
    drop(second);
    drop(target_peer);
    f.server.shutdown().await.unwrap();
    assert_eq!(host.adapter().route_count(), 0);
    assert_eq!(host.available_slots(), 3);
}
