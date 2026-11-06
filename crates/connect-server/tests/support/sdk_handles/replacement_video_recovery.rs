//! Production replacement recovery; original frames and clocks, no media mocks.
use super::*;
use sdk::{Direction, Medium, StreamSelector};
use std::process::Stdio;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[path = "replacement_video_recovery/readiness.rs"]
mod readiness;
#[path = "../canonical_native_connections/consultation/groups/replacement/decoded_readiness/audio.rs"]
mod readiness_audio;

struct Encoder {
    executable: std::path::PathBuf,
    executable_sha256: String,
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
        let executable_sha256 = readiness::file_sha256(&binary);
        let mut child = tokio::process::Command::new(&binary)
            .arg(raw)
            .arg("12")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        Self {
            executable: binary,
            executable_sha256,
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
fn offered(id: &str, direction: Direction, audio_codec: &str) -> NonEmpty<sdk::OfferedStream> {
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
            codecs: single(audio_codec.into()),
        }],
    }
}
async fn publish(stream: &sdk::Stream, frame: &[u8], index: u32, sequence: &mut u16) {
    publish_timestamp(stream, frame, index * 3000, sequence).await;
}
async fn publish_timestamp(stream: &sdk::Stream, frame: &[u8], timestamp: u32, sequence: &mut u16) {
    let parts = frame.chunks(1000).collect::<Vec<_>>();
    for (part, bytes) in parts.iter().enumerate() {
        let mut body = vec![if part == 0 { 0x10 } else { 0 }];
        body.extend_from_slice(bytes);
        let packet = sdk::RtpPacket::new(
            u32::from(*sequence),
            rtp(
                96,
                *sequence,
                timestamp,
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
    receive_timestamp(stream, index * 3000).await
}
async fn receive_timestamp(
    stream: &sdk::Stream,
    timestamp: u32,
) -> (Vec<u8>, sdk::VideoReceiveEpoch) {
    let mut bytes = Vec::new();
    let mut epoch = None;
    let mut sequence: Option<u16> = None;
    for _ in 0..256 {
        let packet = match stream.receive_packet(options()).await.unwrap() {
            sdk::MediaPacket::Rtp(packet) => packet,
            sdk::MediaPacket::Clock { .. } => continue,
            sdk::MediaPacket::Gap { .. } => panic!("unexpected local video queue loss"),
        };
        assert_eq!(packet.header().timestamp, timestamp);
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

async fn sound(stream: &sdk::Stream, opus: bool) {
    let selected = if opus {
        json!({"name":"opus","params":{"sample_rate":48000,"channels":1,"payload_type":111}})
    } else {
        json!({"name":"pcmu","params":{"sample_rate":8000,"channels":1,"payload_type":0}})
    };
    let decoder = readiness_audio::Decoder::selected(&selected).unwrap();
    let payload = decoder.encoded_silence().unwrap();
    let pt = if opus { 111 } else { 0 };
    let quantum = decoder.rate() / 50;
    for sequence in 1..=12u16 {
        stream
            .send_rtp(
                sdk::RtpPacket::new(
                    u32::from(sequence),
                    rtp(
                        pt,
                        sequence,
                        u32::from(sequence) * quantum,
                        0x71717171,
                        false,
                        &payload,
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
    phase_state(f, controller, command, needed, false).await
}
async fn completed(
    f: &Fixture,
    controller: &mut CanonicalHttpClient,
    command: &TransitionCommand,
) -> connect_client::canonical::payloads::OperationUpdated {
    phase_state(f, controller, command, 7, true).await
}
// Confirmed effects can precede the original journal/public projection. The
// final decoder continuity phase needs both, under the same original ten-second
// phase wait and the caller's unchanged original replacement cutoff.
async fn phase_state(
    f: &Fixture,
    controller: &mut CanonicalHttpClient,
    command: &TransitionCommand,
    needed: usize,
    require_completed: bool,
) -> connect_client::canonical::payloads::OperationUpdated {
    tokio::time::timeout(Duration::from_secs(10),async {
        loop {
            let root=controller.operation_get(&command.operation_id,CommandScope{conversation_id:Some(command.conversation_id.clone()),session_id:Some(command.session_id.clone()),connection_id:command.connection_id.clone(),..Default::default()}).await.unwrap();
            let state=serde_json::to_value(&root).unwrap();
            assert!(!matches!(state["state"].as_str(),Some("failed"|"cancelled"|"unknown")),"{state}");
            let db=rusqlite::Connection::open(&f.config.database).unwrap();
            let confirmed:usize=db.query_row("SELECT count(*) FROM operation_effects WHERE operation_record_id=?1 AND state='confirmed'",[root.record_id.as_ref().unwrap().as_str()],|r|r.get(0)).unwrap();
            if confirmed>=needed && (!require_completed || state["state"]=="completed") {return root}
            drop(db);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.expect("original replacement phase did not settle")
}
#[tokio::test]
async fn signed_production_replacement_transfers_video_and_recovers_fresh_frames_in_both_directions(
) {
    tokio::time::timeout(Duration::from_secs(90), exercise(0))
        .await
        .unwrap();
}
#[tokio::test]
async fn signed_production_replacement_hold_resume_recovers_both_video_directions() {
    tokio::time::timeout(Duration::from_secs(90), exercise(1))
        .await
        .unwrap();
}
#[tokio::test]
async fn signed_production_replacement_codec_change_preserves_video_and_adopts_new_audio() {
    tokio::time::timeout(Duration::from_secs(90), exercise(2))
        .await
        .unwrap();
}
#[cfg(feature = "test-support")]
#[tokio::test]
async fn signed_replacement_commit_preserves_current_owner_under_registry_contention() {
    tokio::time::timeout(Duration::from_secs(90), exercise(3))
        .await
        .unwrap();
}
#[cfg(feature = "test-support")]
#[tokio::test]
async fn signed_replacement_preparation_preserves_current_authority_under_registry_contention() {
    tokio::time::timeout(Duration::from_secs(90), exercise(4))
        .await
        .unwrap();
}
#[cfg(feature = "test-support")]
#[tokio::test]
async fn signed_native_control_preserves_subscription_recovery_and_replacement_gates() {
    // A new combined production-path case for the control execution bridge.
    // Scheduler exclusion, cancellation and poll custody have separate probes.
    tokio::time::timeout(Duration::from_secs(90), exercise(5))
        .await
        .unwrap();
}
#[cfg(feature = "test-support")]
#[tokio::test]
async fn signed_replacement_initial_mute_retains_current_owners_under_registry_contention() {
    tokio::time::timeout(Duration::from_secs(90), exercise(6))
        .await
        .unwrap();
}
#[tokio::test]
async fn signed_replacement_original_registration_outlives_its_preparation_cutoff() {
    // Same cap as every actual signed replacement case; do not renew either
    // the preparation, original operation or physical native connection.
    tokio::time::timeout(Duration::from_secs(90), exercise(7))
        .await
        .unwrap();
}

#[tokio::test]
async fn signed_three_participant_replacement_reads_nonretained_source_in_current_scene() {
    tokio::time::timeout(Duration::from_secs(90), exercise(8))
        .await
        .unwrap();
}
#[tokio::test]
async fn signed_opus_replacement_decodes_private_audio_and_preserves_video_epoch() {
    tokio::time::timeout(Duration::from_secs(90), exercise(9))
        .await
        .unwrap();
}
#[tokio::test]
async fn signed_data_session_upgraded_to_av_opus_replacement_decodes_private_audio_and_video() {
    tokio::time::timeout(Duration::from_secs(90), exercise(10))
        .await
        .unwrap();
}
#[tokio::test]
async fn signed_private_delta_bootstrap_requires_actual_current_hint_before_ready() {
    tokio::time::timeout(Duration::from_secs(90), exercise(11))
        .await
        .unwrap();
}
#[tokio::test]
async fn signed_private_delta_bootstrap_recovers_lost_hint_without_another_video_frame() {
    tokio::time::timeout(Duration::from_secs(90), exercise(12))
        .await
        .unwrap();
}
#[cfg(feature = "test-support")]
#[tokio::test]
async fn signed_opus_replacement_video_plan_contention_recovers_same_original_operation() {
    tokio::time::timeout(Duration::from_secs(90), exercise(17))
        .await
        .unwrap();
}
#[cfg(feature = "test-support")]
#[tokio::test]
async fn signed_private_readiness_outlives_start_snapshot_without_renewing_original_reservation() {
    tokio::time::timeout(Duration::from_secs(90), exercise(13))
        .await
        .unwrap();
}
#[cfg(feature = "test-support")]
#[tokio::test]
async fn signed_private_readiness_cannot_outlive_its_exact_original_reservation_expiry() {
    tokio::time::timeout(Duration::from_secs(90), exercise(14))
        .await
        .unwrap();
}
#[cfg(feature = "test-support")]
#[tokio::test]
async fn signed_private_readiness_rechecks_current_original_operator_revocation() {
    tokio::time::timeout(Duration::from_secs(90), exercise(15))
        .await
        .unwrap();
}
#[cfg(feature = "test-support")]
#[tokio::test]
async fn signed_private_readiness_cannot_survive_original_physical_target_cancellation() {
    tokio::time::timeout(Duration::from_secs(90), exercise(16))
        .await
        .unwrap();
}
async fn codec_change(
    owner: &mut CanonicalClient,
    scope: &CommandScope,
    caps: &Value,
    expected: u64,
) -> Value {
    let mut scope = scope.clone();
    scope.operation_id = Some("replacement-codec-change".into());
    scope.expected_revision = Some(expected);
    let request = owner
        .send_value(
            "connection.update",
            json!({"action":"codec-renegotiate","codec_preferences":["g.711-a","vp8"]}),
            scope.clone(),
        )
        .await
        .unwrap();
    let mut result = None;
    for _ in 0..32 {
        let frame = owner.receive().await.unwrap();
        if frame.kind() == "connection.offer" {
            let p = &frame.original()["payload"];
            let streams=p["substrate_setup"]["streams"].as_array().unwrap().iter().map(|b|json!({"id":b["strm_id"],"kind":b["kind"],"direction":match b["direction"].as_str().unwrap(){"sendonly"=>"recvonly","recvonly"=>"sendonly",_=>"sendrecv"},"codec":b["codec"]})).collect::<Vec<_>>();
            owner.answer_connection_offer(&frame,serde_json::from_value(json!({"by_participant":p["by_participant"],"substrate":"quic","capabilities":caps,"streams_answered":streams,"substrate_setup":p["substrate_setup"]})).unwrap(),&scope).await.unwrap();
        } else if frame.scope(rvoip_uctp::v2::Scope::Reply) == Some(request.as_str()) {
            if frame.kind() == "operation.updated" {
                let p = &frame.original()["payload"];
                assert_eq!(p["state"], "completed", "{p}");
                result = Some(p["result"].clone());
            } else if frame.kind() == "ack" {
                return result.unwrap();
            } else {
                panic!("unexpected codec result {:?}", frame.original());
            }
        }
    }
    panic!("bounded codec result missing")
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
async fn exercise(mode: u8) {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("connect_client::canonical::native_video=debug,connect_service::canonical_flow_ready=debug,connect_service::canonical_replacement=debug")
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
        scope.clone(),
    );
    let endpoint: String = rusqlite::Connection::open(&f.config.database)
        .unwrap()
        .query_row(
            "SELECT id FROM endpoints WHERE device_id=?1",
            [&replacement_config.device_id],
            |r| r.get(0),
        )
        .unwrap();
    let third_identity = if mode == 8 {
        let admin = f.server.store.authenticate(&f.admin).unwrap().credential_id;
        let ProvisioningOutcome::Identity { identity_id, .. } = f
            .server
            .store
            .execute_provisioning(
                &admin,
                ProvisioningCommand::Identity {
                    operation_id: "replacement-third-identity".into(),
                    alias: "replacement-third".into(),
                    kind: IdentityKind::Human,
                    display_name: None,
                },
            )
            .unwrap()
        else {
            panic!("third identity")
        };
        Some(identity_id)
    } else {
        None
    };
    let third_client = if let Some(identity) = &third_identity {
        let config = super::super::canonical_native_connections::party_with_scope(
            &f,
            identity,
            297,
            vec!["audio".into(), "video".into()],
            scope.clone(),
        );
        Some(
            Client::connect_configured(configured(config))
                .await
                .unwrap(),
        )
    } else {
        None
    };
    let mut invitees = vec![f.peer.identity_id.parse().unwrap()];
    if let Some(identity) = &third_identity {
        invitees.push(identity.parse().unwrap());
    }
    let publisher = Client::connect_configured(configured(f.client.clone()))
        .await
        .unwrap();
    let subscriber = Client::connect_configured(configured(receiver_config))
        .await
        .unwrap();
    let room = publisher
        .create_conversation(
            invitees,
            ConversationPolicy::Persistent,
            mutation("replacement-feedback-room"),
        )
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap()
        .value;
    let participants = room.refresh(options()).await.unwrap().participants;
    let recipient = participants
        .iter()
        .find(|peer| peer.identity_id.as_str() == f.peer.identity_id)
        .unwrap()
        .id
        .clone();
    let mut recipients = vec![recipient];
    if let Some(identity) = &third_identity {
        recipients.push(
            participants
                .iter()
                .find(|peer| peer.identity_id.as_str() == identity.as_str())
                .unwrap()
                .id
                .clone(),
        );
    }
    let mut caps = json!({"audio":["g.711-mu"],"video":["vp8"]});
    if mode >= 9 {
        caps["audio"] = json!(["opus"]);
    }
    if mode == 10 {
        caps["data"] = json!(["reliable-ordered"]);
    }
    if mode == 2 {
        caps["audio"] = json!(["g.711-mu", "g.711-a"]);
    }
    let session = room
        .invite(
            "native-session".parse().unwrap(),
            recipients,
            if mode == 10 {
                sdk::SessionMedium::TextChat
            } else {
                sdk::SessionMedium::VoiceVideo
            },
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
            caps.clone(),
            sdk::TopologyRevision::try_from(0).unwrap(),
            mutation("replacement-feedback-accept"),
        )
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap();
    let third_session = if let Some(client) = &third_client {
        let session = client
            .conversation_by_id(room.id().clone())
            .session_by_id(session.id().clone());
        let topology = session.refresh(options()).await.unwrap().topology_revision;
        session
            .accept(caps.clone(), topology, mutation("replacement-third-accept"))
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap();
        Some(session)
    } else {
        None
    };
    if mode == 10 {
        let before = session.refresh(options()).await.unwrap();
        assert_eq!(before.media, vec![Medium::Data]);
        session
            .update(
                sdk::SessionUpdate::MediumUpgrade(NonEmpty {
                    first: Medium::Audio,
                    rest: vec![Medium::Video],
                }),
                before.topology_revision,
                mutation("replacement-data-av-upgrade"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap();
        let after = session.refresh(options()).await.unwrap();
        assert_eq!(after.id, before.id);
        assert!(after.topology_revision.get() > before.topology_revision.get());
        assert_eq!(
            after.media,
            vec![Medium::Audio, Medium::Data, Medium::Video]
        );
        eprintln!("production replacement phase: original-data-session-additive-av-upgrade");
    }
    eprintln!("production replacement phase: initial-media-offers");
    let source = session
        .offer_connection(
            "feedback-source".parse().unwrap(),
            offered(
                "feedback-camera",
                Direction::Sendrecv,
                if mode >= 9 { "opus" } else { "g.711-mu" },
            ),
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
            offered(
                "feedback-monitor",
                Direction::Sendrecv,
                if mode >= 9 { "opus" } else { "g.711-mu" },
            ),
            mutation("feedback-target-offer"),
        )
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap()
        .value;
    let third_connection = if let Some(session) = &third_session {
        Some(
            session
                .offer_connection(
                    "feedback-third".parse().unwrap(),
                    offered("feedback-third-camera", Direction::Sendrecv, "g.711-mu"),
                    mutation("feedback-third-offer"),
                )
                .await
                .unwrap()
                .wait(options())
                .await
                .unwrap()
                .value,
        )
    } else {
        None
    };
    let mut selections = vec![
        (
            &source,
            if mode == 8 {
                "feedback-third-camera"
            } else {
                "feedback-monitor"
            },
            "feedback-source-select",
        ),
        (&target, "feedback-camera", "feedback-target-select"),
    ];
    if let Some(third) = &third_connection {
        selections.push((third, "feedback-camera", "feedback-third-select"));
    }
    for (connection, remote, operation) in selections {
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
    if mode == 5 {
        let revision = target
            .refresh(options())
            .await
            .unwrap()
            .stream_revision
            .unwrap();
        let removed = target
            .unsubscribe(
                single("feedback-camera".parse().unwrap()),
                revision,
                mutation("executor-unselect"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        assert_eq!(removed.stream_revision.get(), revision.get() + 1);
        let selected = || NonEmpty {
            first: StreamSelector::Stream("feedback-camera".parse().unwrap()),
            rest: vec![StreamSelector::Stream(
                "feedback-camera-voice".parse().unwrap(),
            )],
        };
        let restored = target
            .subscribe(
                selected(),
                removed.stream_revision,
                mutation("executor-reselect"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        assert_eq!(restored.stream_revision.get(), revision.get() + 2);
        // Same original operation and expected revision: recover its result,
        // never silently construct another logical mutation after uncertainty.
        let recovered = target
            .subscribe(
                selected(),
                removed.stream_revision,
                mutation("executor-reselect"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        assert_eq!(recovered.stream_revision, restored.stream_revision);
        let actual = target
            .refresh(options())
            .await
            .unwrap()
            .stream_revision
            .unwrap();
        assert_eq!(actual, restored.stream_revision);
        eprintln!(
            "native executor signed controls: unsubscribe/resubscribe each advance once; same-operation recovery preserves the original revision"
        );
    }
    let source_audio = source.stream_by_id("feedback-camera-voice".parse().unwrap());
    let target_audio = target.stream_by_id("feedback-monitor-voice".parse().unwrap());
    tokio::join!(
        sound(&source_audio, mode >= 9),
        sound(&target_audio, mode >= 9),
        async {
            if let Some(third) = &third_connection {
                sound(
                    &third.stream_by_id("feedback-third-camera-voice".parse().unwrap()),
                    false,
                )
                .await;
            }
        }
    );
    let old_send = source.stream_by_id("feedback-camera".parse().unwrap());
    let retained_recv = target.stream_by_id("feedback-camera".parse().unwrap());
    // In the three-party control, receive Carol even though Bob is retained.
    let retained_send = third_connection
        .as_ref()
        .map(|third| third.stream_by_id("feedback-third-camera".parse().unwrap()))
        .unwrap_or_else(|| target.stream_by_id("feedback-monitor".parse().unwrap()));
    let old_recv = source.stream_by_id(
        (if mode == 8 {
            "feedback-third-camera"
        } else {
            "feedback-monitor"
        })
        .parse()
        .unwrap(),
    );
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
    let original_operator_credential = key.id.clone();
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
                    previous: (if mode == 8 {
                        "feedback-third-camera"
                    } else {
                        "feedback-monitor"
                    })
                    .into(),
                    replacement: "replacement-incoming-camera".into(),
                }],
            }),
        },
        payload_extensions: Default::default(),
    };
    #[cfg(feature = "test-support")]
    let held_registry_probe = if mode == 3 {
        Some(
            f.server
                .service
                .arm_canonical_held_registry_probe_for_test(&command.operation_id)
                .unwrap(),
        )
    } else {
        None
    };
    eprintln!("production replacement phase: replacement-prepare");
    #[cfg(feature = "test-support")]
    let preparation_registry_probe = if mode == 4 {
        Some(
            f.server
                .service
                .arm_canonical_preparation_registry_probe_for_test(&command.operation_id)
                .unwrap(),
        )
    } else {
        None
    };
    #[cfg(feature = "test-support")]
    let start_registry_probe = if mode == 6 {
        Some(
            f.server
                .service
                .arm_canonical_start_registry_probe_for_test(&command.operation_id)
                .unwrap(),
        )
    } else {
        None
    };
    #[cfg(feature = "test-support")]
    let video_preparation_probe = if mode == 17 {
        Some(
            f.server
                .service
                .arm_canonical_video_preparation_probe_for_test(&command.operation_id)
                .unwrap(),
        )
    } else {
        None
    };
    let root = controller.transition(command.clone()).await.unwrap();
    phase(&f, &mut controller, &command, 1).await;
    let original_preparation = readiness::original_cutoff(&f, &root);
    let original_cutoff = original_preparation.cutoff;
    eprintln!("production replacement phase: replacement-allocate");
    let mut replacement = CanonicalClient::open(replacement_config).await.unwrap();
    let mut offer = json!({"substrate":"quic","capabilities":{"audio_codecs":[{"name":"pcmu","params":{"sample_rate":8000,"channels":1}}],"video_codecs":[{"name":"vp8","params":{"sample_rate":90000,"channels":0}}],"max_streams_per_connection":8,"video_feedback":"rtcp-pli-epoch/1"},"streams_offered":[{"id":"replacement-voice","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]},{"id":"replacement-camera","kind":"video","direction":"sendonly","codec_preferences":["vp8"]},{"id":"replacement-incoming-camera","kind":"video","direction":"recvonly","codec_preferences":["vp8"]}]});
    if mode >= 9 {
        offer["capabilities"]["audio_codecs"] =
            json!([{"name":"opus","params":{"sample_rate":48000,"channels":1}}]);
        offer["streams_offered"][0]["codec_preferences"] = json!(["opus"]);
    }
    if mode == 2 {
        offer["capabilities"]["audio_codecs"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name":"pcma","params":{"sample_rate":8000,"channels":1}}));
    }
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
    if mode >= 9 {
        let selected = bindings
            .iter()
            .find(|binding| binding["strm_id"] == "replacement-voice")
            .unwrap();
        assert_eq!(selected["codec"]["name"], "opus");
        assert_eq!(selected["codec"]["params"]["sample_rate"], 48000);
        assert_eq!(selected["codec"]["params"]["channels"], 1);
        assert_eq!(selected["codec"]["params"]["payload_type"], 111);
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
    let response = replacement.native_replacement_answer(&callback).unwrap();
    tokio::time::timeout_at(
        original_cutoff,
        replacement.answer_connection_offer(&callback, response, &scope),
    )
    .await
    .expect("original replacement expired during exact answer")
    .unwrap();
    eprintln!("production replacement phase: replacement-resource-capture");
    phase(&f, &mut controller, &command, 5).await;
    let mut io = replacement.claim_native_video_datagrams().unwrap();
    let mut sequence = 0u16;
    #[cfg(feature = "test-support")]
    let original_video_phase_rows = if let Some(probe) = video_preparation_probe.as_ref() {
        let probe_cutoff =
            original_cutoff.min(tokio::time::Instant::now() + Duration::from_secs(2));
        tokio::time::timeout_at(probe_cutoff, probe.entered())
            .await
            .expect("original video-state exclusion was not observed");
        assert_eq!(
            probe.observed(),
            (1, true, 0, 0),
            "actual old-state MetadataBusy must precede any private route publication"
        );
        let before = readiness::original_video_phase_rows(&f, &original_preparation.record_id);
        assert_eq!(before.len(), 2);
        // Repacketize the actual original key payload to prove old-route
        // liveness under fresh source authority. This is not another encoder
        // capture: its indices must stay0,1,2... for the later real private/
        // public decoder proof. Advance RTP to1500 with fresh outer sequences,
        // before the unchanged private marker3000; never replay a packet.
        publish_timestamp(&retained_send, &first, 1500, &mut retained_sequence).await;
        let (original, history) = receive_timestamp(&old_recv, 1500).await;
        assert_eq!(original, first);
        assert_eq!(history, previous_target_epoch);
        // The SDK's original signed reader is active. No private route exists,
        // no target video packet or signed receiver notice may be published.
        let observe_until =
            probe_cutoff.min(tokio::time::Instant::now() + Duration::from_millis(30));
        loop {
            match tokio::time::timeout_at(observe_until, io.receive()).await {
                Err(_) => break,
                Ok(result) => assert!(
                    result.unwrap().video_epoch.is_none(),
                    "busy preparation published private signed video"
                ),
            }
        }
        assert_eq!(
            io.verified_video_receiver_notice_count_for_test().unwrap(),
            0
        );
        let now = readiness::original_cutoff(&f, &root);
        readiness::assert_same_original_preparation(&original_preparation, &now);
        assert!(tokio::time::Instant::now() < original_cutoff);
        probe.release();
        tokio::time::timeout_at(probe_cutoff, probe.left())
            .await
            .unwrap();
        Some(before)
    } else {
        None
    };
    if (11..=16).contains(&mode) {
        let frozen = readiness::private_bootstrap(
            &f,
            &mut controller,
            &command,
            &root,
            &mut replacement,
            &mut io,
            &mut encoder,
            &retained_send,
            &mut retained_sequence,
            incoming,
            original_cutoff,
            mode == 12,
            mode,
            &original_preparation,
            &old_recv,
        )
        .await;
        #[cfg(feature = "test-support")]
        if let Some(ready) = frozen {
            let original_origin = f
                .server
                .store
                .native_replacement_origin(&original_operator_credential, command.clone())
                .unwrap();
            assert_eq!(original_origin.record_id(), original_preparation.record_id);
            // Every negative starts with genuine original private decoding;
            // its Ready was deliberately never submitted before the boundary.
            if mode == 16 {
                replacement.shutdown().await.unwrap();
                readiness::refuse_cancelled_target(
                    &f,
                    &command,
                    &original_preparation,
                    &original_origin,
                    &mut io,
                    &mut encoder,
                    &retained_send,
                    &old_recv,
                    &mut retained_sequence,
                )
                .await;
            } else {
                readiness::refuse_original_boundary(
                    &f,
                    &command,
                    &original_preparation,
                    &original_origin,
                    &mut replacement,
                    &ready,
                    &mut io,
                    &mut encoder,
                    &retained_send,
                    &old_recv,
                    &mut retained_sequence,
                    &original_operator_credential,
                    mode,
                )
                .await;
                replacement.shutdown().await.unwrap();
            }
        } else {
            replacement.shutdown().await.unwrap();
        }
        #[cfg(not(feature = "test-support"))]
        {
            assert!(frozen.is_none(), "boundary-only cases require test-support");
            replacement.shutdown().await.unwrap();
        }
        encoder.close().await;
        drop(io);
        publisher.close(options()).await.unwrap();
        subscriber.close(options()).await.unwrap();
        f.server.shutdown().await.unwrap();
        return;
    }
    let (done, original_receiver_history) = readiness::activate(
        &f,
        &mut controller,
        &command,
        &root,
        &mut replacement,
        &mut io,
        &mut encoder,
        &retained_send,
        &mut retained_sequence,
        voice,
        incoming,
        camera,
        &retained_recv,
        &mut sequence,
        original_cutoff,
    )
    .await;
    assert_eq!(done["state"], "completed");
    assert_eq!(done["result"]["generation"], 2);
    #[cfg(feature = "test-support")]
    if let Some(before) = original_video_phase_rows {
        assert_eq!(
            readiness::original_video_phase_rows(&f, &original_preparation.record_id),
            before,
            "video metadata retry replayed Attach or Activate"
        );
        let now = readiness::original_cutoff(&f, &root);
        readiness::assert_same_original_preparation(&original_preparation, &now);
        assert!(tokio::time::Instant::now() < original_cutoff);
        assert_eq!(done["record_id"], original_preparation.record_id);
        assert_eq!(
            io.verified_video_receiver_notice_count_for_test().unwrap(),
            1
        );
        assert!(io
            .is_current_video_receive_epoch(incoming, &original_receiver_history)
            .unwrap());
        assert_eq!(
            video_preparation_probe.as_ref().unwrap().observed(),
            (1, true, 0, 0)
        );
        eprintln!("signed video pre-effect contention: actual_old_mutex=true typed_busy=true no_private_route=true no_verified_receiver_notice=true same_root_reservation_generation_expiry=true same_private_public_history=true attach_activate_unchanged=true");
    }
    #[cfg(feature = "test-support")]
    if let Some(probe) = held_registry_probe {
        assert_eq!(
            probe.load(std::sync::atomic::Ordering::Acquire),
            2,
            "legacy lookup and expired invocation must refuse while current owned entries survive registry contention"
        );
    }

    // The test-owned decoder was explicitly retired and reset after a real
    // private-key/public-interframe continuity proof. Now it receives an
    // interframe from the unchanged retained source. Only actual signed history can solicit source recovery.
    eprintln!("production replacement phase: replacement-receiver-recovery");
    assert!(
        matches!(retained_send.receive_video_feedback(RequestOptions::new(Duration::from_millis(30)).unwrap()).await,
        Err(error) if error.kind==sdk::ErrorKind::Deadline),
        "cached publisher feedback cannot qualify fresh postcommit receiver recovery"
    );
    let delta = encoder.frame(3, false).await;
    assert_eq!(delta[0] & 1, 1);
    publish(&retained_send, &delta, 3, &mut retained_sequence).await;
    let (bytes, epoch) = raw_receive(&mut io, 3, incoming).await;
    assert_eq!(bytes, delta);
    assert_ne!(epoch.epoch, previous_target_epoch.epoch);
    assert_eq!(
        epoch, original_receiver_history,
        "cold recovery must preserve the entire actually signed private/public decoder history"
    );
    let hint = retained_send
        .receive_video_feedback(options())
        .await
        .unwrap();
    assert!(retained_send.video_feedback_is_current(&hint).unwrap());
    assert_eq!(hint.ssrc(), 0x74657374);
    assert!(hint.expires_in() > Duration::ZERO && hint.expires_in() <= Duration::from_millis(200));
    let first_recovery_feedback_sequence = hint.datagram_sequence();
    let fresh = encoder
        .frame(4, retained_send.video_feedback_is_current(&hint).unwrap())
        .await;
    publish(&retained_send, &fresh, 4, &mut retained_sequence).await;
    let (bytes, same) = raw_receive(&mut io, 4, incoming).await;
    assert_eq!(same, epoch);
    assert_eq!(bytes, fresh);
    decode(f.temp.path(), &bytes, 4).await;
    let before_gate_receive = same.clone();
    // In the reverse direction the retained receiver requests a keyframe from
    // the raw generation-two publisher. Pumping receives the physical feedback;
    // the boolean is consumed once immediately before the next fresh encoding.
    eprintln!("production replacement phase: replacement-publisher-recovery");
    let delta = encoder.frame(5, false).await;
    assert_eq!(delta[0] & 1, 1);
    raw_publish(&io, camera, &delta, 5, &mut sequence);
    let (bytes, epoch) = receive(&retained_recv, 5).await;
    assert_eq!(bytes, delta);
    assert_ne!(epoch.epoch, previous_retained_epoch.epoch);
    retained_recv.request_keyframe(options()).await.unwrap();
    // A data-free receive may remain pending after consuming feedback. Its
    // cancellation must retain the hint's original arrival and one-use state.
    let _ = tokio::time::timeout(Duration::from_millis(50), io.receive()).await;
    let requested = io.take_keyframe_request(camera, 0x74657374).unwrap();
    assert!(requested);
    let fresh = encoder.frame(6, requested).await;
    assert!(!io.take_keyframe_request(camera, 0x74657374).unwrap());
    raw_publish(&io, camera, &fresh, 6, &mut sequence);
    let (bytes, same) = receive(&retained_recv, 6).await;
    assert_eq!(same, epoch);
    assert_eq!(bytes, fresh);
    decode(f.temp.path(), &bytes, 6).await;
    if mode == 7 {
        tokio::time::sleep_until(original_cutoff + Duration::from_millis(20)).await;
        assert!(chrono::Utc::now().timestamp() >= original_preparation.expires_at);
        let (record,reservation,expiry,candidate,runtime,reserved_generation,generation,observed_runtime):
            (String,String,i64,String,String,u64,u64,String)=rusqlite::Connection::open(&f.config.database).unwrap()
            .query_row("SELECT n.operation_record_id,n.reservation_id,n.expires_at,n.candidate_connection_id,n.candidate_runtime_id,n.generation,c.generation,m.runtime_connection_id FROM native_replacements n JOIN connections c ON c.id=n.candidate_connection_id JOIN connection_offers m ON m.connection_id=c.id WHERE n.operation_record_id=?1",
                [&original_preparation.record_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?))).unwrap();
        assert_eq!(record, original_preparation.record_id);
        assert_eq!(record, root.record_id.as_ref().unwrap().as_str());
        assert_eq!(reservation, original_preparation.reservation_id);
        assert_eq!(
            expiry, original_preparation.expires_at,
            "ordinary lifecycle must not renew the original preparation"
        );
        assert_eq!(candidate, original_preparation.candidate_connection_id);
        assert_eq!(candidate, scope.connection_id.as_ref().unwrap().as_str());
        assert_eq!(runtime, original_preparation.candidate_runtime_id);
        assert_eq!(observed_runtime, original_preparation.candidate_runtime_id);
        assert_eq!(reserved_generation, original_preparation.generation);
        assert_eq!(generation, original_preparation.generation);
        assert_eq!(generation, 2);
        eprintln!("production replacement phase: original-cutoff-elapsed unchanged_root={record} unchanged_preparation_expiry={expiry} generation={generation}");
    }
    if mode == 1 || mode == 5 || mode == 7 {
        eprintln!("production replacement phase: signed-hold-resume");
        let revision: u64 = rusqlite::Connection::open(&f.config.database)
            .unwrap()
            .query_row(
                "SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1",
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
        let delta = encoder.frame(7, false).await;
        assert_eq!(delta[0] & 1, 1);
        publish(&retained_send, &delta, 7, &mut retained_sequence).await;
        let (bytes, resumed_receive) = raw_receive(&mut io, 7, incoming).await;
        assert_eq!(bytes, delta);
        assert_ne!(resumed_receive.epoch, before_gate_receive.epoch);
        assert_eq!(
            resumed_receive.incarnation, before_gate_receive.incarnation,
            "ordinary receiver history must retain the original physical registration"
        );
        let hint = retained_send
            .receive_video_feedback(options())
            .await
            .unwrap();
        assert!(retained_send.video_feedback_is_current(&hint).unwrap());
        let distance = hint
            .datagram_sequence()
            .wrapping_sub(first_recovery_feedback_sequence);
        assert!(
            distance > 0 && distance < (1u32 << 31),
            "same publisher feedback cursor was reset or replayed"
        );
        let fresh = encoder.frame(8, true).await;
        publish(&retained_send, &fresh, 8, &mut retained_sequence).await;
        let (bytes, current) = raw_receive(&mut io, 8, incoming).await;
        assert_eq!(current, resumed_receive);
        assert_eq!(bytes, fresh);
        decode(f.temp.path(), &bytes, 8).await;
        let delta = encoder.frame(9, false).await;
        assert_eq!(delta[0] & 1, 1);
        raw_publish(&io, camera, &delta, 9, &mut sequence);
        let (bytes, resumed_publish) = receive(&retained_recv, 9).await;
        assert_eq!(bytes, delta);
        assert_ne!(resumed_publish.epoch, epoch.epoch);
        assert_eq!(resumed_publish.incarnation, epoch.incarnation);
        retained_recv.request_keyframe(options()).await.unwrap();
        let _ = tokio::time::timeout(Duration::from_millis(50), io.receive()).await;
        let requested = io.take_keyframe_request(camera, 0x74657374).unwrap();
        assert!(requested);
        let fresh = encoder.frame(10, requested).await;
        assert!(!io.take_keyframe_request(camera, 0x74657374).unwrap());
        raw_publish(&io, camera, &fresh, 10, &mut sequence);
        let (bytes, current) = receive(&retained_recv, 10).await;
        assert_eq!(current, resumed_publish);
        assert_eq!(bytes, fresh);
        decode(f.temp.path(), &bytes, 10).await;
        eprintln!(
            "production replacement gate proof: signed generation2 hold/resume, fresh epoch and independent VP8 decode in both directions at original RTP24000 and30000"
        );
    }
    if mode == 2 {
        eprintln!("production replacement phase: signed-codec-resource-adoption");
        let revision:u64=rusqlite::Connection::open(&f.config.database).unwrap().query_row("SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1",[scope.connection_id.as_ref().unwrap()],|r|r.get(0)).unwrap();
        let result = codec_change(&mut replacement, &scope, &offer["capabilities"], revision).await;
        assert_eq!(result["resources_changed"], true);
        assert_eq!(result["stream_revision"], revision + 1);
        let bindings = result["codec_bindings"].as_array().unwrap();
        let audio = bindings
            .iter()
            .find(|b| b["stream_id"] == "replacement-voice")
            .unwrap();
        assert_eq!(audio["connection_generation"], 2);
        assert_eq!(audio["stream_generation"], 2);
        assert_eq!(audio["codec"]["params"]["payload_type"], 8);
        let new_voice = audio["stream_local_id"].as_u64().unwrap() as u16;
        assert_ne!(new_voice, voice);
        for (id, handle) in [
            ("replacement-camera", camera),
            ("replacement-incoming-camera", incoming),
        ] {
            let b = bindings.iter().find(|b| b["stream_id"] == id).unwrap();
            assert_eq!(b["stream_local_id"], handle);
            assert_eq!(b["stream_generation"], 1);
            assert_eq!(b["connection_generation"], 2);
        }
        use rvoip_uctp::substrate::{pack, MediaDatagram};
        for n in 1..=12u16 {
            io.send(&pack(&MediaDatagram {
                flags: 0,
                stream_local_id: new_voice,
                seq: u32::from(n),
                payload: rtp(8, n, u32::from(n) * 160, 0x72727272, false, &[0xd5; 160]).into(),
            }))
            .unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let fresh = encoder.frame(7, true).await;
        publish(&retained_send, &fresh, 7, &mut retained_sequence).await;
        let (bytes, _) = raw_receive(&mut io, 7, incoming).await;
        assert_eq!(bytes, fresh);
        decode(f.temp.path(), &bytes, 7).await;
        retained_recv.request_keyframe(options()).await.unwrap();
        let _ = tokio::time::timeout(Duration::from_millis(50), io.receive()).await;
        let requested = io.take_keyframe_request(camera, 0x74657374).unwrap();
        assert!(requested);
        let fresh = encoder.frame(8, requested).await;
        assert!(!io.take_keyframe_request(camera, 0x74657374).unwrap());
        raw_publish(&io, camera, &fresh, 8, &mut sequence);
        let (bytes, _) = receive(&retained_recv, 8).await;
        assert_eq!(bytes, fresh);
        decode(f.temp.path(), &bytes, 8).await;
        eprintln!(
            "production replacement codec proof: actual generation2 signed codec commit, new PCMA handle/generation, retained video allocations, post-commit video decode original RTP21000/24000 and one-use current feedback"
        );
    }
    encoder.close().await;
    replacement.shutdown().await.unwrap();
    drop(io);
    publisher.close(options()).await.unwrap();
    subscriber.close(options()).await.unwrap();
    if let Some(third) = third_client {
        third.close(options()).await.unwrap();
    }
    f.server.shutdown().await.unwrap();
    #[cfg(feature = "test-support")]
    if let Some(probe) = start_registry_probe {
        assert_eq!(
            probe.load(std::sync::atomic::Ordering::Acquire),
            2,
            "initial mute: contended registry refuses the old lookup, exact held owners succeed, expired authority remains refused"
        );
    }
    #[cfg(feature = "test-support")]
    if let Some(probe) = preparation_registry_probe {
        let bits = probe.load(std::sync::atomic::Ordering::Acquire);
        eprintln!(
            "original preparation registry probe: before={} held={} cancelled_refused={} after={}",
            bits & 1 != 0,
            bits & 2 != 0,
            bits & 4 != 0,
            bits & 8 != 0
        );
        eprintln!(
            "original preparation refusal probe: wrong_owner={} missing_owner={} changed_intent={} expired={} initial_capture_busy={}",
            bits & 16 != 0,
            bits & 32 != 0,
            bits & 64 != 0,
            bits & 128 != 0,
            bits & 256 != 0
        );
        assert_eq!(
            bits, 511,
            "original preparation retains exact ownership and current authority under metadata exclusion"
        );
    }
    eprintln!(
        "production replacement: signed generation2, transferred both original video selections, real receiver feedback and fresh libvpx keys independently decoded at original RTP12000 and18000; one-use raw publisher hint; owned peers and encoder joined; no full transport cell qualified"
    );
}
