//! Public native-client actor for the browser/native functional lane.
//! This is not a live encoder, native presenter or operating-profile witness.
use super::*;
use connect_client::{canonical::CommandScope, NativeVideoPacket, VideoFrameAssembler};
use std::{collections::HashMap, sync::Mutex, time::Instant};

#[derive(Default)]
pub(super) struct Coordination {
    values: Mutex<HashMap<String, bool>>,
    window: Mutex<(Option<Instant>, Option<Instant>)>,
}
impl Coordination {
    pub(super) fn signal(&self, name: &str) {
        if self.has(name) {
            return;
        }
        assert!([
            "browser-offered",
            "browser-subscribed",
            "data-go",
            "native-offered",
            "native-subscribed",
            "native-data-sent",
            "measure-start",
            "measure-end",
            "native-end",
            "native-ended",
        ]
        .contains(&name));
        if name == "measure-start" {
            let mut window = self.window.lock().unwrap();
            assert!(window.0.is_none() && window.1.is_none());
            window.0 = Some(Instant::now());
        } else if name == "measure-end" {
            let mut window = self.window.lock().unwrap();
            assert!(window.0.is_some() && window.1.is_none());
            window.1 = Some(Instant::now());
        }
        self.values.lock().unwrap().insert(name.into(), true);
    }
    pub(super) fn has(&self, name: &str) -> bool {
        self.values
            .lock()
            .unwrap()
            .get(name)
            .copied()
            .unwrap_or(false)
    }
    pub(super) fn snapshot(&self) -> Value {
        json!(*self.values.lock().unwrap())
    }
}

fn capabilities() -> Value {
    json!({"audio_codecs":[{"name":"opus","params":{"sample_rate":48000,"channels":1}}],
        "video_codecs":[{"name":"vp8","params":{"sample_rate":90000,"channels":0}}],
        "data_protocols":["text","json","binary"],"max_streams_per_connection":32,
        "identity_assurance_offered":"identified"})
}
#[derive(Clone)]
struct Binding {
    handle: u16,
    pt: u8,
    send: bool,
    receive: bool,
    kind: String,
}
struct Source {
    audio: Vec<Vec<u8>>,
    video: Vec<Vec<u8>>,
    epoch: Instant,
    next_audio: usize,
    next_video: usize,
    audio_sequence: u16,
    video_sequence: u16,
    datagram_sequence: u32,
}
impl Source {
    fn read(directory: &Path) -> Self {
        let raw = std::fs::read(directory.join("audio/native-250hz.frames")).unwrap();
        let mut offset = 0;
        let mut audio = Vec::new();
        while offset < raw.len() {
            let length = usize::from(u16::from_le_bytes(
                raw[offset..offset + 2].try_into().unwrap(),
            ));
            offset += 2;
            assert!((1..=1168).contains(&length) && offset + length <= raw.len());
            audio.push(raw[offset..offset + length].to_vec());
            offset += length;
        }
        assert_eq!(audio.len(), 200);
        let raw = std::fs::read(directory.join("video/native-blue-vp8.ivf")).unwrap();
        assert_eq!(&raw[..12], b"DKIF\0\0\x20\0VP80");
        assert_eq!(u16::from_le_bytes(raw[12..14].try_into().unwrap()), 640);
        assert_eq!(u16::from_le_bytes(raw[14..16].try_into().unwrap()), 360);
        assert_eq!(u32::from_le_bytes(raw[16..20].try_into().unwrap()), 30);
        assert_eq!(u32::from_le_bytes(raw[20..24].try_into().unwrap()), 1);
        let mut video = Vec::new();
        offset = 32;
        while offset < raw.len() {
            let length = u32::from_le_bytes(raw[offset..offset + 4].try_into().unwrap()) as usize;
            let pts = u64::from_le_bytes(raw[offset + 4..offset + 12].try_into().unwrap());
            assert_eq!(pts as usize, video.len());
            offset += 12;
            assert!((1..=256 * 1024).contains(&length) && offset + length <= raw.len());
            video.push(raw[offset..offset + length].to_vec());
            offset += length;
        }
        assert_eq!(video.len(), 120);
        assert!((0..120)
            .filter(|i| video[*i][0] & 1 == 0)
            .eq([0, 30, 60, 90]));
        Self {
            audio,
            video,
            epoch: Instant::now(),
            next_audio: 0,
            next_video: 0,
            audio_sequence: 1000,
            video_sequence: 10000,
            datagram_sequence: 0,
        }
    }
    fn packet(
        &mut self,
        binding: &Binding,
        payload: &[u8],
        timestamp: u32,
        sequence: u16,
        marker: bool,
        video_start: Option<bool>,
    ) -> Vec<u8> {
        let mut packet = vec![1, 0];
        packet.extend_from_slice(&binding.handle.to_be_bytes());
        packet.extend_from_slice(&self.datagram_sequence.to_be_bytes());
        self.datagram_sequence = self.datagram_sequence.wrapping_add(1);
        packet.extend_from_slice(&[0x80, binding.pt | if marker { 0x80 } else { 0 }]);
        packet.extend_from_slice(&sequence.to_be_bytes());
        packet.extend_from_slice(&timestamp.to_be_bytes());
        packet.extend_from_slice(
            &(if video_start.is_some() {
                0x56503801u32
            } else {
                0x4f505501u32
            })
            .to_be_bytes(),
        );
        if let Some(start) = video_start {
            packet.push(if start { 0x10 } else { 0 });
        }
        packet.extend_from_slice(payload);
        assert!(packet.len() <= 1188);
        // Lossless substrate parser verifies the actual RTP image. Do not use
        // pack_rtp_datagram: its normalized payload omits the VP8 marker bit.
        rvoip_uctp::substrate::observe_rtp_datagram(&packet).unwrap();
        packet
    }
    fn pump(&mut self, client: &CanonicalClient, bindings: &HashMap<String, Binding>) {
        let elapsed = self.epoch.elapsed().as_micros();
        let audio_index = (elapsed / 20000) as usize;
        if let Some(binding) = bindings.get("browser-audio-1").filter(|b| b.send) {
            if audio_index >= self.next_audio {
                self.audio_sequence = self
                    .audio_sequence
                    .wrapping_add((audio_index - self.next_audio) as u16);
                let encoded = self.audio[audio_index % self.audio.len()].clone();
                let packet = self.packet(
                    binding,
                    &encoded,
                    (audio_index as u32).wrapping_mul(960),
                    self.audio_sequence,
                    false,
                    None,
                );
                client.send_native_rtp_datagram(&packet).unwrap();
                self.audio_sequence = self.audio_sequence.wrapping_add(1);
                self.next_audio = audio_index + 1;
            }
        }
        let video_index = (elapsed * 30 / 1000000) as usize;
        if let Some(binding) = bindings.get("browser-video-1").filter(|b| b.send) {
            if video_index >= self.next_video {
                // A dropped encoded reference frame must leave a sequence gap.
                // Never conceal skipped source frames behind contiguous RTP.
                for skipped in self.next_video..video_index {
                    self.video_sequence = self.video_sequence.wrapping_add(
                        self.video[skipped % self.video.len()].len().div_ceil(1167) as u16,
                    );
                }
                let encoded = self.video[video_index % self.video.len()].clone();
                let fragments = encoded.len().div_ceil(1167);
                assert!(fragments <= 64);
                for (i, chunk) in encoded.chunks(1167).enumerate() {
                    let packet = self.packet(
                        binding,
                        chunk,
                        (video_index as u32).wrapping_mul(3000),
                        self.video_sequence,
                        i + 1 == fragments,
                        Some(i == 0),
                    );
                    client.send_native_rtp_datagram(&packet).unwrap();
                    self.video_sequence = self.video_sequence.wrapping_add(1);
                }
                self.next_video = video_index + 1;
            }
        }
    }
}

struct Actor {
    client: CanonicalClient,
    cid: String,
    source_participant: String,
    state: Arc<Coordination>,
    source: Source,
    started: Instant,
    bindings: HashMap<String, Binding>,
    revision: u64,
    operations: HashMap<String, Value>,
    queries: HashMap<String, (String, bool)>,
    data: Vec<Value>,
    audio: Vec<u8>,
    audio_events: Vec<Value>,
    video: Vec<u8>,
    video_events: Vec<Value>,
    assembler: VideoFrameAssembler,
    continuity: Option<(u32, u32, u16)>,
    waiting_key: bool,
    gaps: u64,
}
impl Actor {
    fn scope(&self, operation: &str, revision: Option<u64>) -> CommandScope {
        CommandScope {
            conversation_id: Some(self.cid.clone()),
            session_id: Some("http3-media-session".into()),
            connection_id: Some("browser-connection-1".into()),
            operation_id: Some(operation.into()),
            expected_revision: revision,
        }
    }
    async fn frame(&mut self, frame: Envelope) {
        let original = frame.original();
        assert_ne!(
            frame.kind(),
            "error",
            "native signed command refused: {original:?}"
        );
        if frame.kind() == "connection.offer" {
            for (field, value) in [
                ("cid", self.cid.as_str()),
                ("sid", "http3-media-session"),
                ("connid", "browser-connection-1"),
            ] {
                assert_eq!(original[field], value);
            }
            let operation = original["operation_id"].as_str().unwrap().to_owned();
            let payload = &original["payload"];
            assert_eq!(payload["substrate"], "quic");
            let setup = &payload["substrate_setup"];
            assert_eq!(setup["kind"], "native-prepared/1");
            assert!(!setup["offer_id"].as_str().unwrap().is_empty());
            let revision = setup["revision"].as_u64().unwrap();
            assert!(revision >= self.revision && revision > 0);
            let streams = setup["streams"].as_array().unwrap();
            assert!(!streams.is_empty() && streams.len() <= 32);
            let mut bindings = HashMap::new();
            let mut handles = std::collections::HashSet::new();
            let answered = streams
                .iter()
                .map(|s| {
                    let id = s["strm_id"].as_str().unwrap();
                    let kind = s["kind"].as_str().unwrap();
                    let direction = match s["direction"].as_str().unwrap() {
                        "sendonly" => "recvonly",
                        "recvonly" => "sendonly",
                        "sendrecv" => "sendrecv",
                        _ => panic!("native direction"),
                    };
                    let own = [
                        "browser-audio-1",
                        "browser-video-1",
                        "browser-text-1",
                        "browser-json-1",
                        "browser-binary-1",
                    ]
                    .contains(&id);
                    assert!(
                        own || [
                            "browser-video-0",
                            "browser-text-0",
                            "browser-json-0",
                            "browser-binary-0"
                        ]
                        .contains(&id)
                    );
                    assert!(own || direction == "recvonly");
                    let handle = u16::try_from(s["stream_local_id"].as_u64().unwrap()).unwrap();
                    assert!(handle > 0 && handles.insert(handle));
                    let codec = &s["codec"];
                    let params = &codec["params"];
                    if kind == "data" {
                        assert!(
                            ["text", "json", "binary"].contains(&codec["name"].as_str().unwrap())
                        );
                        assert_eq!(params["reliability"], "reliable_ordered");
                    } else {
                        match kind {
                            "audio" => {
                                assert_eq!(codec["name"], "opus");
                                assert_eq!(params["sample_rate"], 48000);
                                assert_eq!(params["channels"], 1);
                            }
                            "video" => {
                                assert_eq!(codec["name"], "vp8");
                                assert_eq!(params["sample_rate"], 90000);
                            }
                            _ => panic!("native media kind"),
                        }
                        let pt = u8::try_from(params["payload_type"].as_u64().unwrap()).unwrap();
                        assert!(pt <= 127);
                        assert!(bindings
                            .insert(
                                id.into(),
                                Binding {
                                    handle,
                                    pt,
                                    send: direction != "recvonly",
                                    receive: direction != "sendonly",
                                    kind: kind.into()
                                }
                            )
                            .is_none());
                    }
                    json!({"id":id,"kind":kind,"direction":direction,"codec":codec})
                })
                .collect::<Vec<_>>();
            self.client.answer_connection_offer(&frame,serde_json::from_value(json!({"by_participant":payload["by_participant"],"substrate":"quic","capabilities":capabilities(),"streams_answered":answered,"substrate_setup":setup})).unwrap(),&self.scope(&operation,None)).await.unwrap();
            // These bindings describe the submitted exact offer. Server-side
            // permission still gates every packet; only operation.get proves
            // command completion to the caller.
            self.bindings = bindings;
            self.revision = revision;
        } else if frame.kind() == "operation.updated" {
            assert_eq!(original["cid"], self.cid);
            let p = &original["payload"];
            if let Some((operation, received)) = original["in_reply_to"]
                .as_str()
                .and_then(|reply| self.queries.get_mut(reply))
            {
                assert_eq!(p["operation_id"], *operation);
                assert!(
                    !*received,
                    "duplicate original operation status in one reply"
                );
                *received = true;
                self.operations.insert(operation.clone(), p.clone());
            }
        } else if frame.kind() == "ack" {
            if let Some((_, received)) = original["in_reply_to"]
                .as_str()
                .and_then(|reply| self.queries.remove(reply))
            {
                assert!(
                    received,
                    "status ACK omitted its correlated authoritative state"
                );
            }
        } else if frame.kind() == "message.send" {
            for (field, value) in [
                ("cid", self.cid.as_str()),
                ("sid", "http3-media-session"),
                ("connid", "browser-connection-1"),
            ] {
                assert_eq!(original[field], value);
            }
            assert!(self.data.len() < 3);
            assert_eq!(original["payload"]["from"], self.source_participant);
            self.data.push(original.clone());
        }
    }
    fn datagram(&mut self, bytes: Vec<u8>) {
        let datagram = rvoip_uctp::substrate::observe_rtp_datagram(&bytes).unwrap();
        let binding = self
            .bindings
            .values()
            .find(|b| b.handle == datagram.stream_local_id && b.receive)
            .unwrap();
        assert_eq!(datagram.rtp.packet().header.payload_type, binding.pt);
        let at = self.started.elapsed().as_secs_f64() * 1000.;
        if binding.kind == "audio" {
            let payload = &datagram.rtp.packet().payload;
            assert!(!payload.is_empty() && payload.len() <= 1168);
            assert!(self.audio_events.len() < 1000);
            self.audio
                .extend_from_slice(&(payload.len() as u16).to_le_bytes());
            self.audio.extend_from_slice(payload);
            self.audio_events.push(json!({"arrival_ms":at}));
        } else {
            let packet = NativeVideoPacket::parse(bytes[8..].to_vec()).unwrap();
            if let Some(frame) = self.assembler.push(&packet).unwrap() {
                let range = self.assembler.completed_sequence_range().unwrap();
                if let Some((ssrc, timestamp, last)) = self.continuity {
                    if range.ssrc == ssrc
                        && (frame.timestamp_rtp.wrapping_sub(timestamp) as i32) <= 0
                    {
                        return;
                    }
                    if range.ssrc != ssrc || range.first != last.wrapping_add(1) {
                        self.waiting_key = true;
                        self.gaps += 1;
                    }
                }
                self.continuity = Some((range.ssrc, frame.timestamp_rtp, range.last));
                if frame.keyframe {
                    self.waiting_key = false;
                }
                if self.waiting_key {
                    return;
                }
                assert!(self.video_events.len() < 600);
                self.video
                    .extend_from_slice(&(frame.encoded.len() as u32).to_le_bytes());
                self.video
                    .extend_from_slice(&u64::from(frame.timestamp_rtp).to_le_bytes());
                self.video.extend_from_slice(&frame.encoded);
                self.video_events
                    .push(json!({"pts":frame.timestamp_rtp,"arrival_ms":at}));
            }
        }
    }
    async fn advance(&mut self) {
        assert!(
            self.started.elapsed() < Duration::from_secs(45),
            "native actor overall deadline"
        );
        self.source.pump(&self.client, &self.bindings);
        self.assembler.expire_stale();
        if let Ok(frame) =
            tokio::time::timeout(Duration::from_millis(1), self.client.receive()).await
        {
            self.frame(frame.unwrap()).await;
        }
        if let Ok(packet) = tokio::time::timeout(
            Duration::from_millis(1),
            self.client.receive_native_rtp_datagram(),
        )
        .await
        {
            self.datagram(packet.unwrap());
        }
    }
    async fn command(
        &mut self,
        kind: &str,
        operation: &str,
        payload: Value,
        revision: Option<u64>,
    ) {
        let scope = self.scope(operation, revision);
        self.client.send_value(kind, payload, scope).await.unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut next_query = Instant::now();
        loop {
            self.advance().await;
            if let Some(status) = self.operations.get(operation) {
                assert!(
                    !["failed", "cancelled", "rejected"]
                        .contains(&status["state"].as_str().unwrap()),
                    "original native operation terminal refusal: {status:?}"
                );
                if status["state"] == "completed" {
                    return;
                }
            }
            assert!(
                Instant::now() < deadline,
                "original native operation deadline: {operation}"
            );
            if Instant::now() >= next_query && self.queries.is_empty() {
                // Keep processing signed native parameter offers while a status
                // request waits for the original command to release authority.
                // Completion requires the correlated operation state; the ACK
                // alone neither completes the command nor opens another query.
                let query = self
                    .client
                    .send_value(
                        "operation.get",
                        json!({"operation_id":operation}),
                        CommandScope {
                            conversation_id: Some(self.cid.clone()),
                            ..Default::default()
                        },
                    )
                    .await
                    .unwrap();
                self.queries.insert(query, (operation.into(), false));
                next_query = Instant::now() + Duration::from_millis(50);
            }
        }
    }
    async fn until(&mut self, flag: &str) {
        while !self.state.has(flag) {
            self.advance().await;
        }
    }
}

pub(super) async fn run(
    config: CanonicalClientConfig,
    cid: String,
    source_participant: String,
    state: Arc<Coordination>,
    fixtures: &Path,
    output: &Path,
) -> Value {
    let mut client = CanonicalClient::open(config).await.unwrap();
    client.start_signaling_reader().unwrap();
    let mut actor = Actor {
        client,
        cid,
        source_participant,
        state,
        source: Source::read(fixtures),
        started: Instant::now(),
        bindings: HashMap::new(),
        revision: 0,
        operations: HashMap::new(),
        queries: HashMap::new(),
        data: Vec::new(),
        audio: Vec::new(),
        audio_events: Vec::new(),
        video: Vec::new(),
        video_events: Vec::new(),
        assembler: VideoFrameAssembler::default(),
        continuity: None,
        waiting_key: true,
        gaps: 0,
    };
    let mut streams = vec![
        json!({"id":"browser-audio-1","kind":"audio","direction":"sendrecv","codec_preferences":["opus"]}),
        json!({"id":"browser-video-1","kind":"video","direction":"sendrecv","codec_preferences":["vp8"]}),
    ];
    for protocol in ["text", "json", "binary"] {
        streams.push(json!({"id":format!("browser-{protocol}-1"),"kind":"data","direction":"sendrecv","codec_preferences":[protocol]}));
    }
    actor
        .command(
            "connection.offer",
            "browser-offer-1",
            json!({"substrate":"quic","capabilities":capabilities(),"streams_offered":streams}),
            Some(0),
        )
        .await;
    actor.state.signal("native-offered");
    actor.until("browser-offered").await;
    actor.command("stream.subscribe","browser-subscribe-1",json!({"subscriptions":[{"from_participant":actor.source_participant,"kinds":["audio","video","data"]}]}),Some(actor.revision)).await;
    actor.state.signal("native-subscribed");
    actor.until("data-go").await;
    for protocol in ["text", "json", "binary"] {
        let (content_type, body, encoding) = match protocol {
            "text" => ("text/plain", "identified sender 1 😀".into(), "utf8"),
            "json" => (
                "application/json",
                json!({"source":1,"marker":"ordered-json"}).to_string(),
                "utf8",
            ),
            _ => (
                "application/octet-stream",
                base64::engine::general_purpose::STANDARD.encode([0, 255, 17, 1]),
                "base64",
            ),
        };
        actor.command("message.send",&format!("browser-data-1-{protocol}"),json!({"msg_id":format!("browser-message-1-{protocol}"),"to":"all","content_type":content_type,"label":format!("browser-{protocol}-1"),"reliability":{"mode":"reliable_ordered"},"body":body,"body_encoding":encoding}),Some(actor.revision)).await;
    }
    actor.state.signal("native-data-sent");
    actor.until("native-end").await;
    actor
        .command(
            "connection.end",
            "browser-end-1",
            json!({"reason_code":200,"reason":"normal-clearing"}),
            None,
        )
        .await;
    actor.client.shutdown().await.unwrap();
    actor.state.signal("native-ended");
    assert_eq!(actor.data.len(), 3);
    for protocol in ["text", "json", "binary"] {
        let message = actor
            .data
            .iter()
            .find(|m| m["operation_id"] == format!("browser-data-0-{protocol}"))
            .unwrap();
        let payload = &message["payload"];
        assert_eq!(payload["msg_id"], format!("browser-message-0-{protocol}"));
        assert_eq!(payload["label"], format!("browser-{protocol}-0"));
        match protocol {
            "text" => assert_eq!(payload["body"], "identified sender 0 😀"),
            "json" => assert_eq!(
                serde_json::from_str::<Value>(payload["body"].as_str().unwrap()).unwrap(),
                json!({"source":0,"marker":"ordered-json"})
            ),
            _ => assert_eq!(
                base64::engine::general_purpose::STANDARD
                    .decode(payload["body"].as_str().unwrap())
                    .unwrap(),
                [0, 255, 17, 0]
            ),
        }
    }
    let window = *actor.state.window.lock().unwrap();
    let start = window
        .0
        .unwrap()
        .duration_since(actor.started)
        .as_secs_f64()
        * 1000.;
    let end = window
        .1
        .unwrap()
        .duration_since(actor.started)
        .as_secs_f64()
        * 1000.;
    assert!(end - start >= 4000.);
    std::fs::create_dir(output).unwrap();
    std::fs::write(output.join("received.frames"), actor.audio).unwrap();
    let mut header = b"DKIF\0\0\x20\0VP80".to_vec();
    header.extend_from_slice(&640u16.to_le_bytes());
    header.extend_from_slice(&360u16.to_le_bytes());
    header.extend_from_slice(&90000u32.to_le_bytes());
    header.extend_from_slice(&1u32.to_le_bytes());
    header.extend_from_slice(&(actor.video_events.len() as u32).to_le_bytes());
    header.extend_from_slice(&0u32.to_le_bytes());
    header.extend_from_slice(&actor.video);
    std::fs::write(output.join("received.ivf"), header).unwrap();
    let result = json!({"window_start_ms":start,"window_end_ms":end,"audio_events":actor.audio_events,"video_events":actor.video_events,"video_sequence_gaps":actor.gaps,"assembly":actor.assembler.diagnostics(),"data_messages":3,"canonical_public_native_client":true,"native_transport":"quic","native_live_encode_playout_qualified":false});
    std::fs::write(
        output.join("received.json"),
        serde_json::to_vec(&result).unwrap(),
    )
    .unwrap();
    result
}

pub(super) fn endpoint(f: &Fixture, config: &CanonicalClientConfig) {
    let admin = f.server.store.authenticate(&f.admin).unwrap().credential_id;
    let ProvisioningOutcome::Endpoint { endpoint_id, .. } = f
        .server
        .store
        .execute_provisioning(
            &admin,
            ProvisioningCommand::Endpoint {
                operation_id: "browser-native-endpoint".into(),
                identity_id: config.identity_id.clone(),
                device: DeviceSpec {
                    id: config.device_id.clone(),
                    kind: config.device_kind.clone(),
                    platform: config.device_platform.clone(),
                },
                transport: EndpointTransport::Quic,
                address: format!(
                    "quic://localhost:{}/browser-native",
                    f.server.address.port()
                ),
                capabilities: vec!["audio".into(), "video".into(), "data".into()],
                secret_ref: None,
            },
        )
        .unwrap()
    else {
        panic!("native fixture endpoint")
    };
    f.server
        .store
        .execute_provisioning(
            &admin,
            ProvisioningCommand::VerifyEndpoint {
                operation_id: "browser-native-verify".into(),
                endpoint_id,
                expected_revision: 0,
                evidence_ref: "isolated-public-native-browser-fixture".into(),
            },
        )
        .unwrap();
}
