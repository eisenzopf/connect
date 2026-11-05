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
pub(super) fn original_cutoff(
    f: &Fixture,
    root: &connect_client::canonical::payloads::OperationUpdated,
) -> Instant {
    let expires: i64 = rusqlite::Connection::open(&f.config.database)
        .unwrap()
        .query_row(
            "SELECT expires_at FROM operations WHERE record_id=?1",
            [root.record_id.as_ref().unwrap().as_str()],
            |r| r.get(0),
        )
        .unwrap();
    let wall = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    let remaining = Duration::from_secs(u64::try_from(expires).unwrap())
        .checked_sub(wall)
        .expect("original replacement expiry already passed");
    Instant::now() + remaining
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
    assert_eq!(preparation.audio().payload_type, 0);
    let mut decoder = readiness_audio::Decoder::selected(
        &json!({"name":"pcmu","params":{"sample_rate":8000,"channels":1,"payload_type":0}}),
    )
    .unwrap();
    let mut tick = tokio::time::interval(Duration::from_millis(20));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut sequence = 0u16;
    timeout_at(cutoff,async {
        while !decoder.usable() {
            tokio::select! {
                _=tick.tick()=>{
                    sequence=sequence.wrapping_add(1);
                    io.send(&pack(&MediaDatagram{flags:0,stream_local_id:preparation.audio().stream_local_id,
                        seq:u32::from(sequence),payload:rtp(0,sequence,u32::from(sequence)*160,0x72727272,false,&[0xff;160]).into()})).unwrap();
                }
                received=io.receive()=>{
                    let packet=received.unwrap();
                    assert!(packet.video_epoch.is_none(),"no private source video has been published yet");
                    let ObservedNativeMediaDatagram::Rtp(packet)=observe_native_media_datagram(&packet.bytes).unwrap() else{continue;};
                    if packet.stream_local_id!=preparation.audio().stream_local_id {continue;}
                    let packet=packet.rtp.into_packet();assert_eq!(packet.header.payload_type,0);
                    decoder.decode(packet.header.ssrc,packet.header.sequence_number,packet.header.timestamp,&packet.payload).unwrap();
                }
            }
        }
    }).await.expect("original private audio did not actually decode before expiry");
    assert!(sequence > 0);
    assert!(decoder.samples() >= 480);
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
    cutoff: Instant,
) -> Value {
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
            let done=phase(f,controller,command,7).await;
            assert_eq!(done.record_id,root.record_id);
            assert_eq!(done.operation_id,root.operation_id);
            let done=serde_json::to_value(done).unwrap();assert_eq!(done["state"],"completed");
            assert_eq!(done["result"]["connection_id"],preparation.connection_id().unwrap());
            assert_eq!(done["result"]["generation"],2);
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
            done
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
