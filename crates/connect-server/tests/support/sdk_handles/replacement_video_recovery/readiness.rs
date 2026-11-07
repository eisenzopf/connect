//! Actual original private decoding and unchanged decoder/epoch through commit.
//! The later cold-start recovery deliberately owns a fresh application decoder.
use super::*;
use connect_client::{
    canonical::{NativeDecodedVideoReadiness, NativeReplacementReadiness, NativeReplacementReady},
    ClientError, NativeVideoFrame, NativeVideoPacket, VideoFrameAssembler,
};
use futures_util::FutureExt;
use sha2::{Digest, Sha256};
use std::{io::Read, panic::AssertUnwindSafe};
use tokio::time::{timeout_at, Instant};

pub(super) fn file_sha256(path: &Path) -> String {
    let mut input = std::fs::File::open(path).unwrap();
    let mut hash = Sha256::new();
    let mut block = [0u8; 65536];
    loop {
        let n = input.read(&mut block).unwrap();
        if n == 0 {
            break;
        }
        hash.update(&block[..n]);
    }
    format!("{:x}", hash.finalize())
}
pub(super) struct OriginalPreparation {
    pub(super) record_id: String,
    pub(super) reservation_id: String,
    pub(super) expires_at: i64,
    pub(super) candidate_connection_id: String,
    pub(super) candidate_runtime_id: String,
    pub(super) generation: u64,
    pub(super) cutoff: Instant,
}
pub(super) fn original_cutoff(
    f: &Fixture,
    root: &connect_client::canonical::payloads::OperationUpdated,
) -> OriginalPreparation {
    let (record_id,reservation_id,expires_at,candidate_connection_id,candidate_runtime_id,generation):
        (String,String,i64,String,String,u64)=rusqlite::Connection::open(&f.config.database).unwrap()
        .query_row("SELECT operation_record_id,reservation_id,expires_at,candidate_connection_id,candidate_runtime_id,generation FROM native_replacements WHERE operation_record_id=?1",
            [root.record_id.as_ref().unwrap().as_str()],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?))).unwrap();
    assert_eq!(record_id, root.record_id.as_ref().unwrap().as_str());
    assert_eq!(generation, 2);
    let wall = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    let remaining = Duration::from_secs(u64::try_from(expires_at).unwrap())
        .checked_sub(wall)
        .expect("original replacement preparation expiry already passed");
    OriginalPreparation {
        record_id,
        reservation_id,
        expires_at,
        candidate_connection_id,
        candidate_runtime_id,
        generation,
        cutoff: Instant::now() + remaining,
    }
}
#[cfg(feature = "test-support")]
pub(super) fn original_video_phase_rows(
    f: &Fixture,
    record: &str,
) -> Vec<(String, String, String, bool, String, String)> {
    let db = rusqlite::Connection::open(&f.config.database).unwrap();
    let mut query = db.prepare("SELECT s.phase,s.attempt_id,s.effect_id,s.started,s.observation_json,e.state FROM native_replacement_media_steps s JOIN operation_effects e ON e.id=s.effect_id WHERE s.operation_record_id=?1 AND s.phase IN ('attach','activate') ORDER BY s.phase").unwrap();
    let rows: Vec<(String, String, String, bool, String, String)> = query
        .query_map([record], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        rows.iter().map(|row| row.0.as_str()).collect::<Vec<_>>(),
        vec!["activate", "attach"]
    );
    assert!(rows.iter().all(|row| row.3 && row.5 == "confirmed"));
    rows
}
#[cfg(feature = "test-support")]
pub(super) fn assert_same_original_preparation(
    before: &OriginalPreparation,
    after: &OriginalPreparation,
) {
    assert_eq!(
        (
            &before.record_id,
            &before.reservation_id,
            before.expires_at,
            &before.candidate_connection_id,
            &before.candidate_runtime_id,
            before.generation
        ),
        (
            &after.record_id,
            &after.reservation_id,
            after.expires_at,
            &after.candidate_connection_id,
            &after.candidate_runtime_id,
            after.generation
        )
    );
}
struct Decoder {
    child: tokio::process::Child,
    input: tokio::process::ChildStdin,
    output: tokio::process::ChildStdout,
    pid: u32,
    executable_sha256: String,
    outputs: u32,
}
impl Decoder {
    fn open(encoder: &Encoder, cutoff: Instant) -> Self {
        assert!(
            Instant::now() < cutoff,
            "original replacement expired before decoder launch"
        );
        assert_eq!(file_sha256(&encoder.executable), encoder.executable_sha256);
        let mut child = tokio::process::Command::new(&encoder.executable)
            .arg("--decode-vp8")
            .env_clear()
            .env("LANG", "C")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let pid = child.id().unwrap();
        eprintln!(
            "replacement decoder started: pid={pid} executable_sha256={}",
            encoder.executable_sha256
        );
        Self {
            pid,
            input: child.stdin.take().unwrap(),
            output: child.stdout.take().unwrap(),
            child,
            executable_sha256: encoder.executable_sha256.clone(),
            outputs: 0,
        }
    }
    async fn frame(
        &mut self,
        frame: &NativeVideoFrame,
        index: u32,
        arrived: Instant,
        cutoff: Instant,
    ) {
        assert_eq!(frame.timestamp_rtp, index * 3000);
        assert!(frame.encoded.len() >= 3 && frame.encoded.len() <= 256 * 1024);
        assert_eq!(frame.keyframe, frame.encoded[0] & 1 == 0);
        let until = cutoff.min(arrived + Duration::from_millis(200));
        assert!(
            Instant::now() < until,
            "original received frame aged out before decoding"
        );
        let mut request = Vec::from(&b"D8F1"[..]);
        request.extend(index.to_le_bytes());
        request.extend((frame.encoded.len() as u32).to_le_bytes());
        request.extend(u32::from(frame.keyframe).to_le_bytes());
        request.extend(&frame.encoded);
        let mut reply = [0u8; 32];
        timeout_at(until, async {
            self.input.write_all(&request).await.unwrap();
            self.output.read_exact(&mut reply).await.unwrap();
        })
        .await
        .expect("actual original VP8 decoder exceeded received frame age");
        let word = |at| u32::from_le_bytes(reply[at..at + 4].try_into().unwrap());
        assert_eq!(&reply[..4], b"V8D1");
        assert_eq!(word(4), index);
        assert_eq!(word(8), 1);
        assert_eq!(word(12), u32::from(frame.keyframe));
        assert_eq!((word(16), word(20)), (640, 360));
        assert_eq!(word(28), 256);
        assert!(
            (i64::from(word(24)) - i64::from(60 + 3 * index) * 256).abs() <= 3 * 256,
            "actual retained decoder source marker differs"
        );
        self.outputs += 1;
        eprintln!(
            "replacement decoder output: pid={} ordinal={index} keyframe={} encoded_sha256={:x} luma_sum={} outputs={} receive_to_decode_us={}",
            self.pid,
            frame.keyframe,
            Sha256::digest(&frame.encoded),
            word(24),
            self.outputs,
            arrived.elapsed().as_micros()
        );
    }
    async fn close(self, cutoff: Instant) -> bool {
        let Self {
            mut child,
            mut input,
            mut output,
            pid,
            executable_sha256,
            outputs,
        } = self;
        let mut extra = [0u8; 1];
        let stopped = timeout_at(cutoff, async {
            input.shutdown().await?;
            drop(input);
            let trailing = output.read(&mut extra).await?;
            let status = child.wait().await?;
            Ok::<_, std::io::Error>((trailing, status))
        })
        .await;
        match stopped {
            Ok(Ok((trailing, status))) => {
                eprintln!(
                    "replacement decoder joined: pid={} forced=false status={} trailing={} outputs={} executable_sha256={}",
                    pid, status, trailing, outputs, executable_sha256
                );
                status.success() && trailing == 0 && outputs == 2
            }
            error => {
                let kill = child.start_kill();
                let joined = child.wait().await;
                eprintln!(
                    "replacement decoder joined after failure: pid={} forced=true original={error:?} kill={kill:?} joined={joined:?}",
                    pid
                );
                false
            }
        }
    }
}
async fn audio(
    io: &mut NativeVideoDatagrams,
    preparation: &NativeReplacementReadiness,
    cutoff: Instant,
) -> u64 {
    use rvoip_uctp::substrate::{
        observe_native_media_datagram, pack, MediaDatagram, ObservedNativeMediaDatagram,
    };
    let pt = preparation.audio().payload_type;
    let codec = match pt {
        0 => json!({"name":"pcmu","params":{"sample_rate":8000,"channels":1,"payload_type":0}}),
        111 => {
            json!({"name":"opus","params":{"sample_rate":48000,"channels":1,"payload_type":111}})
        }
        _ => panic!("actual fixture codec differs from original selected PCMU/Opus"),
    };
    let mut decoder = readiness_audio::Decoder::selected(&codec).unwrap();
    let payload = decoder.encoded_silence().unwrap();
    let quantum = decoder.rate() / 50;
    let mut tick = tokio::time::interval(Duration::from_millis(20));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut sequence = 0u16;
    timeout_at(cutoff,async {
        while !decoder.usable() {
            tokio::select! {
                _=tick.tick()=>{
                    sequence=sequence.wrapping_add(1);
                    io.send(&pack(&MediaDatagram{flags:0,stream_local_id:preparation.audio().stream_local_id,
                        seq:u32::from(sequence),payload:rtp(pt,sequence,u32::from(sequence)*quantum,0x72727272,false,&payload).into()})).unwrap();
                }
                received=io.receive()=>{
                    let packet=received.unwrap();
                    assert!(packet.video_epoch.is_none(),"no private source video has been published yet");
                    let ObservedNativeMediaDatagram::Rtp(packet)=observe_native_media_datagram(&packet.bytes).unwrap() else{continue;};
                    if packet.stream_local_id!=preparation.audio().stream_local_id {continue;}
                    let packet=packet.rtp.into_packet();assert_eq!(packet.header.payload_type,pt);
                    decoder.decode(packet.header.ssrc,packet.header.sequence_number,packet.header.timestamp,&packet.payload).unwrap();
                }
            }
        }
    }).await.expect("original private audio did not actually decode before expiry");
    assert!(sequence > 0);
    assert!(decoder.samples() >= u64::from(decoder.rate()) * 60 / 1000);
    decoder.samples()
}
async fn video(
    io: &mut NativeVideoDatagrams,
    handle: u16,
    index: u32,
    cutoff: Instant,
) -> (NativeVideoFrame, sdk::VideoReceiveEpoch, Instant) {
    timeout_at(cutoff, async {
        let mut assembler = VideoFrameAssembler::default();
        let mut history = None;
        for _ in 0..4096 {
            let packet = io.receive().await.unwrap();
            let Some(epoch) = packet.video_epoch else {
                continue;
            };
            assert_eq!(
                u16::from_be_bytes(packet.bytes[2..4].try_into().unwrap()),
                handle
            );
            let rtp = NativeVideoPacket::parse(packet.bytes[24..].to_vec()).unwrap();
            assert_eq!(rtp.payload_type, 96);
            assert_eq!(rtp.timestamp_rtp, index * 3000);
            assert_eq!(rtp.ssrc, epoch.ssrc);
            assert!(io.is_current_video_receive_epoch(handle, &epoch).unwrap());
            if let Some(old) = &history {
                assert_eq!(old, &epoch);
            } else {
                history = Some(epoch);
            }
            if let Some(frame) = assembler.push(&rtp).unwrap() {
                return (frame, history.unwrap(), packet.received_at);
            }
        }
        panic!("bounded actual private/public video frame was not complete")
    })
    .await
    .expect("original received video exceeded original replacement deadline")
}
fn scope(command: &TransitionCommand) -> CommandScope {
    CommandScope {
        conversation_id: Some(command.conversation_id.clone()),
        session_id: Some(command.session_id.clone()),
        connection_id: command.connection_id.clone(),
        ..Default::default()
    }
}
async fn submit(
    client: &mut CanonicalClient,
    ready: &NativeReplacementReady,
    controller: &mut CanonicalHttpClient,
    command: &TransitionCommand,
    root: &connect_client::canonical::payloads::OperationUpdated,
    cutoff: Instant,
) {
    for attempt in 0..8 {
        assert!(
            Instant::now() < cutoff,
            "original replacement expired before immutable ready"
        );
        match timeout_at(cutoff, client.confirm_native_replacement_ready(ready))
            .await
            .expect("original decoded ready deadline expired")
        {
            Ok(_) => return,
            Err(ClientError::Application(problem))
                if matches!(
                    problem.code.as_str(),
                    "authority-busy" | "storage-unavailable"
                ) && attempt < 7 =>
            {
                let status = timeout_at(
                    cutoff,
                    controller.operation_get(&command.operation_id, scope(command)),
                )
                .await
                .unwrap()
                .unwrap();
                assert_eq!(status.record_id, root.record_id);
                assert_eq!(status.operation_id, root.operation_id);
                let state = serde_json::to_value(status).unwrap();
                assert!(matches!(
                    state["state"].as_str(),
                    Some("preparing" | "ready" | "committing" | "completed")
                ));
                // Same original frozen observation, never another allocation.
                timeout_at(cutoff, tokio::time::sleep(Duration::from_millis(20)))
                    .await
                    .unwrap();
            }
            Err(error) => panic!("original physical decoded readiness refused: {error}"),
        }
    }
    unreachable!()
}
pub(super) async fn activate(
    f: &Fixture,
    controller: &mut CanonicalHttpClient,
    command: &TransitionCommand,
    root: &connect_client::canonical::payloads::OperationUpdated,
    client: &mut CanonicalClient,
    io: &mut NativeVideoDatagrams,
    encoder: &mut Encoder,
    source: &sdk::Stream,
    sequence: &mut u16,
    voice: u16,
    incoming: u16,
    camera: u16,
    publisher_receiver: &sdk::Stream,
    target_sequence: &mut u16,
    cutoff: Instant,
) -> (Value, sdk::VideoReceiveEpoch) {
    let preparation = client.native_replacement_readiness().unwrap();
    assert_eq!(preparation.audio().stream_local_id, voice);
    assert_eq!(preparation.video().unwrap().stream_local_id, incoming);
    assert_eq!(
        (
            preparation.source_generation(),
            preparation.target_generation(),
            preparation.stream_revision()
        ),
        (1, 2, 2)
    );
    io.reset_video_receive_decoder(incoming).unwrap();
    let mut decoder = Decoder::open(encoder, cutoff);
    let original_pid = decoder.pid;
    // Join the original decoder before propagating any assertion/timeout from
    // this controlled phase, including an actual failure before ready capture.
    let work=AssertUnwindSafe(async {
        timeout_at(cutoff,async {
            let samples=audio(io,&preparation,cutoff).await;
            // Actual private output means preparation has reached its media
            // phase; publish a fresh source keyframe on that original route.
            let key=encoder.frame(1,true).await;assert_eq!(key[0]&1,0);
            publish(source,&key,1,sequence).await;
            let (frame,private_epoch,arrived)=video(io,incoming,1,cutoff).await;
            assert_eq!(frame.encoded,key);assert!(frame.keyframe);
            decoder.frame(&frame,1,arrived,cutoff).await;
            assert!(io.is_current_video_receive_epoch(incoming,&private_epoch).unwrap());
            let before=controller.operation_get(&command.operation_id,scope(command)).await.unwrap();
            assert_eq!(before.record_id,root.record_id);
            assert_eq!(serde_json::to_value(before).unwrap()["state"],"preparing",
                "actual private decoder output cannot commit without signed proof");
            let ready=preparation.with_decoded(samples,Some(NativeDecodedVideoReadiness{decoded_frames:1,decoded_keyframes:1})).unwrap();
            submit(client,&ready,controller,command,root,cutoff).await;
            let done=completed(f,controller,command).await;
            assert_eq!(done.record_id,root.record_id);
            assert_eq!(done.operation_id,root.operation_id);
            let done=serde_json::to_value(done).unwrap();assert_eq!(done["state"],"completed");
            assert_eq!(done["result"]["connection_id"],preparation.connection_id().unwrap());
            assert_eq!(done["result"]["generation"],2);
            // Deliberately interleave the other actual current publisher before
            // first public downlink. This used to prune the retired private
            // receiver registry row and mint a different decoder revision.
            assert!(!io.take_keyframe_request(camera,0x74657374).unwrap(),"no cached publisher hint may qualify");
            raw_publish(io,camera,&key,1,target_sequence);
            let (uplink,uplink_epoch)=receive(publisher_receiver,1).await;
            assert_eq!(uplink,key);decode(f.temp.path(),&uplink,1).await;
            // An actually received PLI must match the target's current SIGNED
            // publisher notice; remote decode alone cannot prove that notice.
            let metadata_until=cutoff.min(Instant::now()+Duration::from_secs(2));let mut observed=false;
            for _ in 0..8 {
                let requested=Instant::now();assert!(requested<metadata_until);
                timeout_at(metadata_until,publisher_receiver.request_keyframe(options())).await.unwrap().unwrap();
                let until=metadata_until.min(requested+Duration::from_millis(100));
                loop {
                    let read=timeout_at(until,io.receive()).await;
                    if let Ok(result)=read {result.unwrap();}
                    if io.take_keyframe_request(camera,0x74657374).unwrap(){observed=true;break;}
                    if Instant::now()>=until{break;}
                }
                if observed {break;}
            }
            assert!(observed,"actual target publisher signed metadata/feedback did not become current");
            assert!(!io.take_keyframe_request(camera,0x74657374).unwrap(),"original hint must be consumed once");
            // Preserve the shared source's existing 100ms feedback limit before
            // the later cold-decoder control. This consumes the SAME cutoff.
            timeout_at(cutoff,tokio::time::sleep(Duration::from_millis(100))).await.unwrap();
            eprintln!("replacement public publisher interleave: remote_key_decoded=true current_signed_publisher_hint=true consumed_once=true receiver_epoch={} receiver_revision={}",uplink_epoch.epoch,uplink_epoch.revision);
            // Decoder and actual signed receive epoch remain unchanged across
            // commit. A real interframe, not another keyframe, must decode.
            let delta=encoder.frame(2,false).await;assert_eq!(delta[0]&1,1);
            publish(source,&delta,2,sequence).await;
            let (frame,public_epoch,arrived)=video(io,incoming,2,cutoff).await;
            assert_eq!(frame.encoded,delta);assert!(!frame.keyframe);
            assert_eq!(public_epoch,private_epoch,"original native decoder epoch changed at commit");
            assert_eq!(decoder.pid,original_pid);
            decoder.frame(&frame,2,arrived,cutoff).await;
            assert_eq!(decoder.outputs,2);
            eprintln!("replacement private/public decode: pid={original_pid} audio_samples={samples} same_epoch=true same_decoder=true original_private_rtp=3000 original_public_interframe_rtp=6000");
            (done, public_epoch)
        }).await.expect("original replacement decoder phase exceeded its original cutoff")
    }).catch_unwind().await;
    let joined = decoder.close(cutoff).await;
    let done = match work {
        Ok(value) => value,
        Err(panic) => std::panic::resume_unwind(panic),
    };
    assert!(
        joined,
        "original retained VP8 decoder did not join naturally"
    );
    // This is an explicit application-decoder restart for the existing recovery
    // test, not a production commit reset or a changed server/source epoch.
    io.reset_video_receive_decoder(incoming).unwrap();
    done
}

// The source is already running: the candidate receives a genuine interframe
// first. No private keyframe is encoded until its actual current PLI is received
// by the retained source. This happens before any target audio is sent and
// before signed decoded readiness/ordinary public ownership can exist.
pub(super) async fn private_bootstrap(
    f: &Fixture,
    controller: &mut CanonicalHttpClient,
    command: &TransitionCommand,
    root: &connect_client::canonical::payloads::OperationUpdated,
    client: &mut CanonicalClient,
    io: &mut NativeVideoDatagrams,
    encoder: &mut Encoder,
    source: &sdk::Stream,
    sequence: &mut u16,
    incoming: u16,
    original_cutoff: Instant,
    lose_first_hint: bool,
    mode: u8,
    original: &OriginalPreparation,
    old_receiver: &sdk::Stream,
) -> Option<NativeReplacementReady> {
    let cutoff = original_cutoff.min(Instant::now() + Duration::from_secs(8));
    let preparation = client.native_replacement_readiness().unwrap();
    assert_eq!(preparation.video().unwrap().stream_local_id, incoming);
    io.reset_video_receive_decoder(incoming).unwrap();
    let mut decoder = Decoder::open(encoder, cutoff);
    let pid = decoder.pid;
    let work = AssertUnwindSafe(async {
        timeout_at(cutoff, async {
            if mode == 13 {
                // Consume the SAME eight-second decoder phase and original
                // reservation; do not renew a credential, root or cutoff.
                let after_original_start_read = Instant::now() + Duration::from_millis(5100);
                assert!(after_original_start_read < cutoff);
                tokio::time::sleep_until(after_original_start_read).await;
                assert_original_preparation(f, original);
                assert_unpublished(f, original);
                let status = controller.operation_get(&command.operation_id, scope(command)).await.unwrap();
                assert_eq!(status.record_id, root.record_id);
                assert_eq!(serde_json::to_value(status).unwrap()["state"], "preparing");
            }
            assert!(matches!(source.receive_video_feedback(
                sdk::RequestOptions::new(Duration::from_millis(30)).unwrap(),
            ).await, Err(error) if error.kind == sdk::ErrorKind::Deadline),
                "a cached source hint cannot qualify private bootstrap");
            let delta = encoder.frame(1, false).await;
            let delta_before_key = delta.clone();
            assert_eq!(delta[0] & 1, 1);
            publish(source, &delta, 1, sequence).await;
            // Keep the SAME native reader polled even after the delta frame
            // completes while its fresh source hint is still pending.
            let mut assembler = VideoFrameAssembler::default();
            let mut captured = None;
            let mut first = None;
            let mut history = None;
            let pending_hint = source.receive_video_feedback(options());
            tokio::pin!(pending_hint);
            loop {
                tokio::select! {
                    hint = &mut pending_hint, if first.is_none() => { first = Some(hint.unwrap()); },
                    packet = io.receive() => {
                        let packet = packet.unwrap();
                        if let Some(epoch) = packet.video_epoch {
                            assert_eq!(u16::from_be_bytes(packet.bytes[2..4].try_into().unwrap()), incoming);
                            let rtp = NativeVideoPacket::parse(packet.bytes[24..].to_vec()).unwrap();
                            assert_eq!((rtp.payload_type, rtp.timestamp_rtp, rtp.ssrc), (96, 3000, epoch.ssrc));
                            assert!(io.is_current_video_receive_epoch(incoming, &epoch).unwrap());
                            if let Some(old) = &history { assert_eq!(old, &epoch); } else { history = Some(epoch.clone()); }
                            if let Some(frame) = assembler.push(&rtp).unwrap() {
                                assert!(captured.replace((frame, epoch, packet.received_at)).is_none());
                            }
                        }
                    },
                }
                if captured.is_some() && first.is_some() { break; }
            }
            let (frame, epoch, _) = captured.unwrap();
            let first_hint = first.unwrap();
            assert_eq!(frame.encoded, delta); assert!(!frame.keyframe);
            assert_eq!(first_hint.ssrc(), 0x74657374);
            assert!(source.video_feedback_is_current(&first_hint).unwrap());
            let first_sequence = first_hint.datagram_sequence();
            let hint = if lose_first_hint {
                // Intentionally lose ONE actual source hint. Keep the same
                // original receiver polling while NO additional video frame
                // exists. No request, frame, timestamp or root is replayed.
                let expired = Instant::now() + first_hint.expires_in() + Duration::from_millis(5);
                loop {
                    tokio::select! {biased;
                        _ = tokio::time::sleep_until(expired) => break,
                        packet = io.receive() => {
                            assert!(packet.unwrap().video_epoch.is_none(), "source video was not withheld");
                        },
                    }
                }
                assert!(!source.video_feedback_is_current(&first_hint).unwrap());
                drop(first_hint);
                let next = source.receive_video_feedback(options());
                tokio::pin!(next);
                let hint = loop {
                    tokio::select! {biased;
                        hint = &mut next => break hint.unwrap(),
                        packet = io.receive() => {
                            assert!(packet.unwrap().video_epoch.is_none(), "another video frame rescued bootstrap");
                        },
                    }
                };
                assert!(hint.datagram_sequence().wrapping_sub(first_sequence) > 0);
                hint
            } else { first_hint };
            assert!(source.video_feedback_is_current(&hint).unwrap());
            assert!(hint.expires_in() > Duration::ZERO && hint.expires_in() <= Duration::from_millis(200));
            // Fresh current hint is the only reason to force this keyframe.
            // The preceding interframe was never submitted to the cold decoder.
            let key = encoder.frame(2, source.video_feedback_is_current(&hint).unwrap()).await;
            assert_eq!(key[0] & 1, 0);
            drop(hint);
            publish(source, &key, 2, sequence).await;
            let (frame, private_epoch, arrived) = video(io, incoming, 2, cutoff).await;
            assert_eq!(private_epoch, epoch); assert_eq!(frame.encoded, key); assert!(frame.keyframe);
            decoder.frame(&frame, 2, arrived, cutoff).await;
            assert_eq!(decoder.outputs, 1);
            let before = controller.operation_get(&command.operation_id, scope(command)).await.unwrap();
            assert_eq!(before.record_id, root.record_id);
            assert_eq!(serde_json::to_value(before).unwrap()["state"], "preparing");
            let confirmed: u64 = rusqlite::Connection::open(&f.config.database).unwrap().query_row(
                "SELECT count(*) FROM native_replacement_media_steps WHERE operation_record_id=?1 AND phase='readiness' AND observation_json IS NOT NULL",
                [root.record_id.as_ref().unwrap().as_str()], |row| row.get(0),
            ).unwrap();
            assert_eq!(confirmed, 0, "bootstrap hint inferred decoded readiness");
            // Only AFTER genuine private video decoding do we pump target
            // audio and build the unchanged signed private readiness proof.
            let samples = audio(io, &preparation, cutoff).await;
            let ready = preparation.with_decoded(samples, Some(NativeDecodedVideoReadiness {
                decoded_frames: 1, decoded_keyframes: 1,
            })).unwrap();
            if mode >= 14 {
                assert_original_preparation(f, original);
                assert_unpublished(f, original);
                // Join the same real decoder before waiting for a later
                // boundary. Two PRIVATE outputs satisfy its unchanged natural
                // join contract; neither is public activation evidence.
                let delta = encoder.frame(3, false).await; assert_eq!(delta[0] & 1, 1);
                publish(source, &delta, 3, sequence).await;
                let (frame, same, arrived) = video(io, incoming, 3, cutoff).await;
                assert_eq!(same, private_epoch); assert_eq!(frame.encoded, delta);
                decoder.frame(&frame, 3, arrived, cutoff).await;
                assert_eq!(decoder.outputs, 2);
                // The unchanged old receive route got these same original
                // markers too. Verify/drain them BEFORE the boundary, so its
                // later fresh marker4 check cannot accidentally read old RTP.
                for (index, expected) in [(1, &delta_before_key), (2, &key), (3, &delta)] {
                    let (bytes, _) = receive(old_receiver, index).await;
                    assert_eq!(&bytes, expected);
                }
                return Some(ready);
            }
            submit(client, &ready, controller, command, root, cutoff).await;
            let done = completed(f, controller, command).await;
            assert_eq!(done.record_id, root.record_id); assert_eq!(done.operation_id, root.operation_id);
            let done = serde_json::to_value(done).unwrap();
            assert_eq!(done["state"], "completed"); assert_eq!(done["result"]["generation"], 2);
            let delta = encoder.frame(3, false).await; assert_eq!(delta[0] & 1, 1);
            publish(source, &delta, 3, sequence).await;
            let (frame, public_epoch, arrived) = video(io, incoming, 3, cutoff).await;
            assert_eq!(public_epoch, private_epoch); assert_eq!(frame.encoded, delta);
            assert_eq!(decoder.pid, pid); decoder.frame(&frame, 3, arrived, cutoff).await;
            assert_eq!(decoder.outputs, 2);
            eprintln!("private delta bootstrap: first_hint_lost={lose_first_hint} genuine_private_hint=true key_only_after_hint=true video_before_target_audio=true same_root=true same_epoch=true same_decoder=true");
            assert_original_preparation(f, original);
            None
        }).await.expect("private bootstrap exceeded the unchanged original eight-second/captured preparation bound")
    }).catch_unwind().await;
    let joined = decoder.close(cutoff).await;
    let frozen = match work {
        Ok(frozen) => frozen,
        Err(panic) => std::panic::resume_unwind(panic),
    };
    assert!(
        joined,
        "original private bootstrap decoder did not join naturally"
    );
    frozen
}

pub(super) async fn wait_for_original_expiry(original: &OriginalPreparation) {
    // The projected monotonic cutoff bounds pre-expiry media work. The
    // Store expires this reservation against Unix wall time. Confirm that
    // same boundary before testing the original operation after expiry.
    tokio::time::sleep_until(original.cutoff).await;
    while chrono::Utc::now().timestamp() < original.expires_at {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn assert_original_preparation(f: &Fixture, original: &OriginalPreparation) {
    let actual: (String, i64, String, String, u64) = rusqlite::Connection::open(&f.config.database).unwrap()
        .query_row("SELECT reservation_id,expires_at,candidate_connection_id,candidate_runtime_id,generation FROM native_replacements WHERE operation_record_id=?1",
            [&original.record_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
    assert_eq!(
        actual,
        (
            original.reservation_id.clone(),
            original.expires_at,
            original.candidate_connection_id.clone(),
            original.candidate_runtime_id.clone(),
            original.generation
        )
    );
}
fn assert_unpublished(f: &Fixture, original: &OriginalPreparation) {
    let c = rusqlite::Connection::open(&f.config.database).unwrap();
    let ready: u64 = c
        .query_row(
            "SELECT count(*) FROM journal WHERE kind='connection_ready' AND operation_record_id=?1",
            [&original.record_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        ready, 0,
        "original private decoder output cannot publish Ready"
    );
    let live: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM connections WHERE id=?1 AND state IN ('ready','active'))",
            [&original.candidate_connection_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        !live,
        "original target must never become ordinary live media"
    );
    let observed: u64 = c.query_row("SELECT count(*) FROM native_replacement_media_steps WHERE operation_record_id=?1 AND phase='readiness' AND observation_json IS NOT NULL",
        [&original.record_id],|r|r.get(0)).unwrap();
    assert_eq!(
        observed, 0,
        "genuine private decoding is not a submitted signed Ready"
    );
}
async fn no_new_private_frame(
    io: &mut NativeVideoDatagrams,
    source: &sdk::Stream,
    old_receiver: &sdk::Stream,
    encoder: &mut Encoder,
    sequence: &mut u16,
) {
    // This new original frame did not exist before expiry/revocation/cancel.
    // The original old receiver must still work; queued earlier bytes are not
    // silently called post-boundary authorization.
    let key = encoder.frame(4, true).await;
    publish(source, &key, 4, sequence).await;
    let (old, _) = receive(old_receiver, 4).await;
    assert_eq!(old, key, "ordinary old connection must remain usable");
    let _ = timeout_at(Instant::now() + Duration::from_millis(300), async {
        loop {
            match io.receive().await {
                Ok(packet) if packet.video_epoch.is_some() => {
                    let frame = NativeVideoPacket::parse(packet.bytes[24..].to_vec()).unwrap();
                    assert_ne!(
                        frame.timestamp_rtp,
                        4 * 3000,
                        "new video crossed a failed original authority boundary"
                    );
                }
                Ok(_) => {}
                Err(ClientError::Transport | ClientError::MediaEnded | ClientError::Cancelled) => {
                    break
                }
                Err(error) => panic!("unexpected original private reader error: {error}"),
            }
        }
    })
    .await;
}
#[cfg(feature = "test-support")]
pub(super) async fn refuse_original_boundary(
    f: &Fixture,
    command: &TransitionCommand,
    original: &OriginalPreparation,
    origin: &connect_store::NativeReplacementOrigin,
    client: &mut CanonicalClient,
    ready: &NativeReplacementReady,
    io: &mut NativeVideoDatagrams,
    encoder: &mut Encoder,
    source: &sdk::Stream,
    old_receiver: &sdk::Stream,
    sequence: &mut u16,
    operator_credential: &str,
    mode: u8,
) {
    assert!(matches!(mode, 14 | 15));
    assert_original_preparation(f, original);
    assert_unpublished(f, original);
    if mode == 14 {
        wait_for_original_expiry(original).await;
    } else {
        assert!(Instant::now() < original.cutoff);
        let admin = f.server.store.authenticate(&f.admin).unwrap();
        let store = f.server.store.clone();
        let credential = operator_credential.to_owned();
        tokio::task::spawn_blocking(move || store.revoke(&admin, &credential))
            .await
            .unwrap()
            .unwrap();
        assert!(f.server.store.revalidate(operator_credential).is_err());
    }
    // This exact original metadata origin grants no IO. Its fresh Store read
    // must refuse independently of a concurrent physical teardown.
    assert_eq!(origin.record_id(), original.record_id);
    let current = f
        .server
        .store
        .native_replacement_context(origin)
        .err()
        .expect("expired/revoked original Store authority remained current");
    assert!(
        if mode == 15 {
            matches!(current.code.as_str(), "unauthorized" | "forbidden")
        } else {
            current.code == "forbidden"
        },
        "{current}"
    );
    // Same actual target, same frozen decoded observation and root. A local
    // refusal qualifies only with the original terminal record AND exact Core
    // retirement; arbitrary transport uncertainty is never sufficient.
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        client.confirm_native_replacement_ready(ready),
    )
    .await
    .expect("original signed Ready refusal did not settle");
    match result {
        Err(ClientError::Application(problem)) => {
            // Revoked original operator resolves as unauthorized; a concurrent
            // genuine root retirement may already refuse the live preparation.
            let refused = if mode == 15 {
                matches!(problem.code.as_str(),"unauthorized"|"forbidden"|"not-ready")
            } else {
                matches!(problem.code.as_str(),"forbidden"|"not-ready"|"expired")
            };
            assert!(refused,"{problem}");
        },
        Err(ClientError::Protocol|ClientError::Transport|ClientError::MediaEnded) => {
            original_target_retired(f, original).await;
        },
        Err(error) => panic!("expected actual current-authority refusal or proven original target retirement, got {error}"),
        Ok(_) => panic!("expired or revoked original authority accepted signed Ready"),
    }
    no_new_private_frame(io, source, old_receiver, encoder, sequence).await;
    assert_original_preparation(f, original);
    assert_unpublished(f, original);
    let (operation, state): (String, String) = rusqlite::Connection::open(&f.config.database)
        .unwrap()
        .query_row(
            "SELECT operation_id,state FROM operations WHERE record_id=?1",
            [&original.record_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(operation, command.operation_id);
    assert!(!matches!(state.as_str(), "ready" | "committing"));
    if state == "completed" {
        original_target_retired(f, original).await;
    }
}
#[cfg(feature = "test-support")]
pub(super) async fn refuse_cancelled_target(
    f: &Fixture,
    command: &TransitionCommand,
    original: &OriginalPreparation,
    origin: &connect_store::NativeReplacementOrigin,
    io: &mut NativeVideoDatagrams,
    encoder: &mut Encoder,
    source: &sdk::Stream,
    old_receiver: &sdk::Stream,
    sequence: &mut u16,
) {
    // Caller has naturally joined the exact original physical target. Never
    // open a successor to submit a continuation for this cancelled owner.
    assert!(Instant::now() < original.cutoff);
    original_target_retired(f, original).await;
    let current = f
        .server
        .store
        .native_replacement_context(origin)
        .err()
        .expect("cancelled original Store authority remained current");
    assert_eq!(current.code, "forbidden");
    no_new_private_frame(io, source, old_receiver, encoder, sequence).await;
    assert_original_preparation(f, original);
    assert_unpublished(f, original);
    let operation: String = rusqlite::Connection::open(&f.config.database)
        .unwrap()
        .query_row(
            "SELECT operation_id FROM operations WHERE record_id=?1",
            [&original.record_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(operation, command.operation_id);
}

#[cfg(feature = "test-support")]
async fn original_target_retired(f: &Fixture, original: &OriginalPreparation) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let (state, result): (String, Option<String>) =
                rusqlite::Connection::open(&f.config.database)
                    .unwrap()
                    .query_row(
                        "SELECT state,result_json FROM operations WHERE record_id=?1",
                        [&original.record_id],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .unwrap();
            assert!(!matches!(state.as_str(), "ready" | "committing"));
            let stopped = state == "completed"
                && result.as_ref().is_some_and(|raw| {
                    let value: serde_json::Value = serde_json::from_str(raw).unwrap();
                    value["activation_state"] == "stopped" && value["readiness"].is_null()
                });
            assert!(
                state != "completed" || stopped,
                "successful activation cannot qualify retirement"
            );
            let absent = {
                #[cfg(feature = "test-support")]
                {
                    let session = f
                        .server
                        .service
                        .canonical_session_metadata_for_test("native-session")
                        .unwrap();
                    let held = session.read().unwrap();
                    !held
                        .connections
                        .contains_key(&rvoip_core::ids::ConnectionId::from_string(
                            original.candidate_runtime_id.clone(),
                        ))
                }
                #[cfg(not(feature = "test-support"))]
                {
                    panic!("exact original Core retirement control requires test-support");
                }
            };
            if (matches!(state.as_str(), "failed" | "cancelled") || stopped) && absent {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("original target terminal record and exact Core retirement did not join");
}
