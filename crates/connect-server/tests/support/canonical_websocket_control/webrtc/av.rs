//! Signed WSS/Store/Core audio+video. Explicit fixture/decoder lane; no camera.
use super::*;
use rvoip_core::stream::MediaFrame;
use rvoip_webrtc::media::{from_tracks, WebRtcMediaStream};
use std::path::{Path, PathBuf};

fn config() -> WebRtcConfig {
    let mut c = WebRtcConfig::loopback();
    c.require_vp8_audio_video_profile = true;
    c.capabilities.audio_codecs = vec![CodecInfo::from_name_with_defaults("opus")];
    c.capabilities.video_codecs = vec![CodecInfo::from_name_with_defaults("vp8")];
    c.opus_settings.max_average_bitrate_bps = 32000;
    c
}
fn request(cid: &str, i: usize, sdp: &str) -> Value {
    scoped(
        &cid,
        i,
        "connection.offer",
        &format!("rtc-offer-{i}"),
        0,
        json!({
            "substrate":"websocket+webrtc",
            "capabilities":{"audio_codecs":[{"name":"opus","params":{"sample_rate":48000,"channels":1}}],"video_codecs":[{"name":"vp8","params":{"sample_rate":90000,"channels":0}}],"max_streams_per_connection":2,"identity_assurance_offered":"identified"},
            "streams_offered":[{"id":format!("rtc-audio-{i}"),"kind":"audio","direction":"sendrecv","codec_preferences":["opus"]},{"id":format!("rtc-video-{i}"),"kind":"video","direction":"sendrecv","codec_preferences":["vp8"]}],
            "substrate_setup":{"kind":"websocket+webrtc","sdp":sdp}
        }),
    )
}
fn video_producer(
    peer: &Arc<RvoipPeerConnection>,
    frame: Vec<u8>,
) -> (CancellationToken, tokio::task::JoinHandle<u64>) {
    let track = peer.local_video_track().unwrap();
    let ssrc = peer.local_video_ssrc().unwrap();
    let stop = CancellationToken::new();
    let cancel = stop.clone();
    let task = tokio::spawn(async move {
        let mut sent = 0u64;
        let mut tick = tokio::time::interval(Duration::from_millis(60));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {biased; _=cancel.cancelled()=>break, _=tick.tick()=>{}}
            let mut packet = rvoip_webrtc::media::vp8_rtp_packet_for_ssrc(
                ssrc,
                (sent as u16).wrapping_add(1),
                (sent as u32).wrapping_mul(5400),
            );
            packet.header.marker = true;
            let mut body = vec![0x10];
            body.extend_from_slice(&frame);
            packet.payload = body.into();
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
                    "Synthetic video source stopped on its original write result"
                );
                break;
            }
            sent += 1;
        }
        sent
    });
    (stop, task)
}
struct Received {
    streams: Vec<Arc<WebRtcMediaStream>>,
    audio: mpsc::Receiver<MediaFrame>,
    video: mpsc::Receiver<MediaFrame>,
}
async fn receive(peer: &Arc<RvoipPeerConnection>, i: usize) -> Received {
    let mut streams = vec![];
    let mut audio = None;
    let mut video = None;
    for _ in 0..2 {
        let remote = tokio::time::timeout(Duration::from_secs(5), peer.recv_remote_track())
            .await
            .unwrap()
            .unwrap();
        let is_video = remote
            .kind()
            .await
            .to_string()
            .eq_ignore_ascii_case("video");
        let stream = if is_video {
            assert!(video.is_none());
            from_tracks(
                StreamId::from_string(format!("av-video-{i}")),
                CodecInfo::from_name_with_defaults("vp8"),
                peer.local_video_track().unwrap(),
                peer.local_video_ssrc().unwrap(),
                96,
                Some(remote),
            )
        } else {
            assert!(audio.is_none());
            from_tracks(
                StreamId::from_string(format!("av-audio-{i}")),
                CodecInfo::from_name_with_defaults("opus"),
                peer.local_audio_track().unwrap(),
                peer.local_audio_ssrc().unwrap(),
                111,
                Some(remote),
            )
        };
        let rx = stream.try_frames_in().unwrap();
        if is_video {
            video = Some(rx);
        } else {
            audio = Some(rx);
        }
        streams.push(stream);
    }
    Received {
        streams,
        audio: audio.unwrap(),
        video: video.unwrap(),
    }
}
async fn picture(
    rx: &mut mpsc::Receiver<MediaFrame>,
    expected: &[u8],
    label: &str,
    dir: &Path,
    red: bool,
) {
    let frame = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(frame.payload_type, Some(96));
    let rtp = rvoip_rtp_core::packet::RtpPacket::parse(&frame.payload).unwrap();
    assert!(rtp.header.marker);
    assert_eq!(rtp.header.payload_type, 96);
    assert_eq!(rtp.header.timestamp, frame.timestamp_rtp);
    assert_eq!(rtp.payload[0], 0x10);
    assert_eq!(&rtp.payload[1..], expected);
    let mut ivf = Vec::from(&b"DKIF\x00\x00\x20\x00VP80"[..]);
    ivf.extend_from_slice(&64u16.to_le_bytes());
    ivf.extend_from_slice(&64u16.to_le_bytes());
    ivf.extend_from_slice(&90000u32.to_le_bytes());
    ivf.extend_from_slice(&1u32.to_le_bytes());
    ivf.extend_from_slice(&1u32.to_le_bytes());
    ivf.extend_from_slice(&0u32.to_le_bytes());
    ivf.extend_from_slice(&(expected.len() as u32).to_le_bytes());
    ivf.extend_from_slice(&u64::from(frame.timestamp_rtp).to_le_bytes());
    ivf.extend_from_slice(&rtp.payload[1..]);
    let input = dir.join(format!("{label}.ivf"));
    let output = dir.join(format!("{label}.rgb"));
    std::fs::write(&input, ivf).unwrap();
    let decoded = std::process::Command::new(
        std::env::var("CONNECT_GATE6_FFMPEG").expect("explicit local decoder path"),
    )
    .args([
        "-nostdin",
        "-y",
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        "ivf",
        "-c:v",
        "vp8",
        "-i",
    ])
    .arg(input)
    .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24"])
    .arg(&output)
    .output()
    .unwrap();
    std::fs::write(dir.join(format!("{label}-decode.log")), decoded.stderr).unwrap();
    assert!(decoded.status.success());
    let pixels = std::fs::read(output).unwrap();
    assert_eq!(pixels.len(), 64 * 64 * 3);
    let mean = [0, 1, 2].map(|i| {
        pixels[i..]
            .iter()
            .step_by(3)
            .map(|v| u64::from(*v))
            .sum::<u64>()
            / 4096
    });
    if red {
        assert!(mean[0] > 200 && mean[1] < 40 && mean[2] < 40, "{mean:?}");
    } else {
        assert!(mean[2] > 200 && mean[0] < 40 && mean[1] < 40, "{mean:?}");
    }
}
async fn no_video(rx: &mut mpsc::Receiver<MediaFrame>) {
    tokio::time::sleep(Duration::from_millis(300)).await;
    while rx.try_recv().is_ok() {}
    assert!(
        tokio::time::timeout(Duration::from_millis(400), rx.recv())
            .await
            .is_err(),
        "withdrawn video still delivers"
    );
}
async fn run(algorithm: &str) {
    Box::pin(run_profile(algorithm, false)).await;
}
async fn run_profile(algorithm: &str, configured: bool) {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .try_init();
    let fixtures = PathBuf::from(
        std::env::var("CONNECT_GATE6_AV_FIXTURE_DIR")
            .expect("explicit synthetic source fixture directory"),
    );
    let red = std::fs::read(fixtures.join("red.vp8")).unwrap();
    let blue = std::fs::read(fixtures.join("blue.vp8")).unwrap();
    assert_eq!((red.len(), blue.len()), (64, 64));
    let dir = fixtures.join(format!("signed-{algorithm}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let mut f = configured_end_scope(algorithm, 8, None, true).await;
    let host = if configured {
        f.server.shutdown().await.unwrap();
        f.config.webrtc = Some(connect_server::canonical_webrtc::CanonicalWebRtcConfig {
            profile: connect_server::canonical_webrtc::CanonicalWebRtcProfile::OpusVp8,
            max_peers: 2,
            addressing: None,
        });
        f.server = RunningCanonicalServer::start(f.config.clone())
            .await
            .unwrap();
        f.client.endpoint = f.server.address;
        f.server
            .test_webrtc_host()
            .expect("configured production RTC host")
    } else {
        let host = SignalingHost::new(config(), 2).unwrap();
        f.server
            .service
            .install_canonical_webrtc(host.clone())
            .unwrap();
        host
    };
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
        let original = request(&cid, i, &sdp);
        let id = ws.send(original.clone(), true).await;
        let answer = ws.read().await;
        assert_eq!(
            answer.kind(),
            "connection.answer",
            "{:?}",
            answer.original()
        );
        assert_eq!(answer.original()["in_reply_to"], id);
        for scope in ["cid", "sid", "connid", "operation_id"] {
            assert_eq!(answer.original()[scope], original[scope]);
        }
        let payload = &answer.original()["payload"];
        assert_eq!(payload["streams_answered"].as_array().unwrap().len(), 2);
        for (j, (kind, codec, pt, rate, channels)) in [
            ("audio", "opus", 111, 48000, 1),
            ("video", "vp8", 96, 90000, 0),
        ]
        .into_iter()
        .enumerate()
        {
            let stream = &payload["streams_answered"][j];
            assert_eq!(
                stream["id"],
                original["payload"]["streams_offered"][j]["id"]
            );
            assert_eq!(stream["kind"], kind);
            assert_eq!(stream["codec"]["name"], codec);
            assert_eq!(
                stream["codec"]["params"],
                json!({"payload_type":pt,"sample_rate":rate,"channels":channels})
            );
        }
        peer.set_remote_answer(payload["substrate_setup"]["sdp"].as_str().unwrap())
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
    assert_eq!(host.adapter().total_allocations(), 2);
    for peer in &peers {
        assert!(
            tokio::time::timeout(Duration::from_millis(300), peer.recv_remote_track())
                .await
                .is_err(),
            "no implicit AV playback"
        );
    }
    for (i, ws, credential, participant) in [
        (0, &mut first, &f.client, &recipient),
        (1, &mut second, &other, &initiator),
    ] {
        command(ws,scoped(&cid,i,"stream.subscribe",&format!("av-select-{i}"),1,json!({"subscriptions":[{"from_participant":participant,"kinds":["audio"]},{"strm_id":format!("rtc-video-{}",1-i)}]}))).await;
        completed(&f, credential, &format!("av-select-{i}")).await;
    }
    let mut a = receive(&peers[0], 0).await;
    let mut b = receive(&peers[1], 1).await;
    opus::tone(&mut a.audio, 880.0, false).await;
    opus::tone(&mut b.audio, 440.0, false).await;
    picture(&mut a.video, &blue, "alice-receives-blue", &dir, false).await;
    picture(&mut b.video, &red, "bob-receives-red", &dir, true).await;
    command(
        &mut first,
        scoped(
            &cid,
            0,
            "stream.unsubscribe",
            "av-withdraw-video",
            2,
            json!({"strm_ids":["rtc-video-1"]}),
        ),
    )
    .await;
    completed(&f, &f.client, "av-withdraw-video").await;
    no_video(&mut a.video).await;
    // Require newly received media after the withdrawal observation, rather
    // than satisfying continued delivery with packets buffered beforehand.
    while a.audio.try_recv().is_ok() {}
    while b.audio.try_recv().is_ok() {}
    while b.video.try_recv().is_ok() {}
    opus::tone(&mut a.audio, 880.0, false).await;
    opus::tone(&mut b.audio, 440.0, false).await;
    picture(&mut b.video, &red, "bob-continues-red", &dir, true).await;
    command(
        &mut first,
        scoped(
            &cid,
            0,
            "stream.subscribe",
            "av-restore-video",
            3,
            json!({"subscriptions":[{"strm_id":"rtc-video-1"}]}),
        ),
    )
    .await;
    completed(&f, &f.client, "av-restore-video").await;
    picture(&mut a.video, &blue, "alice-restores-blue", &dir, false).await;
    // Gate only the first participant's video publication on its original
    // connection. Audio and the opposite video selection must remain live.
    command(
        &mut first,
        scoped(
            &cid,
            0,
            "connection.update",
            "av-mute-video",
            4,
            json!({"action":"mute","streams":["rtc-video-0"]}),
        ),
    )
    .await;
    completed(&f, &f.client, "av-mute-video").await;
    no_video(&mut b.video).await;
    while a.audio.try_recv().is_ok() {}
    while b.audio.try_recv().is_ok() {}
    while a.video.try_recv().is_ok() {}
    opus::tone(&mut a.audio, 880.0, false).await;
    opus::tone(&mut b.audio, 440.0, false).await;
    picture(
        &mut a.video,
        &blue,
        "alice-blue-during-video-mute",
        &dir,
        false,
    )
    .await;
    command(
        &mut first,
        scoped(
            &cid,
            0,
            "connection.update",
            "av-unmute-video",
            5,
            json!({"action":"unmute","streams":["rtc-video-0"]}),
        ),
    )
    .await;
    completed(&f, &f.client, "av-unmute-video").await;
    picture(&mut b.video, &red, "bob-red-after-unmute", &dir, true).await;
    assert_eq!(host.adapter().total_allocations(), 2);
    for stream in a.streams.into_iter().chain(b.streams) {
        stream.close().await.unwrap();
    }
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
#[ignore = "explicit local synthetic VP8 files and independent FFmpeg decoder; set CONNECT_GATE6_AV_FIXTURE_DIR and CONNECT_GATE6_FFMPEG"]
async fn signed_av_opus_vp8_decoded_duplex_eddsa() {
    run("EdDSA").await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "explicit local synthetic VP8 files and independent FFmpeg decoder; set CONNECT_GATE6_AV_FIXTURE_DIR and CONNECT_GATE6_FFMPEG"]
async fn signed_av_opus_vp8_decoded_duplex_es256() {
    run("ES256").await;
}

#[test]
fn canonical_av_host_requires_exact_video_profile() {
    assert!(SignalingHost::new(config(), 1).is_ok());
    for bad in ["missing", "extra", "codec", "rate", "channels", "fmtp"] {
        let mut c = config();
        match bad {
            "missing" => c.capabilities.video_codecs.clear(),
            "extra" => c
                .capabilities
                .video_codecs
                .push(CodecInfo::from_name_with_defaults("vp9")),
            "codec" => c.capabilities.video_codecs[0].name = "vp9".into(),
            "rate" => c.capabilities.video_codecs[0].clock_rate_hz = 8000,
            "channels" => c.capabilities.video_codecs[0].channels = 1,
            "fmtp" => c.capabilities.video_codecs[0].fmtp = Some("max-fr=30".into()),
            _ => unreachable!(),
        }
        assert!(SignalingHost::new(c, 1).is_err(), "{bad}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn signed_av_refuses_unmatched_or_extra_sdp_before_media_admission() {
    for case in [
        "missing-video",
        "extra-data",
        "duplicate-mid",
        "audio-only-host",
    ] {
        let f = configured_end_scope("EdDSA", 8, None, false).await;
        let mut c = config();
        if case == "audio-only-host" {
            c.require_vp8_audio_video_profile = false;
        }
        let host = SignalingHost::new(c, 1).unwrap();
        f.server
            .service
            .install_canonical_webrtc(host.clone())
            .unwrap();
        let other = canonical_webtransport_media::other(&f);
        provision_media_config(&f, &f.client, 0, &["audio", "video"]).await;
        let (cid, _, _) = canonical_webtransport_media::accepted_caps(
            &f,
            &other,
            "voice+video",
            json!({"audio":["opus"],"video":["vp8"]}),
        )
        .await;
        let remote = RvoipPeerConnection::new(&config(), PeerRole::Offerer)
            .await
            .unwrap();
        remote.add_local_audio_track().await.unwrap();
        if case != "missing-video" {
            remote.add_local_video_track().await.unwrap();
        }
        if case == "extra-data" {
            remote
                .create_data_channel(
                    "unoffered-data",
                    rvoip_webrtc::peer::DataChannelOptions::reliable(),
                )
                .await
                .unwrap();
        }
        let mut sdp = remote.create_offer_and_gather().await.unwrap();
        if case == "duplicate-mid" {
            let mids: Vec<_> = sdp
                .lines()
                .filter_map(|l| l.strip_prefix("a=mid:"))
                .map(str::to_owned)
                .collect();
            assert_eq!(mids.len(), 2);
            sdp = sdp.replace(
                &format!("a=mid:{}\r\n", mids[1]),
                &format!("a=mid:{}\r\n", mids[0]),
            );
        }
        let mut ws = Peer::open(&f).await;
        refused(
            &mut ws,
            request(&cid, 0, &sdp),
            "unsupported-media-profile",
            501,
        )
        .await;
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
            record(&f, &f.client, "rtc-offer-0").state,
            OperationState::Failed,
            "{case}"
        );
        remote.close().await.unwrap();
        ws.socket.close(None).await.unwrap();
        drop(ws);
        f.server.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "explicit local decoded AV/production configuration lane"]
async fn configured_signed_av_opus_vp8_decoded_duplex_eddsa() {
    Box::pin(run_profile("EdDSA", true)).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "explicit local decoded AV/production configuration lane"]
async fn configured_signed_av_opus_vp8_decoded_duplex_es256() {
    Box::pin(run_profile("ES256", true)).await;
}
