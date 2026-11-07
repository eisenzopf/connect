//! Same original synthetic publishers; genuine new PCM/VP8 input after boundary.
use super::*;
use std::sync::{
    atomic::{AtomicU32, Ordering},
    Mutex,
};
pub(super) const MARKER_HZ: f32 = 1230.0;
pub(super) type Producer = (CancellationToken, tokio::task::JoinHandle<u64>);
pub(super) struct Marker {
    hz: Arc<AtomicU32>,
    video: Arc<Mutex<Vec<u8>>>,
}
impl Marker {
    pub(super) fn select(&self, hz: f32, video: &[u8]) {
        // Change input to the SAME encoder/track/task; never rewrap an old RTP packet.
        self.hz.store(hz.to_bits(), Ordering::Release);
        *self.video.lock().unwrap() = video.to_vec();
    }
}
pub(super) fn producers(
    peer: &Arc<RvoipPeerConnection>,
    hz: f32,
    frame: Vec<u8>,
) -> (Vec<Producer>, Marker) {
    let audio_hz = Arc::new(AtomicU32::new(hz.to_bits()));
    let input = Arc::new(Mutex::new(frame));
    let marker = Marker {
        hz: audio_hz.clone(),
        video: input.clone(),
    };
    let audio_track = peer.local_audio_track().unwrap();
    let audio_ssrc = peer.local_audio_ssrc().unwrap();
    let video_track = peer.local_video_track().unwrap();
    let video_ssrc = peer.local_video_ssrc().unwrap();
    let audio_stop = CancellationToken::new();
    let video_stop = CancellationToken::new();
    let stop = audio_stop.clone();
    let audio = tokio::spawn(async move {
        let mut encoder =
            ::opus::Encoder::new(48000, ::opus::Channels::Mono, ::opus::Application::Voip).unwrap();
        encoder.set_bitrate(::opus::Bitrate::Bits(32000)).unwrap();
        let mut sequence = u16::MAX - 5;
        let mut sent = 0u64;
        let mut tick = tokio::time::interval(Duration::from_millis(20));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {biased; _=stop.cancelled()=>break, _=tick.tick()=>{}}
            let hz = f32::from_bits(audio_hz.load(Ordering::Acquire));
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
                audio_ssrc,
                sequence,
                (sent as u32).wrapping_mul(960),
            );
            packet.payload = bytes[..count].to_vec().into();
            if !matches!(
                tokio::time::timeout(
                    Duration::from_millis(500),
                    audio_track.write_rtp_with_extensions(packet, &[])
                )
                .await,
                Ok(Ok(()))
            ) {
                break;
            }
            sent += 1;
            sequence = sequence.wrapping_add(1);
        }
        sent
    });
    let stop = video_stop.clone();
    let video = tokio::spawn(async move {
        let mut sent = 0u64;
        let mut tick = tokio::time::interval(Duration::from_millis(60));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {biased; _=stop.cancelled()=>break, _=tick.tick()=>{}}
            let frame = input.lock().unwrap().clone();
            let mut packet = rvoip_webrtc::media::vp8_rtp_packet_for_ssrc(
                video_ssrc,
                (sent as u16).wrapping_add(1),
                (sent as u32).wrapping_mul(5400),
            );
            packet.header.marker = true;
            let mut body = vec![0x10];
            body.extend_from_slice(&frame);
            packet.payload = body.into();
            if !matches!(
                tokio::time::timeout(
                    Duration::from_millis(500),
                    video_track.write_rtp_with_extensions(packet, &[])
                )
                .await,
                Ok(Ok(()))
            ) {
                break;
            }
            sent += 1;
        }
        sent
    });
    (vec![(audio_stop, audio), (video_stop, video)], marker)
}
/// Every positive private-phase await uses one unchanged original cutoff.
pub(super) async fn until_phase<T>(
    cutoff: Option<tokio::time::Instant>,
    phase: &str,
    future: impl std::future::Future<Output = T>,
) -> T {
    match cutoff {
        Some(cutoff) => {
            assert!(
                tokio::time::Instant::now() < cutoff,
                "original private cutoff elapsed before await phase={phase}"
            );
            let result = tokio::time::timeout_at(cutoff, future)
                .await
                .unwrap_or_else(|_| {
                    panic!("same original eight-second private-phase deadline phase={phase}")
                });
            assert!(
                tokio::time::Instant::now() < cutoff,
                "private phase completed after original cutoff phase={phase}"
            );
            result
        }
        None => future.await,
    }
}
pub(super) async fn active_playout_delay(
    a: &mut Received,
    b: &mut Received,
    c: &mut Received,
    cutoff: Option<tokio::time::Instant>,
) {
    until_phase(cutoff, "post-admission-5100ms-active-playout", async {
        let delay = tokio::time::sleep(Duration::from_millis(5100));
        tokio::pin!(delay);
        loop {
            tokio::select! {biased;
                _ = &mut delay => break,
                frame = a.audio.recv() => assert!(frame.is_some(), "original audio receiver closed during playout"),
                frame = a.video.recv() => assert!(frame.is_some(), "original video receiver closed during playout"),
                frame = b.audio.recv() => assert!(frame.is_some(), "retained audio receiver closed during playout"),
                frame = b.video.recv() => assert!(frame.is_some(), "retained video receiver closed during playout"),
                frame = c.audio.recv() => assert!(frame.is_some(), "candidate audio receiver closed during playout"),
                frame = c.video.recv() => assert!(frame.is_some(), "candidate video receiver closed during playout"),
            }
        }
    })
    .await;
}

pub(super) async fn audio(
    rx: &mut mpsc::Receiver<MediaFrame>,
    cutoff: tokio::time::Instant,
) -> u64 {
    let mut decoder = ::opus::Decoder::new(48000, ::opus::Channels::Mono).unwrap();
    let mut pcm = [0i16; 5760];
    let mut matched = 0;
    let mut samples = 0;
    let mut previous = None;
    loop {
        let frame = until_phase(Some(cutoff), "private-audio-marker", rx.recv())
            .await
            .expect("original private audio owner");
        assert_eq!(frame.payload_type, Some(111));
        if let Some(previous) = previous {
            let delta = frame.timestamp_rtp.wrapping_sub(previous);
            assert!(delta > 0 && delta % 960 == 0);
        }
        previous = Some(frame.timestamp_rtp);
        assert_eq!(
            ::opus::packet::get_nb_samples(&frame.payload, 48000).unwrap(),
            960
        );
        let count = decoder.decode(&frame.payload, &mut pcm, false).unwrap();
        assert_eq!(count, 960);
        let energy = pcm[..count]
            .iter()
            .map(|x| f64::from(*x).powi(2))
            .sum::<f64>();
        let crossings = pcm[..count]
            .windows(2)
            .filter(|p| p[0] <= 0 && p[1] > 0)
            .count() as f32;
        let hz = crossings * 48000.0 / count as f32;
        if (energy / count as f64).sqrt() > 2000.0 && (hz - MARKER_HZ).abs() < 75.0 {
            matched += 1;
            samples += count as u64;
        } else {
            matched = 0;
            samples = 0;
        }
        if matched == 5 {
            return samples;
        }
    }
}
pub(super) async fn picture(
    rx: &mut mpsc::Receiver<MediaFrame>,
    expected: &[u8],
    label: &str,
    dir: &Path,
    channel: usize,
    cutoff: tokio::time::Instant,
) {
    let frame = loop {
        let frame = until_phase(Some(cutoff), label, rx.recv())
            .await
            .expect("original private video owner");
        assert_eq!(frame.payload_type, Some(96));
        let packet = rvoip_rtp_core::packet::RtpPacket::parse(&frame.payload).unwrap();
        assert!(packet.header.marker);
        assert_eq!(packet.header.timestamp, frame.timestamp_rtp);
        assert_eq!(packet.payload[0], 0x10);
        if &packet.payload[1..] == expected {
            break frame;
        }
    };
    decode(frame, expected, label, dir, channel, cutoff).await;
}
pub(super) async fn picture_once(
    rx: &mut mpsc::Receiver<MediaFrame>,
    expected: &[u8],
    label: &str,
    dir: &Path,
    red: bool,
) {
    let cutoff = tokio::time::Instant::now() + Duration::from_secs(5);
    let frame = until_phase(Some(cutoff), label, rx.recv())
        .await
        .expect("original RTC video owner");
    assert_eq!(frame.payload_type, Some(96));
    let packet = rvoip_rtp_core::packet::RtpPacket::parse(&frame.payload).unwrap();
    assert!(packet.header.marker);
    assert_eq!(packet.header.timestamp, frame.timestamp_rtp);
    assert_eq!(packet.payload[0], 0x10);
    assert_eq!(&packet.payload[1..], expected);
    decode(frame, expected, label, dir, if red { 0 } else { 2 }, cutoff).await;
}
async fn decode(
    frame: MediaFrame,
    expected: &[u8],
    label: &str,
    dir: &Path,
    channel: usize,
    cutoff: tokio::time::Instant,
) {
    let packet = rvoip_rtp_core::packet::RtpPacket::parse(&frame.payload).unwrap();
    let mut ivf = Vec::from(&b"DKIF\x00\x00\x20\x00VP80"[..]);
    ivf.extend_from_slice(&64u16.to_le_bytes());
    ivf.extend_from_slice(&64u16.to_le_bytes());
    ivf.extend_from_slice(&90000u32.to_le_bytes());
    ivf.extend_from_slice(&1u32.to_le_bytes());
    ivf.extend_from_slice(&1u32.to_le_bytes());
    ivf.extend_from_slice(&0u32.to_le_bytes());
    ivf.extend_from_slice(&(expected.len() as u32).to_le_bytes());
    ivf.extend_from_slice(&u64::from(frame.timestamp_rtp).to_le_bytes());
    ivf.extend_from_slice(&packet.payload[1..]);
    let input = dir.join(format!("{label}.ivf"));
    let output = dir.join(format!("{label}.rgb"));
    std::fs::write(&input, ivf).unwrap();
    let stderr = std::fs::File::create(dir.join(format!("{label}-decode.log"))).unwrap();
    let mut child = tokio::process::Command::new(
        std::env::var("CONNECT_GATE6_FFMPEG").expect("explicit local decoder"),
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
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::from(stderr))
    .kill_on_drop(true)
    .spawn()
    .unwrap();
    // This wait owns its cutoff; no enclosing timeout can drop the original
    // child before explicit kill+join. Forced decode always fails the test.
    let decode_until = cutoff.min(tokio::time::Instant::now() + Duration::from_secs(5));
    let status = match tokio::time::timeout_at(decode_until, child.wait()).await {
        Ok(status) => status.unwrap(),
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            panic!("decoder exceeded original bounded wait; original child killed and joined phase={label}");
        }
    };
    assert!(status.success());
    assert!(tokio::time::Instant::now() < decode_until);
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
    assert!(
        mean[channel] > 200 && (0..3).filter(|i| *i != channel).all(|i| mean[i] < 40),
        "actual marker pixels: {mean:?}"
    );
}
pub(super) async fn originals(
    a: &mut Received,
    b: &mut Received,
    red: &[u8],
    blue: &[u8],
    dir: &Path,
    cutoff: tokio::time::Instant,
) {
    drain(a);
    drain(b);
    until_phase(
        Some(cutoff),
        "original-audio-after-private",
        opus::tone(&mut a.audio, 880.0, false),
    )
    .await;
    until_phase(
        Some(cutoff),
        "retained-audio-after-private",
        opus::tone(&mut b.audio, 440.0, false),
    )
    .await;
    picture(&mut a.video, blue, "bounded-original-blue", dir, 2, cutoff).await;
    picture(&mut b.video, red, "bounded-retained-red", dir, 0, cutoff).await;
}
