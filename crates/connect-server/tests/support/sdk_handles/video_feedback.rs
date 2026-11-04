//! Public signed SDK/server video feedback, actual fresh encoding and decoding.
use super::*;
use sdk::{Direction, Medium, StreamSelector};
use std::process::Stdio;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Encoder {
    child: tokio::process::Child,
    input: tokio::process::ChildStdin,
    output: tokio::process::ChildStdout,
}
impl Encoder {
    async fn open(directory: &Path) -> Self {
        assert_eq!(std::env::consts::OS, "linux");
        assert_eq!(std::env::consts::ARCH, "aarch64");
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let binary = directory.join("feedback-encoder");
        let built = tokio::process::Command::new("python3")
            .arg(root.join("scripts/video/build_live_vp8_encoder.py"))
            .arg("--output")
            .arg(&binary)
            .output()
            .await
            .unwrap();
        assert!(
            built.status.success(),
            "{}",
            String::from_utf8_lossy(&built.stderr)
        );
        eprintln!(
            "signed-feedback encoder build {}",
            String::from_utf8_lossy(&built.stdout)
        );
        let raw = directory.join("feedback-original.i420");
        let plane = 640 * 360;
        let mut bytes = Vec::with_capacity(plane * 3 / 2 * 8);
        for index in 0..8 {
            bytes.extend(std::iter::repeat_n(60 + index * 3, plane));
            bytes.extend(std::iter::repeat_n(128, plane / 2));
        }
        init::private_write(&raw, &bytes).unwrap();
        let mut child = tokio::process::Command::new(binary)
            .arg(raw)
            .arg("8")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        Self {
            input: child.stdin.take().unwrap(),
            output: child.stdout.take().unwrap(),
            child,
        }
    }
    async fn frame(&mut self, index: u32, force: bool) -> Vec<u8> {
        let mut request = index.to_le_bytes().to_vec();
        request.extend(u32::from(force).to_le_bytes());
        self.input.write_all(&request).await.unwrap();
        let mut header = [0u8; 16];
        self.output.read_exact(&mut header).await.unwrap();
        assert_eq!(&header[..4], b"V8F1");
        assert_eq!(u32::from_le_bytes(header[4..8].try_into().unwrap()), index);
        let size = u32::from_le_bytes(header[8..12].try_into().unwrap()) as usize;
        assert!((3..=256 * 1024).contains(&size));
        let mut bytes = vec![0; size];
        self.output.read_exact(&mut bytes).await.unwrap();
        assert_eq!(bytes[0] & 1 == 0, header[12] == 1);
        assert!(!force || bytes[0] & 1 == 0);
        bytes
    }
    async fn close(mut self) {
        self.input.shutdown().await.unwrap();
        drop(self.input);
        assert!(self.child.wait().await.unwrap().success());
    }
}
fn rtp(pt: u8, sequence: u16, timestamp: u32, ssrc: u32, marker: bool, payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0x80, pt | if marker { 0x80 } else { 0 }];
    bytes.extend_from_slice(&sequence.to_be_bytes());
    bytes.extend_from_slice(&timestamp.to_be_bytes());
    bytes.extend_from_slice(&ssrc.to_be_bytes());
    bytes.extend_from_slice(payload);
    bytes
}
fn configured(control: CanonicalClientConfig) -> sdk::ClientConfig {
    sdk::ClientConfig {
        control,
        tenant_id: "tnt_default".parse().unwrap(),
        native_video_feedback: true,
        application: None,
    }
}
fn single<T>(first: T) -> NonEmpty<T> {
    NonEmpty {
        first,
        rest: vec![],
    }
}
fn offered(id: &str, direction: Direction) -> NonEmpty<sdk::OfferedStream> {
    single(sdk::OfferedStream {
        id: id.parse().unwrap(),
        kind: Medium::Video,
        direction,
        codecs: single("vp8".into()),
    })
}
async fn publish(stream: &sdk::Stream, frame: &[u8], index: u32, sequence: &mut u16) {
    let parts = frame.chunks(1000).collect::<Vec<_>>();
    for (part, bytes) in parts.iter().enumerate() {
        let mut body = vec![if part == 0 { 0x10 } else { 0 }];
        body.extend_from_slice(bytes);
        let packet = sdk::RtpPacket::new(
            u32::from(*sequence),
            rtp(
                96,
                *sequence,
                index * 3000,
                0x74657374,
                part + 1 == parts.len(),
                &body,
            ),
        )
        .unwrap();
        stream.send_rtp(packet, options()).await.unwrap();
        *sequence = sequence.wrapping_add(1);
    }
}
async fn receive(stream: &sdk::Stream, index: u32) -> (Vec<u8>, sdk::VideoReceiveEpoch) {
    let mut bytes = Vec::new();
    let mut epoch = None;
    let mut sequence: Option<u16> = None;
    for _ in 0..256 {
        let packet = match stream.receive_packet(options()).await.unwrap() {
            sdk::MediaPacket::Rtp(packet) => packet,
            sdk::MediaPacket::Clock { .. } => continue,
            sdk::MediaPacket::Gap { .. } => panic!("unexpected local video queue loss"),
        };
        assert_eq!(packet.header().timestamp, index * 3000);
        let observed = packet
            .video_epoch()
            .expect("actual packet must join its signed server notice");
        if let Some(epoch) = &epoch {
            assert_eq!(epoch, observed);
        } else {
            epoch = Some(observed.clone());
        }
        if let Some(previous) = sequence {
            assert_eq!(packet.header().sequence_number, previous.wrapping_add(1));
        }
        sequence = Some(packet.header().sequence_number);
        let payload = packet.payload();
        assert!(payload.len() > 1);
        assert_eq!(payload[0] & 0x80, 0);
        assert_eq!(payload[0] & 0x10 != 0, bytes.is_empty());
        bytes.extend_from_slice(&payload[1..]);
        assert!(bytes.len() <= 256 * 1024);
        if packet.header().marker {
            return (bytes, epoch.unwrap());
        }
    }
    panic!("bounded complete VP8 frame missing")
}
async fn decode(directory: &Path, frame: &[u8], index: u32) {
    let mut header = Vec::from(&b"DKIF\0\0\x20\0VP80"[..]);
    header.extend(640u16.to_le_bytes());
    header.extend(360u16.to_le_bytes());
    for n in [90000u32, 1, 1, 0] {
        header.extend(n.to_le_bytes());
    }
    header.extend((frame.len() as u32).to_le_bytes());
    header.extend(u64::from(index * 3000).to_le_bytes());
    header.extend(frame);
    let path = directory.join(format!("signed-fresh-{index}.ivf"));
    init::private_write(&path, &header).unwrap();
    let output = tokio::process::Command::new("ffmpeg")
        .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(path)
        .args([
            "-vf",
            "crop=16:16:0:0",
            "-pix_fmt",
            "yuv420p",
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "pipe:1",
        ])
        .kill_on_drop(true)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout.len(), 384);
    let mean = output.stdout[..256]
        .iter()
        .map(|v| f64::from(*v))
        .sum::<f64>()
        / 256.;
    assert!(
        (mean - f64::from(60 + 3 * index)).abs() <= 3.,
        "fresh original marker"
    );
}

#[tokio::test]
async fn signed_sdk_video_feedback_encodes_fresh_frames_after_reselection_and_rechecks_hold() {
    tokio::time::timeout(Duration::from_secs(90), exercise())
        .await
        .unwrap();
}
async fn exercise() {
    let f = Fixture::with_native_end_scope("EdDSA", 12).await;
    super::super::canonical_native_connections::ready_endpoint_with_capabilities(
        &f,
        vec!["video".into()],
    )
    .await;
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope.operations.extend([
        "stream.subscribe".into(),
        "stream.unsubscribe".into(),
        "connection.end".into(),
    ]);
    let receiver_config = super::super::canonical_native_connections::party_with_scope(
        &f,
        &f.peer.identity_id,
        195,
        vec!["video".into()],
        scope,
    );
    let publisher = Client::connect_configured(configured(f.client.clone()))
        .await
        .unwrap();
    let subscriber = Client::connect_configured(configured(receiver_config))
        .await
        .unwrap();
    let room = publisher
        .create_conversation(
            vec![f.peer.identity_id.parse().unwrap()],
            ConversationPolicy::Persistent,
            mutation("feedback-room"),
        )
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap()
        .value;
    let recipient = room
        .refresh(options())
        .await
        .unwrap()
        .participants
        .into_iter()
        .find(|p| p.identity_id.as_str() == f.peer.identity_id)
        .unwrap()
        .id;
    let caps = json!({"video":["vp8"]});
    let session = room
        .invite(
            "native-session".parse().unwrap(),
            vec![recipient],
            sdk::SessionMedium::Video,
            "signed feedback".into(),
            caps.clone(),
            mutation("feedback-invite"),
        )
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap()
        .value;
    let target_session = subscriber
        .conversation_by_id(room.id().clone())
        .session_by_id(session.id().clone());
    target_session
        .accept(
            caps,
            sdk::TopologyRevision::try_from(0).unwrap(),
            mutation("feedback-accept"),
        )
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap();
    let source = session
        .offer_connection(
            "feedback-source".parse().unwrap(),
            offered("feedback-camera", Direction::Sendonly),
            mutation("feedback-source-offer"),
        )
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap()
        .value;
    let target = target_session
        .offer_connection(
            "feedback-target".parse().unwrap(),
            offered("feedback-monitor", Direction::Recvonly),
            mutation("feedback-target-offer"),
        )
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap()
        .value;
    assert_eq!(
        source
            .native_negotiation()
            .unwrap()
            .unwrap()
            .video_feedback
            .len(),
        1
    );
    assert_eq!(
        target
            .native_negotiation()
            .unwrap()
            .unwrap()
            .video_feedback
            .len(),
        1
    );
    let id: sdk::StreamId = "feedback-camera".parse().unwrap();
    let selected = target
        .subscribe(
            single(StreamSelector::Stream(id.clone())),
            sdk::StreamRevision::try_from(1).unwrap(),
            mutation("feedback-select"),
        )
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap()
        .value;
    let send = source.stream_by_id(id.clone());
    let recv = target.stream_by_id(id.clone());
    let mut encoder = Encoder::open(f.temp.path()).await;
    let mut sequence = 0u16;
    let first = encoder.frame(0, true).await;
    publish(&send, &first, 0, &mut sequence).await;
    let (received, previous_epoch) = receive(&recv, 0).await;
    assert_eq!(received, first);
    let unselected = target
        .unsubscribe(
            single(id.clone()),
            selected.stream_revision,
            mutation("feedback-unselect"),
        )
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap()
        .value;
    target
        .subscribe(
            single(StreamSelector::Stream(id.clone())),
            unselected.stream_revision,
            mutation("feedback-reselect"),
        )
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap();
    let delta = encoder.frame(1, false).await;
    assert_eq!(delta[0] & 1, 1);
    publish(&send, &delta, 1, &mut sequence).await;
    let (received, epoch) = receive(&recv, 1).await;
    assert_eq!(received, delta);
    assert_ne!(epoch.epoch, previous_epoch.epoch);
    assert_eq!(epoch.ssrc, previous_epoch.ssrc);
    recv.request_keyframe(options()).await.unwrap();
    let hint = send.receive_video_feedback(options()).await.unwrap();
    assert_eq!(hint.ssrc(), 0x74657374);
    assert!(send.video_feedback_is_current(&hint).unwrap());
    // Request the next original source frame only after checking this actual
    // server-delivered hint. The old keyframe never enters the fresh decoder.
    let fresh = encoder
        .frame(2, send.video_feedback_is_current(&hint).unwrap())
        .await;
    assert_eq!(fresh[0] & 1, 0);
    publish(&send, &fresh, 2, &mut sequence).await;
    let (received, fresh_epoch) = receive(&recv, 2).await;
    assert_eq!(received, fresh);
    assert_eq!(fresh_epoch, epoch);
    decode(f.temp.path(), &received, 2).await;
    let revision = source
        .native_negotiation()
        .unwrap()
        .unwrap()
        .stream_revision;
    source
        .update(
            sdk::ConnectionUpdate::Hold {
                streams: Some(single(id)),
                direction: sdk::HoldDirection::Inactive,
            },
            revision,
            mutation("feedback-hold"),
        )
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap();
    assert!(!send.video_feedback_is_current(&hint).unwrap());
    encoder.close().await;
    publisher.close(options()).await.unwrap();
    subscriber.close(options()).await.unwrap();
    f.server.shutdown().await.unwrap();
    eprintln!("signed SDK feedback: actual production listener/authority, signed selection/epoch join, same-SSRC reselection, current request reaches publisher, fresh frame2 independently decoded at RTP6000, held source rejects retained hint; not full transport matrix");
}
