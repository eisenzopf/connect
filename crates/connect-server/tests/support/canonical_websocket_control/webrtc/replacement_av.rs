//! Real signed same-root RTC replacement, decoded private readiness and retirement.
//! Source media is local synthetic Opus/VP8; decoder observations use actual PCM/pixels.
use super::*;
use connect_client::canonical::CanonicalHttpClient;
use connect_protocol::{
    replacement_video::{ReplacementVideo, ReplacementVideoStream},
    transitions::{EffectState, TransitionCommand, TransitionIntent},
};

async fn target_av(f: &Fixture) -> (CanonicalClientConfig, String) {
    let admin = f.server.store.authenticate(&f.admin).unwrap().credential_id;
    let mut scope = Scope::collaboration(vec!["*".into()]);
    // A separate signed device retains the same ordinary movement/read scope
    // as the matrix actors, plus its own permission to receive selected media.
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
    c.device_id = "rtc-av-replacement-device".into();
    c.signing_key_id = "rtc-av-replacement-key".into();
    c.signing_private_key = f.temp.path().join("rtc-av-replacement-key.der");
    let public = signing(&c.signing_private_key, &c.signing_algorithm);
    let endpoint = provision_media_config(f, &c, 2, &["audio", "video"]).await;
    f.server
        .store
        .execute_provisioning(
            &admin,
            ProvisioningCommand::SigningKey {
                operation_id: "rtc-av-replacement-enrollment".into(),
                identity_id: c.identity_id.clone(),
                credential_id: issued.id,
                device_id: c.device_id.clone(),
                key_id: c.signing_key_id.clone(),
                algorithm: if c.signing_algorithm == "ES256" {
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
    (c, endpoint)
}
fn renew(mut offer: Value) -> Value {
    offer["id"] = json!(uuid::Uuid::new_v4().to_string());
    offer
}
fn connection_state(f: &Fixture, id: &str) -> (String, u64) {
    rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap()
    .query_row(
        "SELECT state,generation FROM connections WHERE id=?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .unwrap()
}
fn runtime_id(f: &Fixture, id: &str) -> rvoip_core::ids::ConnectionId {
    let raw: String = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap()
    .query_row(
        "SELECT runtime_connection_id FROM connection_offers WHERE connection_id=?1",
        [id],
        |r| r.get(0),
    )
    .unwrap();
    rvoip_core::ids::ConnectionId::from_string(raw)
}
async fn decoded_private_audio(rx: &mut mpsc::Receiver<MediaFrame>) -> u64 {
    let mut decoder = ::opus::Decoder::new(48000, ::opus::Channels::Mono).unwrap();
    let mut pcm = [0i16; 5760];
    let mut samples = 0;
    for _ in 0..3 {
        let frame = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(frame.payload_type, Some(111));
        // Decode original received packets, with FEC/PLC disabled. Silence is
        // valid audio; socket/RTP counters alone never supply readiness.
        assert_eq!(
            ::opus::packet::get_nb_samples(&frame.payload, 48000).unwrap(),
            960
        );
        let count = decoder.decode(&frame.payload, &mut pcm, false).unwrap();
        assert_eq!(count, 960);
        samples += count as u64;
    }
    samples
}
fn drain(received: &mut Received) {
    while received.audio.try_recv().is_ok() {}
    while received.video.try_recv().is_ok() {}
}
async fn original_av(
    a: &mut Received,
    b: &mut Received,
    red: &[u8],
    blue: &[u8],
    dir: &Path,
    label: &str,
) {
    drain(a);
    drain(b);
    opus::tone(&mut a.audio, 880.0, false).await;
    opus::tone(&mut b.audio, 440.0, false).await;
    picture(
        &mut a.video,
        blue,
        &format!("{label}-original-receives-blue"),
        dir,
        false,
    )
    .await;
    picture(
        &mut b.video,
        red,
        &format!("{label}-retained-receives-red"),
        dir,
        true,
    )
    .await;
}
async fn full_replacement(algorithm: &str) {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .try_init();
    let fixtures = PathBuf::from(
        std::env::var("CONNECT_GATE6_AV_FIXTURE_DIR").expect("explicit synthetic AV fixtures"),
    );
    let red = std::fs::read(fixtures.join("red.vp8")).unwrap();
    let blue = std::fs::read(fixtures.join("blue.vp8")).unwrap();
    assert_eq!((red.len(), blue.len()), (64, 64));
    assert_eq!(red[0] & 1, 0);
    assert_eq!(blue[0] & 1, 0);
    let directory = fixtures.join(format!("replacement-{algorithm}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&directory).unwrap();
    tracing::debug!(
        target: "connect_server::gate6_rtc_receiver_trace",
        algorithm,
        fixture_directory = %directory.display(),
        "Original synthetic RTC AV fixture created"
    );
    let f = configured_end_scope(algorithm, 8, None, true).await;
    let host = SignalingHost::new(config(), 3).unwrap();
    f.server
        .service
        .install_canonical_webrtc(host.clone())
        .unwrap();
    let other = canonical_webtransport_media::other(&f);
    provision_media_config(&f, &f.client, 0, &["audio", "video"]).await;
    provision_media_config(&f, &other, 1, &["audio", "video"]).await;
    let (cid, initiator, recipient) = canonical_webtransport_media::accepted_caps(
        &f,
        &other,
        "voice+video",
        json!({"audio":["opus"],"video":["vp8"]}),
    )
    .await;
    let mut first = Peer::open(&f).await;
    let mut second = Peer::open_config(&f, &other).await;
    let mut peers = vec![];
    let mut senders = vec![];
    for (i, ws, credential) in [(0, &mut first, &f.client), (1, &mut second, &other)] {
        let peer = RvoipPeerConnection::new(&config(), PeerRole::Offerer)
            .await
            .unwrap();
        peer.add_local_audio_track().await.unwrap();
        peer.add_local_video_track().await.unwrap();
        let sdp = peer.create_offer_and_gather().await.unwrap();
        let offer = request(&cid, i, &sdp);
        let id = ws.send(offer, true).await;
        let answer = ws.read().await;
        assert_eq!(
            answer.kind(),
            "connection.answer",
            "{:?}",
            answer.original()
        );
        assert_eq!(answer.original()["in_reply_to"], id);
        peer.set_remote_answer(
            answer.original()["payload"]["substrate_setup"]["sdp"]
                .as_str()
                .unwrap(),
        )
        .await
        .unwrap();
        peer.wait_connected(Duration::from_secs(5)).await.unwrap();
        senders.push(opus::producer(&peer, if i == 0 { 440.0 } else { 880.0 }));
        senders.push(video_producer(
            &peer,
            if i == 0 { red.clone() } else { blue.clone() },
        ));
        completed(&f, credential, &format!("rtc-offer-{i}")).await;
        peers.push(peer);
    }
    for (i, ws, credential, participant) in [
        (0, &mut first, &f.client, &recipient),
        (1, &mut second, &other, &initiator),
    ] {
        command(ws,scoped(&cid,i,"stream.subscribe",&format!("rtc-av-select-{i}"),1,json!({"subscriptions":[{"from_participant":participant,"kinds":["audio"]},{"strm_id":format!("rtc-video-{}",1-i)}]}))).await;
        completed(&f, credential, &format!("rtc-av-select-{i}")).await;
    }
    let mut a = receive(&peers[0], 0).await;
    let mut b = receive(&peers[1], 1).await;
    original_av(&mut a, &mut b, &red, &blue, &directory, "before").await;
    let old_runtime = runtime_id(&f, "rtc-connection-0");
    let retained_runtime = runtime_id(&f, "rtc-connection-1");
    let (target_config, endpoint) = target_av(&f).await;
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope.operations.push("connection.replace".into());
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
    let operation = "rtc-full-av-replacement";
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
                video: Some(ReplacementVideo {
                    publications: vec![ReplacementVideoStream {
                        previous: "rtc-video-0".into(),
                        replacement: "replacement-video-send".into(),
                    }],
                    receivers: vec![ReplacementVideoStream {
                        previous: "rtc-video-0".into(),
                        replacement: "replacement-video-receive".into(),
                    }],
                }),
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
    .expect("original replacement preparation");
    let candidate = format!("replacement_{}", prepared.record_id);
    let replacement = RvoipPeerConnection::new(&config(), PeerRole::Offerer)
        .await
        .unwrap();
    replacement.add_local_audio_track().await.unwrap();
    replacement.add_local_video_track().await.unwrap();
    let sdp = replacement.create_offer_and_gather().await.unwrap();
    let mut target_peer = Peer::open_config(&f, &target_config).await;
    let mut allocation = scoped(
        &cid,
        2,
        "connection.offer",
        operation,
        0,
        json!({"substrate":"websocket+webrtc","capabilities":{"audio_codecs":[{"name":"opus","params":{"sample_rate":48000,"channels":1}}],"video_codecs":[{"name":"vp8","params":{"sample_rate":90000,"channels":0}}],"max_streams_per_connection":3,"identity_assurance_offered":"identified"},"streams_offered":[{"id":"replacement-audio","kind":"audio","direction":"sendrecv","codec_preferences":["opus"]},{"id":"replacement-video-send","kind":"video","direction":"sendonly","codec_preferences":["vp8"]},{"id":"replacement-video-receive","kind":"video","direction":"recvonly","codec_preferences":["vp8"]}],"substrate_setup":{"kind":"websocket+webrtc","sdp":sdp},"replacement_preparation":{"profile":"connect-replacement-preparation-v1","operation_record_id":prepared.record_id}}),
    );
    allocation["connid"] = json!(candidate);
    command(&mut target_peer, allocation.clone()).await;
    let status =
        super::super::replacement::signed_original_status(&mut target_peer, &cid, operation).await;
    assert_eq!(status["record_id"], prepared.record_id);
    assert_eq!(status["state"], "preparing");
    assert!(status.get("result").is_none());
    assert_eq!(host.adapter().route_count(), 3);
    assert_eq!(host.adapter().total_allocations(), 3);
    assert_eq!(host.available_slots(), 0);
    assert!(host.adapter().is_connection_live(&old_runtime));
    assert!(host.adapter().is_connection_live(&retained_runtime));
    let mut media = renew(allocation);
    media["payload"]["replacement_preparation"]["profile"] = json!("connect-replacement-media-v1");
    let id = target_peer.send(media.clone(), true).await;
    let answer = target_peer.read().await;
    assert_eq!(
        answer.kind(),
        "connection.answer",
        "{:?}",
        answer.original()
    );
    assert_eq!(answer.original()["in_reply_to"], id);
    let payload = &answer.original()["payload"];
    let challenge = payload["replacement_preparation"]["readiness"].clone();
    assert_eq!(challenge["source_generation"], 1);
    assert_eq!(challenge["target_generation"], 2);
    assert_eq!(challenge["stream_revision"], 2);
    assert_eq!(payload["streams_answered"][1]["direction"], "recvonly");
    assert_eq!(payload["streams_answered"][2]["direction"], "sendonly");
    replacement
        .set_remote_answer(payload["substrate_setup"]["sdp"].as_str().unwrap())
        .await
        .unwrap();
    replacement
        .wait_connected(Duration::from_secs(5))
        .await
        .unwrap();
    senders.push(opus::producer(&replacement, 660.0));
    senders.push(video_producer(&replacement, red.clone()));
    let mut c = receive(&replacement, 2).await;
    let decoded_samples = decoded_private_audio(&mut c.audio).await;
    picture(
        &mut c.video,
        &blue,
        "private-target-first-blue-keyframe",
        &directory,
        false,
    )
    .await;
    let mut observation = challenge.clone();
    observation["audio"] =
        json!({"stream_id":"replacement-audio","decoded_samples":decoded_samples});
    // The preceding independent VP8 decode consumed exactly one actual source
    // keyframe on this new receiver. No RTP-only readiness is substituted.
    observation["video"] = json!({"stream_id":"replacement-video-receive","decoded_frames":1,"decoded_keyframes":1,"decoder_startup":"fresh-peer-first-decoded-frame"});
    let mut ready = renew(media);
    ready["payload"]["replacement_preparation"]["profile"] = json!("connect-replacement-ready-v1");
    ready["payload"]["replacement_preparation"]["readiness"] = observation;
    for invalid in ["nonce", "generation", "audio", "video"] {
        let mut bad = renew(ready.clone());
        let value = &mut bad["payload"]["replacement_preparation"]["readiness"];
        match invalid {
            "nonce" => value["nonce"] = json!("another-target-nonce"),
            "generation" => value["target_generation"] = json!(3),
            "audio" => value["audio"]["decoded_samples"] = json!(0),
            "video" => value["video"]["decoded_keyframes"] = json!(0),
            _ => unreachable!(),
        }
        refused(&mut target_peer, bad, "forbidden", 403).await;
        let pending = record(&f, &target_config, operation);
        assert_eq!(pending.state, OperationState::Preparing);
        assert!(pending.result.is_none());
        assert_eq!(connection_state(&f, "rtc-connection-0").1, 1);
        assert!(host.adapter().is_connection_live(&old_runtime));
        assert_eq!(host.adapter().route_count(), 3);
    }
    original_av(&mut a, &mut b, &red, &blue, &directory, "waiting-for-ready").await;
    command(&mut target_peer, renew(ready.clone())).await;
    let done = completed(&f, &target_config, operation).await;
    assert_eq!(done.record_id, prepared.record_id);
    assert_eq!(done.result.as_ref().unwrap()["generation"], 2);
    let activation: connect_protocol::replacement_activation::NativeActivationBoundary =
        serde_json::from_value(done.result.as_ref().unwrap()["native_activation"].clone()).unwrap();
    activation.validate().unwrap();
    assert!(
        matches!(activation.audience,connect_protocol::replacement_activation::NativeActivationAudience::Unsupported{ref reason} if reason=="non-native-adapter"),
        "actual saved websocket+webrtc offer has no native packet cut"
    );

    assert_eq!(done.result.as_ref().unwrap()["connection_id"], candidate);
    assert!(
        done.effects.iter().all(|e| e.attempt <= 1),
        "no transport or movement attempt may replay"
    );
    for provider in [
        "native-replacement-prepare",
        "native-replacement-allocate",
        "native-replacement-start",
        "native-replacement-attach",
        "native-replacement-activate",
        "native-replacement-readiness",
        "native-replacement-commit",
    ] {
        assert_eq!(
            done.effects
                .iter()
                .filter(|e| e.provider == provider
                    && e.state == EffectState::Confirmed
                    && e.attempt == 1)
                .count(),
            1,
            "missing exact original phase {provider}"
        );
    }

    tokio::time::timeout(Duration::from_secs(5), async {
        while host.adapter().route_count() != 2 || host.available_slots() != 1 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("old actual route retires after ready");
    assert!(!host.adapter().is_connection_live(&old_runtime));
    assert!(host.adapter().is_connection_live(&retained_runtime));
    assert!(host
        .adapter()
        .is_connection_live(&runtime_id(&f, &candidate)));
    assert_eq!(
        connection_state(&f, "rtc-connection-0"),
        ("retired".into(), 2)
    );
    assert_eq!(connection_state(&f, &candidate).1, 2);
    // Delivery uncertainty must recover the SAME original result. Repeating
    // the exact signed observation may acknowledge it, but creates no second
    // allocation, readiness capture or movement. Full results are read through
    // operation.get because the ready acknowledgment is intentionally scoped.
    command(&mut target_peer, renew(ready.clone())).await;
    let recovered = record(&f, &target_config, operation);
    let observed =
        super::super::replacement::signed_original_status(&mut target_peer, &cid, operation).await;
    assert_eq!(observed["record_id"], done.record_id);
    assert_eq!(observed["state"], "completed");
    assert_eq!(observed.get("result"), done.result.as_ref());
    assert_eq!(recovered.record_id, done.record_id);
    assert_eq!(recovered.state, OperationState::Completed);
    assert_eq!(recovered.result, done.result);
    assert_eq!(recovered.effects, done.effects);
    assert_eq!(host.adapter().total_allocations(), 3);
    assert_eq!(host.adapter().route_count(), 2);
    let mut changed_observation = renew(ready.clone());
    changed_observation["payload"]["replacement_preparation"]["readiness"]["audio"]
        ["decoded_samples"] = json!(decoded_samples + 960);
    refused(&mut target_peer, changed_observation, "conflict", 409).await;
    let mut changed_nonce = renew(ready.clone());
    changed_nonce["payload"]["replacement_preparation"]["readiness"]["nonce"] =
        json!("another-target-nonce");
    refused(&mut target_peer, changed_nonce, "forbidden", 403).await;
    command(&mut target_peer, renew(ready)).await;
    let unchanged = record(&f, &target_config, operation);
    assert_eq!(
        super::super::replacement::signed_original_status(&mut target_peer, &cid, operation).await,
        observed
    );
    assert_eq!(unchanged.result, done.result);
    assert_eq!(unchanged.effects, done.effects);
    assert_eq!(host.adapter().route_count(), 2);
    assert_eq!(host.adapter().total_allocations(), 3);
    drain(&mut b);
    drain(&mut c);
    opus::tone(&mut b.audio, 660.0, false).await;
    opus::tone(&mut c.audio, 880.0, false).await;
    picture(
        &mut b.video,
        &red,
        "retained-receives-new-target-red",
        &directory,
        true,
    )
    .await;
    picture(
        &mut c.video,
        &blue,
        "new-target-receives-retained-blue",
        &directory,
        false,
    )
    .await;
    for received in [a, b, c] {
        for stream in received.streams {
            stream.close().await.unwrap();
        }
    }
    for (stop, sender) in senders {
        stop.cancel();
        assert!(sender.await.unwrap() > 0);
    }
    for peer in peers {
        peer.close().await.unwrap();
    }
    replacement.close().await.unwrap();
    first.socket.close(None).await.unwrap();
    second.socket.close(None).await.unwrap();
    target_peer.socket.close(None).await.unwrap();
    drop(first);
    drop(second);
    drop(target_peer);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let target = connection_state(&f, &candidate);
            if target.0 == "retired"
                && host.adapter().route_count() == 0
                && host.available_slots() == 3
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("original RTC workers and durable replacement lifetime join");
    let database = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert!(database.query_row("SELECT retired FROM native_connection_lifetimes WHERE operation_record_id=?1 AND connection_id=?2 AND generation=2",rusqlite::params![done.record_id,candidate],|r|r.get::<_,bool>(0)).unwrap());
    drop(database);
    f.server.shutdown().await.unwrap();
    assert_eq!(host.adapter().route_count(), 0);
    assert_eq!(host.available_slots(), 3);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "explicit synthetic VP8 fixtures and independent decoder; CONNECT_GATE6_AV_FIXTURE_DIR and CONNECT_GATE6_FFMPEG"]
async fn signed_webrtc_av_replacement_waits_for_decoded_target_before_retiring_original_eddsa() {
    full_replacement("EdDSA").await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "explicit synthetic VP8 fixtures and independent decoder; CONNECT_GATE6_AV_FIXTURE_DIR and CONNECT_GATE6_FFMPEG"]
async fn signed_webrtc_av_replacement_waits_for_decoded_target_before_retiring_original_es256() {
    full_replacement("ES256").await;
}
