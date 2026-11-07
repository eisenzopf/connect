//! Real signed same-root RTC replacement, decoded private readiness and retirement.
//! Source media is local synthetic Opus/VP8; decoder observations use actual PCM/pixels.
use super::*;
#[path = "replacement_av/private_marker.rs"]
mod private_marker;
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
    private_marker::picture_once(
        &mut a.video,
        blue,
        &format!("{label}-original-receives-blue"),
        dir,
        false,
    )
    .await;
    private_marker::picture_once(
        &mut b.video,
        red,
        &format!("{label}-retained-receives-red"),
        dir,
        true,
    )
    .await;
}
fn public_problem(value: &Value) -> connect_protocol::canonical_http::Problem {
    assert_eq!(
        value.as_object().unwrap().len(),
        3,
        "closed canonical problem shape"
    );
    serde_json::from_value(value.clone()).unwrap()
}
fn ready_response_is_uncertain(problem: &connect_protocol::canonical_http::Problem) -> bool {
    same_problem(
        problem,
        &connect_protocol::canonical_http::Problem::from_error(&connect_protocol::Error::new(
            "authority-busy",
            "fixture class only",
        )),
    )
}

async fn submit_original_ready(
    target: &mut Peer,
    ready: Value,
    cid: &str,
    operation: &str,
    original_record: &str,
) -> Option<connect_protocol::canonical_http::Problem> {
    // One signed Ready submission. A response may be unavailable after its
    // observation was captured; recover only that same original operation.
    assert_eq!(ready["operation_id"], operation);
    let request = target.send(ready, true).await;
    for _ in 0..128 {
        let event = target.read().await;
        assert_eq!(event.original()["in_reply_to"], request);
        match event.kind() {
            "error" => {
                let problem = public_problem(&event.original()["payload"]);
                assert!(
                    ready_response_is_uncertain(&problem),
                    "{:?}",
                    event.original()
                );
                let recovered =
                    super::super::replacement::signed_original_status(target, cid, operation).await;
                assert_eq!(recovered["record_id"], original_record);
                assert_eq!(recovered["operation_id"], operation);
                assert_eq!(recovered["command_type"], "connection.replace");
                assert!(matches!(
                    recovered["state"].as_str(),
                    Some("preparing" | "completed")
                ));
                return Some(problem);
            }
            "operation.updated" => {
                assert_eq!(event.original()["payload"]["record_id"], original_record);
                assert_eq!(event.original()["payload"]["operation_id"], operation);
            }
            "ack" => return None,
            _ => panic!("unexpected original Ready response: {:?}", event.original()),
        }
    }
    panic!("original Ready response exceeded its bounded event count");
}

fn same_problem(
    actual: &connect_protocol::canonical_http::Problem,
    expected: &connect_protocol::canonical_http::Problem,
) -> bool {
    actual.code == expected.code
        && actual.category == expected.category
        && actual.reason == expected.reason
}
fn is_public_refusal_shape(problem: &connect_protocol::canonical_http::Problem) -> bool {
    // Exact public shapes only. Public400/state/operation_rejected also covers
    // unknown internal errors; no internal class can be recovered from it.
    // Qualification separately requires the original domain-boundary witness.
    [
        "not-ready",
        "unauthorized",
        "forbidden",
        "not-found",
        "conflict",
    ]
    .iter()
    .any(|code| {
        same_problem(
            problem,
            &connect_protocol::canonical_http::Problem::from_error(&connect_protocol::Error::new(
                *code,
                "fixture class only",
            )),
        )
    })
}
fn is_refusal_progress(state: Option<&str>) -> bool {
    matches!(
        state,
        Some("accepted" | "preparing" | "failed" | "cancelled")
    )
}
#[test]
fn readiness_refusal_problems_and_progress_require_closed_exact_semantics() {
    use connect_protocol::{canonical_http::Problem, Error};
    for code in [
        "not-ready",
        "unauthorized",
        "forbidden",
        "not-found",
        "conflict",
    ] {
        assert!(is_public_refusal_shape(&Problem::from_error(&Error::new(
            code, "fixture"
        ))));
    }
    assert!(ready_response_is_uncertain(&Problem::from_error(
        &Error::new("authority-busy", "fixture")
    )));
    for code in [
        "invalid-request",
        "unauthorized",
        "forbidden",
        "not-found",
        "conflict",
        "not-ready",
    ] {
        assert!(!ready_response_is_uncertain(&Problem::from_error(
            &Error::new(code, "fixture")
        )));
    }
    for code in [
        "invalid-request",
        "storage-unavailable",
        "authority-busy",
        "unsupported-feature",
    ] {
        assert!(!is_public_refusal_shape(&Problem::from_error(&Error::new(
            code, "fixture"
        ))));
    }
    for (code, category, reason) in [
        (400, "validation", "invalid_request"),
        (400, "auth", "operation_rejected"),
        (403, "auth", "operation_rejected"),
        (503, "state", "operation_rejected"),
    ] {
        assert!(!is_public_refusal_shape(&Problem {
            code,
            category: category.into(),
            reason: reason.into()
        }));
    }
    let unknown = Problem::from_error(&Error::new("unrecognized-private-error", "fixture"));
    let not_ready = Problem::from_error(&Error::new("not-ready", "fixture"));
    assert!(
        same_problem(&unknown, &not_ready),
        "publicstate400 intentionally hides internal class"
    );
    assert!(is_public_refusal_shape(&unknown));
    assert!(
        !qualified_negative(&unknown, None),
        "genericpublic400ALONE is not negative qualification"
    );
    assert!(
        !qualified_negative(&not_ready, None),
        "even knownpublicshape needs original boundary evidence"
    );
    for state in [
        Some("accepted"),
        Some("preparing"),
        Some("failed"),
        Some("cancelled"),
    ] {
        assert!(is_refusal_progress(state));
    }
    for state in [Some("completed"), Some("uncertain"), None] {
        assert!(!is_refusal_progress(state));
    }
}
// Fixture-only corroboration marker; never media authority or a public API.
// Its only production test construction is AFTER all actual original terminal,
// boundary, no-effect and physical retirement assertions in the macro below.
struct OriginalNegativeTerminal;
fn qualified_negative(
    problem: &connect_protocol::canonical_http::Problem,
    corroboration: Option<&OriginalNegativeTerminal>,
) -> bool {
    is_public_refusal_shape(problem) && corroboration.is_some()
}
async fn ready_shape_refused(ws: &mut Peer, value: Value, internal_code: &str) {
    let expected = connect_protocol::canonical_http::Problem::from_error(
        &connect_protocol::Error::new(internal_code, "fixture class only"),
    );
    let id = ws.send(value, true).await;
    tokio::time::timeout(Duration::from_secs(5), async {
        for _ in 0..128 {
            let event = ws.read().await;
            assert_eq!(event.original()["in_reply_to"], id);
            match event.kind() {
                "error" => {
                    assert!(
                        same_problem(&public_problem(&event.original()["payload"]), &expected),
                        "exact readiness refusal problem differs"
                    );
                    return;
                }
                "operation.updated" => assert!(
                    is_refusal_progress(event.original()["payload"]["state"].as_str()),
                    "rejected readiness unexpectedly completed"
                ),
                _ => panic!("unexpected rejected readiness event kind={}", event.kind()),
            }
        }
        panic!("rejected readiness did not produce exact terminal problem")
    })
    .await
    .expect("exact rejected readiness reply cutoff");
}
async fn full_replacement(algorithm: &str) {
    full_replacement_with_private_delay(algorithm, false).await;
}
async fn full_replacement_with_private_delay(algorithm: &str, delay_after_admit: bool) {
    full_replacement_case(algorithm, delay_after_admit, None).await;
}
async fn full_replacement_case(algorithm: &str, delay_after_admit: bool, negative: Option<&str>) {
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
    let green = if delay_after_admit {
        let encoded = std::fs::read(fixtures.join("green.vp8")).unwrap();
        assert!(!encoded.is_empty() && encoded.len() < 1200 && encoded[0] & 1 == 0);
        assert_ne!(encoded, red);
        assert_ne!(encoded, blue);
        Some(encoded)
    } else {
        None
    };
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
    let mut retained_marker = None;
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
        if i == 1 && delay_after_admit {
            let (original, marker) = private_marker::producers(&peer, 880.0, blue.clone());
            senders.extend(original);
            retained_marker = Some(marker);
        } else {
            senders.push(opus::producer(&peer, if i == 0 { 440.0 } else { 880.0 }));
            senders.push(video_producer(
                &peer,
                if i == 0 { red.clone() } else { blue.clone() },
            ));
        }
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
    let original_reservation = || {
        rusqlite::Connection::open_with_flags(&f.config.database,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap().query_row(
            "SELECT operation_record_id,reservation_id,expires_at,candidate_runtime_id,generation FROM native_replacements WHERE operation_record_id=?1",
            [&prepared.record_id], |row| Ok((row.get::<_,String>(0)?, row.get::<_,String>(1)?,
                row.get::<_,i64>(2)?, row.get::<_,String>(3)?, row.get::<_,u64>(4)?))).unwrap()
    };
    let captured_reservation = original_reservation();
    assert_eq!(captured_reservation.0, prepared.record_id);
    assert_eq!(captured_reservation.4, 2);

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
    let original_target_until = tokio::time::Instant::now() + Duration::from_secs(8);
    let private_cutoff = delay_after_admit.then_some(original_target_until);
    let id = private_marker::until_phase(
        private_cutoff,
        "original-media-send",
        target_peer.send(media.clone(), true),
    )
    .await;
    let answer =
        private_marker::until_phase(private_cutoff, "original-media-answer", target_peer.read())
            .await;
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
    // Every negative uses the actual original route, socket, peer and Store
    // record, then joins those exact owners. No substitute decoder/connection.
    macro_rules! finish_failed_original {
        ($received:expr, $physical_closed:expr, $negative_cleanup_until:expr,$boundary:expr,$public_problem:expr) => {{
            tokio::time::timeout_at($negative_cleanup_until, async {
                loop {
                    let current = record(&f, &target_config, operation);
                    if matches!(
                        current.state,
                        OperationState::Failed | OperationState::Cancelled
                    ) && host.adapter().route_count() == 2
                        && host.available_slots() == 1
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("same original failed RTC target and worker must retire");
            let current = record(&f, &target_config, operation);
            assert_eq!(current.record_id, prepared.record_id);
            assert!(current.result.is_none());
            assert!(!current.effects.iter().any(|effect| {
                ["native-replacement-readiness", "native-replacement-commit"]
                    .contains(&effect.provider.as_str())
                    && effect.state == EffectState::Confirmed
            }));
            assert!(current.effects.iter().all(|effect| effect.attempt <= 1));
            assert_eq!(original_reservation(), captured_reservation);
            assert_eq!(host.adapter().total_allocations(), 3);
            assert!(!host.adapter().is_connection_live(
                &rvoip_core::ids::ConnectionId::from_string(captured_reservation.3.clone())
            ));
            assert!(host.adapter().is_connection_live(&old_runtime));
            assert!(host.adapter().is_connection_live(&retained_runtime));
            assert_eq!(connection_state(&f, "rtc-connection-0").1, 1);
            match $boundary {
                "expiry" => assert!(chrono::Utc::now().timestamp() >= captured_reservation.2),
                "operator-revoked" => assert!(f.server.store.revalidate(&key.id).is_err()),
                "physical-cancel" => assert!(!host.adapter().is_connection_live(
                    &rvoip_core::ids::ConnectionId::from_string(captured_reservation.3.clone())
                )),
                _ => unreachable!(),
            }
            // Only these independent actual original assertions qualify the
            // negative. The closed public400 alone has no internal diagnosis.
            let corroboration = OriginalNegativeTerminal;
            assert!(qualified_negative(&$public_problem, Some(&corroboration)));
            original_av(
                &mut a,
                &mut b,
                &red,
                &blue,
                &directory,
                "after-private-refusal",
            )
            .await;
            for received in $received {
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
            if !$physical_closed {
                replacement.close().await.unwrap();
            }
            first.socket.close(None).await.unwrap();
            second.socket.close(None).await.unwrap();
            target_peer.socket.close(None).await.unwrap();
            drop(first);
            drop(second);
            drop(target_peer);
            f.server.shutdown().await.unwrap();
            assert_eq!(host.adapter().route_count(), 0);
            assert_eq!(host.available_slots(), 3);
            return;
        }};
    }
    private_marker::until_phase(
        private_cutoff,
        "candidate-remote-answer",
        replacement.set_remote_answer(payload["substrate_setup"]["sdp"].as_str().unwrap()),
    )
    .await
    .unwrap();
    private_marker::until_phase(
        private_cutoff,
        "candidate-connected",
        replacement.wait_connected(Duration::from_secs(5)),
    )
    .await
    .unwrap();
    senders.push(opus::producer(&replacement, 660.0));
    senders.push(video_producer(&replacement, red.clone()));
    let mut c = private_marker::until_phase(
        private_cutoff,
        "candidate-remote-tracks",
        receive(&replacement, 2),
    )
    .await;
    if delay_after_admit || negative.is_some() {
        // Observe actual signed Store admission, never infer DTLS/resource
        // success from elapsed time or fabricate a ready decoder observation.
        private_marker::until_phase(
            private_cutoff,
            "original-resource-admission",
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    let current = record(&f, &target_config, operation);
                    if [
                        "native-replacement-start",
                        "native-replacement-attach",
                        "native-replacement-activate",
                    ]
                    .iter()
                    .all(|provider| {
                        current.effects.iter().any(|effect| {
                            effect.provider == *provider && effect.state == EffectState::Confirmed
                        })
                    }) {
                        assert_eq!(current.state, OperationState::Preparing);
                        assert!(current.result.is_none());
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }),
        )
        .await
        .expect("actual original RTC resource admission before private delay");
        let original_reservation_expiry = || {
            rusqlite::Connection::open_with_flags(
                &f.config.database,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .unwrap()
            .query_row(
                "SELECT expires_at FROM native_replacements WHERE operation_record_id=?1",
                [&prepared.record_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
        };
        let original_expiry = original_reservation_expiry();
        // This is an actual wait after admission, not a modified clock/read
        // expiry. It crosses every <=5s start capture before this boundary.
        if delay_after_admit {
            // The same original receivers keep consuming playout during the
            // deliberate input delay. Fresh marker decoding still starts only
            // after this unchanged five-second admission boundary is crossed.
            private_marker::active_playout_delay(&mut a, &mut b, &mut c, private_cutoff).await;
            // This SAME original retained publisher has never used these PCM
            // or VP8 inputs before the real boundary. Pending old packets may
            // be discarded but cannot decode as the new tone/green marker.
            retained_marker
                .as_ref()
                .unwrap()
                .select(private_marker::MARKER_HZ, green.as_ref().unwrap());
        }
        assert_eq!(original_reservation_expiry(), original_expiry);
        assert!(
            chrono::Utc::now().timestamp() < original_expiry,
            "the exact original reservation must still be valid"
        );
        let current = record(&f, &target_config, operation);
        assert_eq!(current.record_id, prepared.record_id);
        assert_eq!(current.state, OperationState::Preparing);
        assert!(current.result.is_none());
        assert!(!current
            .effects
            .iter()
            .any(|effect| effect.provider == "native-replacement-readiness"
                && effect.state == EffectState::Confirmed));
        assert_eq!(host.adapter().total_allocations(), 3);
        assert_eq!(host.adapter().route_count(), 3);
        assert!(host.adapter().is_connection_live(&old_runtime));
        assert!(host.adapter().is_connection_live(&retained_runtime));
        // Draining alone is not freshness proof; the genuine new marker below
        // must independently decode before this original eight-second cutoff.
        drain(&mut c);
    }
    let decoded_samples = if delay_after_admit {
        let decoded_samples = private_marker::audio(&mut c.audio, original_target_until).await;
        private_marker::picture(
            &mut c.video,
            green.as_ref().unwrap(),
            "private-target-post-boundary-green",
            &directory,
            1,
            original_target_until,
        )
        .await;
        // Restore the SAME original publisher before proving both ordinary
        // original routes still deliver their original tone/color below.
        retained_marker.as_ref().unwrap().select(880.0, &blue);
        decoded_samples
    } else {
        // Original peers continue ordinary playout while this SAME private
        // audio-then-video observation owns its existing bounds and decoder.
        // Leaving their 64-frame receivers idle would manufacture backpressure.
        private_marker::with_original_playout(
            [&mut a.audio, &mut a.video, &mut b.audio, &mut b.video],
            async {
                let decoded_samples = decoded_private_audio(&mut c.audio).await;
                private_marker::picture_once(
                    &mut c.video,
                    &blue,
                    "private-target-first-blue-keyframe",
                    &directory,
                    false,
                )
                .await;
                decoded_samples
            },
        )
        .await
        .expect("original receivers remain available during private observation")
    };
    let mut observation = challenge.clone();
    observation["audio"] =
        json!({"stream_id":"replacement-audio","decoded_samples":decoded_samples});
    // The preceding independent VP8 decode consumed exactly one actual source
    // keyframe on this new receiver. No RTP-only readiness is substituted.
    observation["video"] = json!({"stream_id":"replacement-video-receive","decoded_frames":1,"decoded_keyframes":1,"decoder_startup":"fresh-peer-first-decoded-frame"});
    let mut ready = renew(media);
    ready["payload"]["replacement_preparation"]["profile"] = json!("connect-replacement-ready-v1");
    ready["payload"]["replacement_preparation"]["readiness"] = observation;
    if let Some(boundary) = negative {
        let before_boundary = record(&f, &target_config, operation);
        assert_eq!(before_boundary.record_id, prepared.record_id);
        assert_eq!(
            before_boundary.state,
            OperationState::Preparing,
            "negative boundary must act on a genuinely pending original"
        );
        assert!(before_boundary.result.is_none());
        assert!(!before_boundary.effects.iter().any(|e| {
            ["native-replacement-readiness", "native-replacement-commit"]
                .contains(&e.provider.as_str())
                && e.state == EffectState::Confirmed
        }));
        assert!(host
            .adapter()
            .is_connection_live(&rvoip_core::ids::ConnectionId::from_string(
                captured_reservation.3.clone()
            )));
        assert!(host.adapter().is_connection_live(&old_runtime));
        assert!(host.adapter().is_connection_live(&retained_runtime));
        let mut negative_cleanup_until = None;
        match boundary {
            "expiry" => {
                let remaining = captured_reservation.2 - chrono::Utc::now().timestamp();
                assert!(remaining > 0);
                tokio::time::sleep(
                    Duration::from_secs(remaining as u64) + Duration::from_millis(20),
                )
                .await;
                assert!(chrono::Utc::now().timestamp() >= captured_reservation.2);
            }
            "operator-revoked" => {
                assert!(chrono::Utc::now().timestamp() < captured_reservation.2);
                let admin = f.server.store.authenticate(&f.admin).unwrap();
                let store = f.server.store.clone();
                let credential = key.id.clone();
                tokio::task::spawn_blocking(move || store.revoke(&admin, &credential))
                    .await
                    .unwrap()
                    .unwrap();
                assert!(f.server.store.revalidate(&key.id).is_err());
            }
            "physical-cancel" => {
                assert!(chrono::Utc::now().timestamp() < captured_reservation.2);
                let until = tokio::time::Instant::now() + Duration::from_secs(5);
                negative_cleanup_until = Some(until);
                tokio::time::timeout_at(until, replacement.close())
                    .await
                    .expect("same original physical close cutoff")
                    .unwrap();
                // This scenario explicitly cancels both ends of the original
                // physical fixture. Remote ICE failure propagation has no five-
                // second acknowledgement contract, so a completed local close
                // alone cannot establish the server boundary tested below.
                // The captured runtime ID was minted by this exact reservation;
                // adapter end closes its original driver/backend/controller and
                // joins the actual terminal event. It is not a Store/root write.
                let original_target =
                    rvoip_core::ids::ConnectionId::from_string(captured_reservation.3.clone());
                tokio::time::timeout_at(
                    until,
                    host.adapter().end(
                        original_target.clone(),
                        rvoip_core::adapter::EndReason::Normal,
                    ),
                )
                .await
                .expect("same original server physical teardown shares cleanup cutoff")
                .expect("same original server physical teardown must join");
                assert!(!host.adapter().is_connection_live(&original_target));
                // Require the actual original root/route/slot boundary before
                // testing whether stale genuine readiness can revive it.
                tokio::time::timeout_at(until,async {
                    loop {
                        let current=record(&f,&target_config,operation);
                        if matches!(current.state,OperationState::Failed|OperationState::Cancelled)
                            && !host.adapter().is_connection_live(&rvoip_core::ids::ConnectionId::from_string(captured_reservation.3.clone()))
                            && host.adapter().route_count()==2 && host.available_slots()==1 {
                            assert_eq!(current.record_id,prepared.record_id);
                            assert!(!current.effects.iter().any(|e|["native-replacement-readiness","native-replacement-commit"].contains(&e.provider.as_str())&&e.state==EffectState::Confirmed));
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                }).await.expect("actual original server route and root must cancel within existing cleanup cutoff");
            }
            _ => unreachable!(),
        }
        assert_eq!(original_reservation(), captured_reservation);
        // This observation came from the actual pre-boundary Opus/VP8 decoder.
        // Even a genuine signed history cannot revive current authority.
        connect_protocol::replacement_readiness::validate_marker(
            &ready["payload"]["replacement_preparation"],
        )
        .expect("genuine original readiness must remain valid wire shape");
        let negative_cleanup_until = negative_cleanup_until
            .unwrap_or_else(|| tokio::time::Instant::now() + Duration::from_secs(5));
        let id = target_peer.send(renew(ready.clone()), true).await;
        let observed_public_problem = tokio::time::timeout_at(negative_cleanup_until, async {
            let mut observed_public_problem = None;
            for _ in 0..128 {
                let event = target_peer.read().await;
                assert_eq!(event.original()["in_reply_to"], id);
                match event.kind() {
                    "error" => {
                        let problem=public_problem(&event.original()["payload"]);
                        assert!(is_public_refusal_shape(&problem),"stale valid readiness did not receive an admitted exact public problem shape");
                        observed_public_problem = Some(problem);
                        break;
                    }
                    "operation.updated" => {
                        assert!(
                            is_refusal_progress(event.original()["payload"]["state"].as_str()),
                            "stale readiness unexpectedly completed"
                        );
                    }
                    _ => panic!("stale private Ready acquired unexpected receipt kind={}",event.kind()),
                }
            }
            observed_public_problem
        })
        .await
        .expect("same original signed stale Ready refusal is bounded")
        .expect("actual signed stale Ready must return a matched public problem, not just progress");
        finish_failed_original!(
            vec![a, b, c],
            boundary == "physical-cancel",
            negative_cleanup_until,
            boundary,
            observed_public_problem
        );
    }
    for invalid in [
        "nonce",
        "generation-shape",
        "generation-owner",
        "audio",
        "video",
    ] {
        let mut bad = renew(ready.clone());
        let value = &mut bad["payload"]["replacement_preparation"]["readiness"];
        match invalid {
            "nonce" => value["nonce"] = json!("another-target-nonce"),
            "generation-shape" => value["target_generation"] = json!(3),
            "generation-owner" => {
                value["source_generation"] = json!(2);
                value["target_generation"] = json!(3);
            }
            "audio" => value["audio"]["decoded_samples"] = json!(0),
            "video" => value["video"]["decoded_keyframes"] = json!(0),
            _ => unreachable!(),
        }
        let expected = if matches!(invalid, "nonce" | "generation-owner") {
            connect_protocol::replacement_readiness::validate_marker(
                &bad["payload"]["replacement_preparation"],
            )
            .unwrap();
            "forbidden"
        } else {
            assert_eq!(
                connect_protocol::replacement_readiness::validate_marker(
                    &bad["payload"]["replacement_preparation"]
                )
                .err()
                .unwrap()
                .code,
                "invalid-request"
            );
            "invalid-request"
        };
        private_marker::until_phase(
            private_cutoff,
            "invalid-readiness-refusal",
            ready_shape_refused(&mut target_peer, bad, expected),
        )
        .await;
        let pending = record(&f, &target_config, operation);
        assert_eq!(pending.state, OperationState::Preparing);
        assert!(pending.result.is_none());
        assert_eq!(connection_state(&f, "rtc-connection-0").1, 1);
        assert!(host.adapter().is_connection_live(&old_runtime));
        assert_eq!(host.adapter().route_count(), 3);
    }
    if delay_after_admit {
        private_marker::originals(
            &mut a,
            &mut b,
            &red,
            &blue,
            &directory,
            original_target_until,
        )
        .await;
    } else {
        original_av(&mut a, &mut b, &red, &blue, &directory, "waiting-for-ready").await;
    }
    let ready_until =
        private_cutoff.unwrap_or_else(|| tokio::time::Instant::now() + Duration::from_secs(5));
    let original_ready = async {
        let uncertain = submit_original_ready(
            &mut target_peer,
            renew(ready.clone()),
            &cid,
            operation,
            &prepared.record_id,
        )
        .await;
        let done = completed(&f, &target_config, operation).await;
        if let Some(problem) = uncertain {
            assert!(ready_response_is_uncertain(&problem));
            assert_eq!(done.record_id, prepared.record_id);
            eprintln!(
                "original signed Ready unavailable response recovered through the same operation; code={} category={:?} reason={:?}",
                problem.code, problem.category, problem.reason
            );
        }
        done
    };
    let done = private_marker::until_phase(
        Some(ready_until),
        "original-ready-and-status",
        original_ready,
    )
    .await;
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
    private_marker::until_phase(
        Some(tokio::time::Instant::now() + Duration::from_secs(5)),
        "same-original-ready-replay",
        submit_original_ready(
            &mut target_peer,
            renew(ready.clone()),
            &cid,
            operation,
            &prepared.record_id,
        ),
    )
    .await;
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
    private_marker::until_phase(
        Some(tokio::time::Instant::now() + Duration::from_secs(5)),
        "same-original-ready-replay",
        submit_original_ready(
            &mut target_peer,
            renew(ready),
            &cid,
            operation,
            &prepared.record_id,
        ),
    )
    .await;
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
    private_marker::picture_once(
        &mut b.video,
        &red,
        "retained-receives-new-target-red",
        &directory,
        true,
    )
    .await;
    private_marker::picture_once(
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "explicit synthetic VP8 fixtures and independent decoder; CONNECT_GATE6_AV_FIXTURE_DIR and CONNECT_GATE6_FFMPEG"]
async fn signed_webrtc_av_private_readiness_survives_original_start_capture_after_actual_admission()
{
    full_replacement_with_private_delay("EdDSA", true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "explicit synthetic VP8 fixtures and independent decoder; CONNECT_GATE6_AV_FIXTURE_DIR and CONNECT_GATE6_FFMPEG"]
async fn signed_webrtc_av_original_reservation_expiry_refuses_genuine_private_observation() {
    full_replacement_case("EdDSA", false, Some("expiry")).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "explicit synthetic VP8 fixtures and independent decoder; CONNECT_GATE6_AV_FIXTURE_DIR and CONNECT_GATE6_FFMPEG"]
async fn signed_webrtc_av_current_operator_revocation_refuses_genuine_private_observation() {
    full_replacement_case("EdDSA", false, Some("operator-revoked")).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "explicit synthetic VP8 fixtures and independent decoder; CONNECT_GATE6_AV_FIXTURE_DIR and CONNECT_GATE6_FFMPEG"]
async fn signed_webrtc_av_original_physical_cancel_refuses_genuine_private_observation() {
    full_replacement_case("EdDSA", false, Some("physical-cancel")).await;
}
