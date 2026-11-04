//! Production replacement recovery; original frames and clocks, no media mocks.
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
        let mut bytes = Vec::with_capacity(plane * 3 / 2 * 12);
        for index in 0..12 {
            bytes.extend(std::iter::repeat_n(60 + index * 3, plane));
            bytes.extend(std::iter::repeat_n(128, plane / 2));
        }
        init::private_write(&raw, &bytes).unwrap();
        let mut child = tokio::process::Command::new(binary)
            .arg(raw)
            .arg("12")
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
    NonEmpty {
        first: sdk::OfferedStream {
            id: id.parse().unwrap(),
            kind: Medium::Video,
            direction,
            codecs: single("vp8".into()),
        },
        rest: vec![sdk::OfferedStream {
            id: format!("{id}-voice").parse().unwrap(),
            kind: Medium::Audio,
            direction: Direction::Sendrecv,
            codecs: single("g.711-mu".into()),
        }],
    }
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

use connect_client::canonical::{
    CanonicalClient, CanonicalHttpClient, CommandScope, NativeVideoDatagrams,
};
use connect_protocol::{
    replacement_video::{ReplacementVideo, ReplacementVideoStream},
    transitions::{TransitionCommand, TransitionIntent},
};

async fn sound(stream: &sdk::Stream) {
    for sequence in 1..=12u16 {
        stream
            .send_rtp(
                sdk::RtpPacket::new(
                    u32::from(sequence),
                    rtp(
                        0,
                        sequence,
                        u32::from(sequence) * 160,
                        0x71717171,
                        false,
                        &[0xff; 160],
                    ),
                )
                .unwrap(),
                options(),
            )
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
async fn raw_receive(
    io: &mut NativeVideoDatagrams,
    index: u32,
    handle: u16,
) -> (Vec<u8>, sdk::VideoReceiveEpoch) {
    tokio::time::timeout(Duration::from_secs(8), async {
        let mut assembler = connect_client::VideoFrameAssembler::default();
        let mut history = None;
        loop {
            let datagram = io.receive().await.unwrap();
            let Some(epoch) = datagram.video_epoch else {
                continue;
            };
            assert_eq!(
                u16::from_be_bytes(datagram.bytes[2..4].try_into().unwrap()),
                handle
            );
            let packet =
                connect_client::NativeVideoPacket::parse(datagram.bytes[24..].to_vec()).unwrap();
            assert_eq!(packet.timestamp_rtp, index * 3000);
            assert!(datagram.received_at.elapsed() < Duration::from_secs(1));
            if let Some(previous) = &history {
                assert_eq!(previous, &epoch)
            } else {
                history = Some(epoch)
            }
            if let Some(frame) = assembler.push(&packet).unwrap() {
                return (frame.encoded, history.unwrap());
            }
        }
    })
    .await
    .expect("replacement did not receive original signed video")
}
fn raw_publish(
    io: &NativeVideoDatagrams,
    handle: u16,
    frame: &[u8],
    index: u32,
    sequence: &mut u16,
) {
    use rvoip_uctp::substrate::{pack, MediaDatagram};
    let parts = frame.chunks(1000).collect::<Vec<_>>();
    for (part, bytes) in parts.iter().enumerate() {
        let mut body = vec![if part == 0 { 0x10 } else { 0 }];
        body.extend_from_slice(bytes);
        io.send(&pack(&MediaDatagram {
            flags: 0,
            stream_local_id: handle,
            seq: u32::from(*sequence),
            payload: rtp(
                96,
                *sequence,
                index * 3000,
                0x74657374,
                part + 1 == parts.len(),
                &body,
            )
            .into(),
        }))
        .unwrap();
        *sequence = sequence.wrapping_add(1);
    }
}
async fn phase(
    f: &Fixture,
    controller: &mut CanonicalHttpClient,
    command: &TransitionCommand,
    needed: usize,
) -> connect_client::canonical::payloads::OperationUpdated {
    tokio::time::timeout(Duration::from_secs(10),async {
        loop {
            let root=controller.operation_get(&command.operation_id,CommandScope{conversation_id:Some(command.conversation_id.clone()),session_id:Some(command.session_id.clone()),connection_id:command.connection_id.clone(),..Default::default()}).await.unwrap();
            let state=serde_json::to_value(&root).unwrap();
            assert!(!matches!(state["state"].as_str(),Some("failed"|"cancelled"|"unknown")),"{state}");
            let db=rusqlite::Connection::open(&f.config.database).unwrap();
            let confirmed:usize=db.query_row("SELECT count(*) FROM operation_effects WHERE operation_record_id=?1 AND state='confirmed'",[root.record_id.as_ref().unwrap().as_str()],|r|r.get(0)).unwrap();
            if confirmed>=needed {return root}
            drop(db);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.expect("original replacement phase did not settle")
}
#[tokio::test]
async fn signed_production_replacement_transfers_video_and_recovers_fresh_frames_in_both_directions(
) {
    tokio::time::timeout(Duration::from_secs(90), exercise(false))
        .await
        .unwrap();
}
#[tokio::test]
async fn signed_production_replacement_hold_resume_recovers_both_video_directions() {
    tokio::time::timeout(Duration::from_secs(90), exercise(true))
        .await
        .unwrap();
}
async fn gate(
    owner: &mut CanonicalClient,
    scope: &CommandScope,
    operation: &str,
    revision: u64,
    action: &str,
) -> u64 {
    let mut scope = scope.clone();
    scope.operation_id = Some(operation.into());
    scope.expected_revision = Some(revision);
    let mut payload =
        json!({"action":action,"streams":["replacement-camera","replacement-incoming-camera"]});
    if action == "hold" {
        payload["details"] = json!({"direction":"inactive"});
    }
    let request = owner
        .send_value("connection.update", payload, scope)
        .await
        .unwrap();
    let mut result = None;
    for _ in 0..32 {
        let frame = owner.receive().await.unwrap();
        if frame.scope(rvoip_uctp::v2::Scope::Reply) != Some(request.as_str()) {
            continue;
        }
        if frame.kind() == "operation.updated" {
            let p = &frame.original()["payload"];
            assert_eq!(p["state"], "completed", "{p}");
            assert_eq!(p["result"]["connection_generation"], 2);
            result = Some(p["result"]["stream_revision"].as_u64().unwrap());
        } else if frame.kind() == "ack" {
            return result.unwrap();
        } else {
            panic!("unexpected gate result {:?}", frame.original());
        }
    }
    panic!("bounded gate result missing")
}
async fn exercise(gates: bool) {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("connect_client::canonical::native_video=debug,connect_service::canonical_flow_ready=debug")
        .with_test_writer()
        .try_init();
    let f = Fixture::with_native_end_scope("EdDSA", 12).await;
    super::super::canonical_native_connections::ready_endpoint_with_capabilities(
        &f,
        vec!["audio".into(), "video".into()],
    )
    .await;
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope.operations.extend([
        "stream.subscribe".into(),
        "stream.unsubscribe".into(),
        "connection.end".into(),
        "connection.update".into(),
    ]);
    let receiver_config = super::super::canonical_native_connections::party_with_scope(
        &f,
        &f.peer.identity_id,
        295,
        vec!["audio".into(), "video".into()],
        scope.clone(),
    );
    let replacement_config = super::super::canonical_native_connections::party_with_scope(
        &f,
        &f.client.identity_id,
        296,
        vec!["audio".into(), "video".into()],
        scope,
    );
    let endpoint: String = rusqlite::Connection::open(&f.config.database)
        .unwrap()
        .query_row(
            "SELECT id FROM endpoints WHERE device_id=?1",
            [&replacement_config.device_id],
            |r| r.get(0),
        )
        .unwrap();
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
            mutation("replacement-feedback-room"),
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
    let caps = json!({"audio":["g.711-mu"],"video":["vp8"]});
    let session = room
        .invite(
            "native-session".parse().unwrap(),
            vec![recipient],
            sdk::SessionMedium::VoiceVideo,
            "replacement feedback".into(),
            caps.clone(),
            mutation("replacement-feedback-invite"),
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
            mutation("replacement-feedback-accept"),
        )
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap();
    eprintln!("production replacement phase: initial-media-offers");
    let source = session
        .offer_connection(
            "feedback-source".parse().unwrap(),
            offered("feedback-camera", Direction::Sendrecv),
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
            offered("feedback-monitor", Direction::Sendrecv),
            mutation("feedback-target-offer"),
        )
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap()
        .value;
    for (connection, remote, operation) in [
        (&source, "feedback-monitor", "feedback-source-select"),
        (&target, "feedback-camera", "feedback-target-select"),
    ] {
        connection
            .subscribe(
                NonEmpty {
                    first: StreamSelector::Stream(remote.parse().unwrap()),
                    rest: vec![StreamSelector::Stream(
                        format!("{remote}-voice").parse().unwrap(),
                    )],
                },
                sdk::StreamRevision::try_from(1).unwrap(),
                mutation(operation),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap();
    }
    let source_audio = source.stream_by_id("feedback-camera-voice".parse().unwrap());
    let target_audio = target.stream_by_id("feedback-monitor-voice".parse().unwrap());
    tokio::join!(sound(&source_audio), sound(&target_audio));
    let old_send = source.stream_by_id("feedback-camera".parse().unwrap());
    let retained_recv = target.stream_by_id("feedback-camera".parse().unwrap());
    let retained_send = target.stream_by_id("feedback-monitor".parse().unwrap());
    let old_recv = source.stream_by_id("feedback-monitor".parse().unwrap());
    eprintln!("production replacement phase: initial-codec-history");
    let mut encoder = Encoder::open(f.temp.path()).await;
    let first = encoder.frame(0, true).await;
    let mut original_sequence = 0;
    let mut retained_sequence = 0;
    publish(&old_send, &first, 0, &mut original_sequence).await;
    let (bytes, previous_retained_epoch) = receive(&retained_recv, 0).await;
    assert_eq!(bytes, first);
    publish(&retained_send, &first, 0, &mut retained_sequence).await;
    let (bytes, previous_target_epoch) = receive(&old_recv, 0).await;
    assert_eq!(bytes, first);
    let mut controller_scope = Scope::collaboration(vec!["*".into()]);
    controller_scope
        .operations
        .push("connection.replace".into());
    let key = f
        .server
        .store
        .issue(
            &f.client.identity_id,
            controller_scope,
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let mut config = canonical_http_config(&f);
    config.credential = key.token;
    let mut controller = CanonicalHttpClient::open(config).await.unwrap();
    let revision = rusqlite::Connection::open(&f.config.database)
        .unwrap()
        .query_row(
            "SELECT topology_revision FROM sessions WHERE id='native-session'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let command = TransitionCommand {
        operation_id: "production-video-replacement".into(),
        conversation_id: room.id().as_str().into(),
        session_id: "native-session".into(),
        connection_id: Some("feedback-source".into()),
        expected_revision: revision,
        intent: TransitionIntent::Replace {
            endpoint_id: Some(endpoint),
            candidate_connid: None,
            retained_connid: "feedback-target".into(),
            video: Some(ReplacementVideo {
                publications: vec![ReplacementVideoStream {
                    previous: "feedback-camera".into(),
                    replacement: "replacement-camera".into(),
                }],
                receivers: vec![ReplacementVideoStream {
                    previous: "feedback-monitor".into(),
                    replacement: "replacement-incoming-camera".into(),
                }],
            }),
        },
        payload_extensions: Default::default(),
    };
    eprintln!("production replacement phase: replacement-prepare");
    let root = controller.transition(command.clone()).await.unwrap();
    phase(&f, &mut controller, &command, 1).await;
    eprintln!("production replacement phase: replacement-allocate");
    let mut replacement = CanonicalClient::open(replacement_config).await.unwrap();
    let offer = json!({"substrate":"quic","capabilities":{"audio_codecs":[{"name":"pcmu","params":{"sample_rate":8000,"channels":1}}],"video_codecs":[{"name":"vp8","params":{"sample_rate":90000,"channels":0}}],"max_streams_per_connection":8,"video_feedback":"rtcp-pli-epoch/1"},"streams_offered":[{"id":"replacement-voice","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]},{"id":"replacement-camera","kind":"video","direction":"sendonly","codec_preferences":["vp8"]},{"id":"replacement-incoming-camera","kind":"video","direction":"recvonly","codec_preferences":["vp8"]}]});
    replacement
        .allocate_native_replacement_target(
            &command,
            &root,
            serde_json::from_value(offer.clone()).unwrap(),
        )
        .await
        .unwrap();
    eprintln!("production replacement phase: replacement-native-negotiation");
    let (_, scope) = replacement
        .begin_native_replacement_media(
            &command,
            &root,
            serde_json::from_value(offer.clone()).unwrap(),
        )
        .await
        .unwrap();
    let answer = replacement.receive().await.unwrap();
    assert_eq!(answer.kind(), "connection.answer");
    let callback = replacement.receive().await.unwrap();
    assert_eq!(callback.kind(), "connection.offer");
    let p = &callback.original()["payload"];
    let bindings = p["substrate_setup"]["streams"].as_array().unwrap();
    for binding in bindings.iter().filter(|b| b["kind"] == "video") {
        assert_eq!(binding["video_feedback"]["connection_generation"], 2)
    }
    let handle = |id: &str| {
        bindings.iter().find(|b| b["strm_id"] == id).unwrap()["stream_local_id"]
            .as_u64()
            .unwrap() as u16
    };
    let (voice, camera, incoming) = (
        handle("replacement-voice"),
        handle("replacement-camera"),
        handle("replacement-incoming-camera"),
    );
    let streams=bindings.iter().map(|b|json!({"id":b["strm_id"],"kind":b["kind"],"direction":match b["direction"].as_str().unwrap(){"sendonly"=>"recvonly","recvonly"=>"sendonly",_=>"sendrecv"},"codec":b["codec"]})).collect::<Vec<_>>();
    replacement.answer_connection_offer(&callback,serde_json::from_value(json!({"by_participant":p["by_participant"],"substrate":"quic","capabilities":offer["capabilities"],"streams_answered":streams,"substrate_setup":p["substrate_setup"]})).unwrap(),&scope).await.unwrap();
    eprintln!("production replacement phase: replacement-resource-capture");
    phase(&f, &mut controller, &command, 5).await;
    let mut io = replacement.claim_native_video_datagrams().unwrap();
    for sequence in 1..=12u16 {
        use rvoip_uctp::substrate::{pack, MediaDatagram};
        io.send(&pack(&MediaDatagram {
            flags: 0,
            stream_local_id: voice,
            seq: u32::from(sequence),
            payload: rtp(
                0,
                sequence,
                u32::from(sequence) * 160,
                0x72727272,
                false,
                &[0xff; 160],
            )
            .into(),
        }))
        .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    eprintln!("production replacement phase: replacement-commit");
    let done = phase(&f, &mut controller, &command, 7).await;
    let done = serde_json::to_value(done).unwrap();
    assert_eq!(done["state"], "completed");
    assert_eq!(done["result"]["generation"], 2);
    // New replacement receiver starts with an interframe from the unchanged
    // retained source. Only actual signed history can solicit source recovery.
    eprintln!("production replacement phase: replacement-receiver-recovery");
    let delta = encoder.frame(1, false).await;
    assert_eq!(delta[0] & 1, 1);
    publish(&retained_send, &delta, 1, &mut retained_sequence).await;
    let (bytes, epoch) = raw_receive(&mut io, 1, incoming).await;
    assert_eq!(bytes, delta);
    assert_ne!(epoch.epoch, previous_target_epoch.epoch);
    let hint = retained_send
        .receive_video_feedback(options())
        .await
        .unwrap();
    assert!(retained_send.video_feedback_is_current(&hint).unwrap());
    let fresh = encoder
        .frame(2, retained_send.video_feedback_is_current(&hint).unwrap())
        .await;
    publish(&retained_send, &fresh, 2, &mut retained_sequence).await;
    let (bytes, same) = raw_receive(&mut io, 2, incoming).await;
    assert_eq!(same, epoch);
    assert_eq!(bytes, fresh);
    decode(f.temp.path(), &bytes, 2).await;
    let before_gate_receive = same.clone();
    // In the reverse direction the retained receiver requests a keyframe from
    // the raw generation-two publisher. Pumping receives the physical feedback;
    // the boolean is consumed once immediately before the next fresh encoding.
    eprintln!("production replacement phase: replacement-publisher-recovery");
    let delta = encoder.frame(3, false).await;
    assert_eq!(delta[0] & 1, 1);
    let mut sequence = 0;
    raw_publish(&io, camera, &delta, 3, &mut sequence);
    let (bytes, epoch) = receive(&retained_recv, 3).await;
    assert_eq!(bytes, delta);
    assert_ne!(epoch.epoch, previous_retained_epoch.epoch);
    retained_recv.request_keyframe(options()).await.unwrap();
    // A data-free receive may remain pending after consuming feedback. Its
    // cancellation must retain the hint's original arrival and one-use state.
    let _ = tokio::time::timeout(Duration::from_millis(50), io.receive()).await;
    let requested = io.take_keyframe_request(camera, 0x74657374).unwrap();
    assert!(requested);
    let fresh = encoder.frame(4, requested).await;
    assert!(!io.take_keyframe_request(camera, 0x74657374).unwrap());
    raw_publish(&io, camera, &fresh, 4, &mut sequence);
    let (bytes, same) = receive(&retained_recv, 4).await;
    assert_eq!(same, epoch);
    assert_eq!(bytes, fresh);
    decode(f.temp.path(), &bytes, 4).await;
    if gates {
        eprintln!("production replacement phase: signed-hold-resume");
        let revision: u64 = rusqlite::Connection::open(&f.config.database)
            .unwrap()
            .query_row(
                "SELECT stream_revision FROM connections WHERE id=?1",
                [scope.connection_id.as_ref().unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        let held = gate(
            &mut replacement,
            &scope,
            "replacement-video-hold",
            revision,
            "hold",
        )
        .await;
        assert_eq!(held, revision + 1);
        assert!(!io.take_keyframe_request(camera, 0x74657374).unwrap());
        let resumed = gate(
            &mut replacement,
            &scope,
            "replacement-video-resume",
            held,
            "resume",
        )
        .await;
        assert_eq!(resumed, held + 1);
        let delta = encoder.frame(5, false).await;
        assert_eq!(delta[0] & 1, 1);
        publish(&retained_send, &delta, 5, &mut retained_sequence).await;
        let (bytes, resumed_receive) = raw_receive(&mut io, 5, incoming).await;
        assert_eq!(bytes, delta);
        assert_ne!(resumed_receive.epoch, before_gate_receive.epoch);
        let hint = retained_send
            .receive_video_feedback(options())
            .await
            .unwrap();
        assert!(retained_send.video_feedback_is_current(&hint).unwrap());
        let fresh = encoder.frame(6, true).await;
        publish(&retained_send, &fresh, 6, &mut retained_sequence).await;
        let (bytes, current) = raw_receive(&mut io, 6, incoming).await;
        assert_eq!(current, resumed_receive);
        assert_eq!(bytes, fresh);
        decode(f.temp.path(), &bytes, 6).await;
        let delta = encoder.frame(7, false).await;
        assert_eq!(delta[0] & 1, 1);
        raw_publish(&io, camera, &delta, 7, &mut sequence);
        let (bytes, resumed_publish) = receive(&retained_recv, 7).await;
        assert_eq!(bytes, delta);
        assert_ne!(resumed_publish.epoch, epoch.epoch);
        retained_recv.request_keyframe(options()).await.unwrap();
        let _ = tokio::time::timeout(Duration::from_millis(50), io.receive()).await;
        let requested = io.take_keyframe_request(camera, 0x74657374).unwrap();
        assert!(requested);
        let fresh = encoder.frame(8, requested).await;
        assert!(!io.take_keyframe_request(camera, 0x74657374).unwrap());
        raw_publish(&io, camera, &fresh, 8, &mut sequence);
        let (bytes, current) = receive(&retained_recv, 8).await;
        assert_eq!(current, resumed_publish);
        assert_eq!(bytes, fresh);
        decode(f.temp.path(), &bytes, 8).await;
        eprintln!("production replacement gate proof: signed generation2 hold/resume, fresh epoch and independent VP8 decode in both directions at original RTP18000 and24000");
    }
    encoder.close().await;
    replacement.shutdown().await.unwrap();
    drop(io);
    publisher.close(options()).await.unwrap();
    subscriber.close(options()).await.unwrap();
    f.server.shutdown().await.unwrap();
    eprintln!("production replacement: signed generation2, transferred both original video selections, real receiver feedback and fresh libvpx keys independently decoded at original RTP6000 and12000; one-use raw publisher hint; owned peers and encoder joined; no full transport cell qualified");
}
