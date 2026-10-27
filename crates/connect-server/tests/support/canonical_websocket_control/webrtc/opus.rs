//! Real 32 kbit/s Opus encoding, signed WSS and independent SRTP-side decoding.
use super::*;
use ::opus::{Application, Bitrate, Channels, Decoder, Encoder};

fn config() -> WebRtcConfig {
    let mut c = WebRtcConfig::loopback();
    c.capabilities.audio_codecs = vec![CodecInfo::from_name_with_defaults("opus")];
    c.opus_settings.max_average_bitrate_bps = 32000;
    c
}

pub(super) fn producer(
    peer: &Arc<RvoipPeerConnection>,
    hz: f32,
) -> (CancellationToken, tokio::task::JoinHandle<u64>) {
    let track = peer.local_audio_track().unwrap();
    let ssrc = peer.local_audio_ssrc().unwrap();
    let stop = CancellationToken::new();
    let cancel = stop.clone();
    let task = tokio::spawn(async move {
        let mut encoder = Encoder::new(48000, Channels::Mono, Application::Voip).unwrap();
        encoder.set_bitrate(Bitrate::Bits(32000)).unwrap();
        let mut sequence = u16::MAX - 5;
        let mut sent = 0u64;
        let mut tick = tokio::time::interval(Duration::from_millis(20));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! { biased; _=cancel.cancelled()=>break, _=tick.tick()=>{} }
            let pcm: Vec<i16> = (0..960)
                .map(|n| {
                    let phase = ((sent * 960 + n) as f32 * hz * std::f32::consts::TAU) / 48000.0;
                    (phase.sin() * 12000.0) as i16
                })
                .collect();
            let mut bytes = [0u8; 1276];
            let count = encoder.encode(&pcm, &mut bytes).unwrap();
            assert_eq!(
                ::opus::packet::get_nb_samples(&bytes[..count], 48000).unwrap(),
                960
            );
            let mut packet = rvoip_webrtc::media::opus_rtp_packet_for_ssrc(
                ssrc,
                sequence,
                (sent as u32).wrapping_mul(960),
            );
            packet.payload = bytes[..count].to_vec().into();
            let result = tokio::time::timeout(
                Duration::from_millis(500),
                track.write_rtp_with_extensions(packet, &[]),
            )
            .await;
            if !matches!(result, Ok(Ok(()))) {
                tracing::warn!(
                    ?result,
                    sent,
                    ssrc,
                    "Synthetic Opus source stopped on its original write result"
                );
                break;
            }
            sequence = sequence.wrapping_add(1);
            sent += 1;
        }
        sent
    });
    (stop, task)
}

async fn receiving(
    peer: &Arc<RvoipPeerConnection>,
    i: usize,
) -> (
    Arc<rvoip_webrtc::media::WebRtcMediaStream>,
    mpsc::Receiver<rvoip_core::stream::MediaFrame>,
) {
    let remote = tokio::time::timeout(Duration::from_secs(5), peer.recv_remote_track())
        .await
        .unwrap()
        .unwrap();
    let stream = rvoip_webrtc::media::from_tracks(
        StreamId::from_string(format!("opus-independent-{i}")),
        CodecInfo::from_name_with_defaults("opus"),
        peer.local_audio_track().unwrap(),
        peer.local_audio_ssrc().unwrap(),
        111,
        Some(remote),
    );
    let rx = stream.try_frames_in().unwrap();
    (stream, rx)
}

pub(super) async fn tone(
    rx: &mut mpsc::Receiver<rvoip_core::stream::MediaFrame>,
    expected_hz: f32,
    absent: bool,
) {
    let mut decoder = Decoder::new(48000, Channels::Mono).unwrap();
    let mut pcm = [0i16; 5760];
    let until = tokio::time::Instant::now() + Duration::from_secs(if absent { 1 } else { 5 });
    let mut matched = 0;
    let mut previous_timestamp = None;
    loop {
        let frame = match tokio::time::timeout_at(until, rx.recv()).await {
            Ok(Some(frame)) => frame,
            Err(_) if absent => return,
            other => panic!("actual Opus output for {expected_hz} Hz: {other:?}"),
        };
        assert_eq!(frame.payload_type, Some(111));
        if let Some(previous) = previous_timestamp {
            let delta = frame.timestamp_rtp.wrapping_sub(previous);
            assert!(
                delta > 0 && delta % 960 == 0,
                "Opus RTP must advance on its 48 kHz / 20 ms clock: {delta}"
            );
        }
        previous_timestamp = Some(frame.timestamp_rtp);
        assert_eq!(
            ::opus::packet::get_nb_samples(&frame.payload, 48000).unwrap(),
            960
        );
        let count = decoder.decode(&frame.payload, &mut pcm, false).unwrap();
        assert_eq!(count, 960);
        let samples = &pcm[..count];
        let energy = samples.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>();
        if absent {
            assert!(
                (energy / count as f64).sqrt() < 500.0,
                "withdrawn Opus source remained audible"
            );
            continue;
        }
        let crossings = samples.windows(2).filter(|p| p[0] <= 0 && p[1] > 0).count() as f32;
        let hz = crossings * 48000.0 / count as f32;
        if (energy / count as f64).sqrt() > 2000.0 && (hz - expected_hz).abs() < 75.0 {
            matched += 1;
            if matched == 5 {
                return;
            }
        }
    }
}

async fn run(algorithm: &str) {
    let f = configured_end_scope(algorithm, 8, None, true).await;
    let host = SignalingHost::new(config(), 2).unwrap();
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
        let peer = RvoipPeerConnection::new(&config(), PeerRole::Offerer)
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
        senders.push(producer(&peer, if i == 0 { 440.0 } else { 880.0 }));
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
    let (stream_a, mut a) = receiving(&peers[0], 0).await;
    let (stream_b, mut b) = receiving(&peers[1], 1).await;
    tone(&mut a, 880.0, false).await;
    tone(&mut b, 440.0, false).await;
    command(
        &mut first,
        scoped(
            &cid,
            0,
            "stream.unsubscribe",
            "opus-withdraw-a",
            2,
            json!({"strm_ids":["rtc-audio-1"]}),
        ),
    )
    .await;
    completed(&f, &f.client, "opus-withdraw-a").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    while a.try_recv().is_ok() {}
    tone(&mut a, 880.0, true).await;
    tone(&mut b, 440.0, false).await;
    command(
        &mut first,
        scoped(
            &cid,
            0,
            "stream.subscribe",
            "opus-restore-a",
            3,
            json!({"subscriptions":[{"strm_id":"rtc-audio-1"}]}),
        ),
    )
    .await;
    completed(&f, &f.client, "opus-restore-a").await;
    tone(&mut a, 880.0, false).await;
    assert_eq!(host.adapter().total_allocations(), 2);
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
    f.server.shutdown().await.unwrap();
    assert_eq!(host.adapter().route_count(), 0);
    assert_eq!(host.available_slots(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn signed_opus_decoded_duplex_subscription_withdrawal_eddsa() {
    run("EdDSA").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn signed_opus_decoded_duplex_subscription_withdrawal_es256() {
    run("ES256").await;
}

#[test]
fn canonical_opus_host_refuses_unsupported_encoder_profiles() {
    let mut stereo = config();
    stereo.opus_settings.stereo = true;
    let mut channels = config();
    channels.capabilities.audio_codecs[0].channels = 2;
    let mut duration = config();
    duration.opus_settings.min_ptime_ms = 40;
    let mut bitrate = config();
    bitrate.opus_settings.max_average_bitrate_bps = 16000;
    for unsupported in [stereo, channels, duration, bitrate] {
        assert!(SignalingHost::new(unsupported, 2).is_err());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn signed_opus_refuses_unmet_sdp_profiles_without_media_admission() {
    for case in ["packet-duration", "stereo", "extra-video", "extra-data"] {
        let f = configured_end_scope("EdDSA", 8, None, false).await;
        let host = SignalingHost::new(config(), 1).unwrap();
        f.server
            .service
            .install_canonical_webrtc(host.clone())
            .unwrap();
        provision_config(&f, &f.client, 0).await;
        let other = canonical_webtransport_media::other(&f);
        let (cid, _, _) = canonical_webtransport_media::accepted_caps(
            &f,
            &other,
            "voice",
            json!({"audio":["opus"]}),
        )
        .await;
        let mut remote_config = config();
        if case == "stereo" {
            remote_config.opus_settings.stereo = true;
        }
        let peer = RvoipPeerConnection::new(&remote_config, PeerRole::Offerer)
            .await
            .unwrap();
        peer.add_local_audio_track().await.unwrap();
        if case == "extra-video" {
            peer.add_local_video_track().await.unwrap();
        }
        if case == "extra-data" {
            peer.create_data_channel(
                "unoffered-data",
                rvoip_webrtc::peer::DataChannelOptions::reliable(),
            )
            .await
            .unwrap();
        }
        let mut sdp = peer.create_offer_and_gather().await.unwrap();
        if case == "packet-duration" {
            sdp.push_str("a=maxptime:10\r\n");
        }
        if case == "extra-video" {
            assert!(sdp.contains("m=video "));
        }
        if case == "extra-data" {
            assert!(sdp.contains("m=application "));
        }
        let operation = format!("opus-refuse-{case}");
        let request = scoped(
            &cid,
            0,
            "connection.offer",
            &operation,
            0,
            json!({
                "substrate":"websocket+webrtc", "capabilities":{"audio_codecs":[{"name":"opus","params":{"sample_rate":48000,"channels":1}}],"max_streams_per_connection":1,"identity_assurance_offered":"identified"},
                "streams_offered":[{"id":"rtc-audio-0","kind":"audio","direction":"sendrecv","codec_preferences":["opus"]}],
                "substrate_setup":{"kind":"websocket+webrtc","sdp":sdp}
            }),
        );
        let mut ws = Peer::open(&f).await;
        refused(&mut ws, request, "unsupported-media-profile", 501).await;
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if host.available_slots() == 1
                    && host.adapter().route_count() == 0
                    && f.server
                        .service
                        .canonical_runtime_snapshot()
                        .unwrap()
                        .connections
                        == 0
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            record(&f, &f.client, &operation).state,
            OperationState::Failed
        );
        peer.close().await.unwrap();
        ws.socket.close(None).await.unwrap();
        drop(ws);
        f.server.shutdown().await.unwrap();
    }
}
