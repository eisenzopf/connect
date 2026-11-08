//! Actual receiver observations on the original signed private preparation.
//! No RTP arrival, input publication or negotiation ACK is decoded readiness.
use super::*;
use connect_client::{
    canonical::{NativeDecodedVideoReadiness, NativeReplacementReadiness, NativeReplacementReady},
    ClientError, NativeVideoPacket, VideoFrameAssembler,
};
use rvoip_uctp::substrate::{
    observe_native_media_datagram, pack, pack_rtp_datagram, MediaDatagram,
    ObservedNativeMediaDatagram, RtpDatagram, RtpMediaPayload,
};
use tokio::time::{timeout_at, Instant};
#[path = "decoded_readiness/audio.rs"]
mod audio;
#[cfg(feature = "test-support")]
#[path = "decoded_readiness/cadence.rs"]
pub(super) mod cadence;
#[cfg(test)]
#[path = "decoded_readiness/tests.rs"]
mod tests;
#[path = "decoded_readiness/video.rs"]
mod video;
#[path = "decoded_readiness/video_tail.rs"]
mod video_tail;
pub(crate) use video_tail::VideoTail;

pub(super) struct Prepared {
    cutoff: Instant,
    readiness: NativeReplacementReadiness,
    audio: audio::Decoder,
}
pub(super) struct VideoSource<'a> {
    pub client: &'a CanonicalClient,
    pub handle: u16,
    /// One actual original publisher counter, continued through paused/committed proofs.
    pub next_sequence: &'a mut u16,
}
/// Convert the original wall-clock expiry once. Neither decode nor retry grants
/// another operation budget; an absent/expired original deadline is a failure.
pub(super) fn original_cutoff(record: &OperationRecord) -> Instant {
    let expires = record
        .expires_at
        .expect("replacement has an original expiry");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    let end = Duration::from_secs(u64::try_from(expires).unwrap());
    let remaining = end
        .checked_sub(now)
        .expect("original replacement already expired");
    Instant::now() + remaining
}
/// The public SDK validates both independently signed negotiation legs. Keep
/// that exact selected codec and challenge for the later original decoder.
pub(super) async fn answer(
    client: &mut CanonicalClient,
    callback: &Envelope,
    scope: &CommandScope,
    cutoff: Instant,
) -> Prepared {
    assert!(
        Instant::now() < cutoff,
        "original replacement expired before parameter answer"
    );
    let answer = client.native_replacement_answer(callback).unwrap();
    timeout_at(
        cutoff,
        client.answer_connection_offer(callback, answer, scope),
    )
    .await
    .expect("original replacement expired before its parameter answer")
    .unwrap();
    let readiness = client.native_replacement_readiness().unwrap();
    assert_eq!(readiness.connection_id(), scope.connection_id.as_deref());
    assert_eq!(readiness.operation_id(), scope.operation_id.as_deref());
    assert_eq!(
        readiness.target_generation(),
        readiness.source_generation() + 1
    );
    assert_eq!(readiness.stream_revision(), 2);
    let streams = callback.original()["payload"]["substrate_setup"]["streams"]
        .as_array()
        .unwrap();
    let audio = streams
        .iter()
        .find(|s| s["strm_id"] == readiness.audio().stream_id)
        .unwrap();
    assert_eq!(audio["stream_local_id"], readiness.audio().stream_local_id);
    assert_eq!(
        audio["codec"]["params"]["payload_type"],
        readiness.audio().payload_type
    );
    let audio = audio::Decoder::selected(&audio["codec"]).expect("actual negotiated audio decoder");
    if let Some(video) = readiness.video() {
        let selected = streams
            .iter()
            .find(|s| s["strm_id"] == video.stream_id)
            .unwrap();
        assert_eq!(selected["kind"], "video");
        assert_eq!(selected["codec"]["name"], "vp8");
        assert_eq!(selected["codec"]["params"]["sample_rate"], 90000);
        assert_eq!(selected["stream_local_id"], video.stream_local_id);
        assert_eq!(
            selected["codec"]["params"]["payload_type"],
            video.payload_type
        );
        assert_eq!(video.payload_type, 96);
    }
    Prepared {
        cutoff,
        readiness,
        audio,
    }
}

/// Keep the real retained speaker alive while this original decoder drains,
/// observes and submits Ready. No task is spawned. Cancellation drops this
/// publisher with the same future, and the source sequence/clock are continued.
/// The setup validates the signed codec-voice PCMU8k binding and actual handle.
pub(super) async fn while_retained_audio<T>(
    source: &CanonicalClient,
    handle: u16,
    sequence: &mut u32,
    clock: Instant,
    cutoff: Instant,
    ready: impl std::future::Future<Output = T>,
) -> T {
    let mut tick = tokio::time::interval(Duration::from_millis(20));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    tokio::pin!(ready);
    timeout_at(cutoff, async {
        loop {
            tokio::select! {
                result = &mut ready => return result,
                _ = tick.tick() => {
                    *sequence = sequence.wrapping_add(1);
                    // Same original source and marked 20ms PCMU payload as the
                    // finite public probes; input alone never counts as Ready.
                    let packet = pack_rtp_datagram(&RtpDatagram {
                        flags: 0,
                        stream_local_id: handle,
                        seq: *sequence,
                        rtp: RtpMediaPayload {
                            payload: (0..160).map(|i| if i % 16 < 8 { 0x80 } else { 0x00 })
                                .collect::<Vec<_>>().into(),
                            payload_type: 0,
                            sequence_number: *sequence as u16,
                            timestamp: (clock.elapsed().as_micros() * 8 / 1000) as u32,
                            ssrc: 0x4736_8000,
                        },
                    }).unwrap();
                    source.send_native_media_datagram(&packet)
                        .expect("original retained source publication failed during private readiness");
                }
            }
        }
    })
    .await
    .expect("original replacement expired while its retained speaker remained live")
}

/// Retain this same physical client and frozen ready object across uncertainty.
/// Only the original controller reads the original source-scoped operation.
pub(super) async fn confirm(
    peer: &mut Peer,
    prepared: Prepared,
    publisher: Option<VideoSource<'_>>,
    controller: &mut CanonicalHttpClient,
    command: &TransitionCommand,
    root: &OperationRecord,
) -> NativeReplacementReady {
    confirm_with_video_tail(peer, prepared, publisher, controller, command, root)
        .await
        .0
}

pub(super) async fn confirm_with_video_tail(
    peer: &mut Peer,
    mut prepared: Prepared,
    mut publisher: Option<VideoSource<'_>>,
    controller: &mut CanonicalHttpClient,
    command: &TransitionCommand,
    root: &OperationRecord,
) -> (NativeReplacementReady, Option<VideoTail>) {
    let first_video_sequence = publisher.as_ref().map(|source| *source.next_sequence);
    assert_eq!(
        prepared.readiness.video().is_some(),
        publisher.is_some(),
        "only an explicit original incoming transfer mapping needs video readiness"
    );
    assert_eq!(peer.handle, prepared.readiness.audio().stream_local_id);
    // Earlier old-route probes may have queued private silence. Drain only this
    // original reader before beginning the fresh decoder; never turn a delayed
    // preparation queue into current decoded readiness.
    let drained = timeout_at(prepared.cutoff, async {
        for count in 0..4096u64 {
            match tokio::time::timeout(
                Duration::from_millis(1),
                peer.client.receive_native_media_datagram(),
            )
            .await
            {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => panic!("original private reader failed before decode: {error}"),
                Err(_) => return count,
            }
        }
        panic!("original preparation queue did not empty within its packet bound");
    })
    .await
    .expect("original replacement expired before fresh decoder observation");
    let input = prepared.audio.encoded_silence().unwrap();
    let mut tick = tokio::time::interval(Duration::from_millis(20));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut assembler = VideoFrameAssembler::default();
    let mut frame = None;
    let mut input_packets = 0u64;
    let mut audio_packets = 0u64;
    let mut video_packets = 0u64;
    let mut video_ssrc = None;
    let mut private_video = Vec::new();
    let observed = timeout_at(prepared.cutoff, async {
        while !prepared.audio.usable() || (publisher.is_some() && frame.is_none()) {
            tokio::select! {
                _ = tick.tick() => {
                    peer.sequence = peer.sequence.wrapping_add(1);
                    let packet = pack_rtp_datagram(&RtpDatagram {
                        flags: 0, stream_local_id: peer.handle, seq: peer.sequence,
                        rtp: RtpMediaPayload { payload: input.clone().into(),
                            payload_type: prepared.readiness.audio().payload_type,
                            sequence_number: peer.sequence as u16,
                            timestamp: (peer.clock.elapsed().as_micros() * u128::from(prepared.audio.rate()) / 1_000_000) as u32,
                            ssrc: 0x4736_8002 },
                    }).map_err(|e| format!("native readiness input: {e:?}"))?;
                    peer.client.send_native_media_datagram(&packet).map_err(|e| e.to_string())?;
                    input_packets += 1;
                    if frame.is_none() && input_packets % 2 == 1 {
                        if let Some(source) = publisher.as_mut() { send_video(source)?; }
                    }
                }
                received = peer.client.receive_native_media_datagram() => {
                    let bytes = received.map_err(|e| e.to_string())?;
                    let ObservedNativeMediaDatagram::Rtp(packet) = observe_native_media_datagram(&bytes)
                        .map_err(|e| format!("native readiness packet: {e:?}"))? else { continue; };
                    if packet.stream_local_id == prepared.readiness.audio().stream_local_id {
                        let p = packet.rtp.into_packet();
                        if p.header.payload_type != prepared.readiness.audio().payload_type {
                            return Err("private audio changed its negotiated payload".to_owned());
                        }
                        prepared.audio.decode(p.header.ssrc, p.header.sequence_number,
                            p.header.timestamp, &p.payload)?;
                        audio_packets += 1;
                    } else if let Some(selected) = prepared.readiness.video() {
                        if packet.stream_local_id != selected.stream_local_id { return Err("unexpected private receiving stream".to_owned()); }
                        let p = NativeVideoPacket::parse(packet.rtp.packet().serialize()
                            .map_err(|e| format!("private RTP image: {e:?}"))?.to_vec())
                            .map_err(|e| e.to_string())?;
                        if p.payload_type != selected.payload_type {
                            return Err("private VP8 changed the original selected source".to_owned());
                        }
                        video_packets += 1;
                        if video_ssrc.is_some_and(|original|original!=p.ssrc) {return Err("private video changed original SSRC".to_owned());}
                        video_ssrc=Some(p.ssrc);
                        if private_video.len()>=2048 {return Err("private video observation bound exceeded".to_owned());}
                        private_video.push((packet.seq,p.timestamp_rtp));
                        if let Some(actual) = assembler.push(&p).map_err(|e| e.to_string())? {
                            if !actual.keyframe || actual.encoded.as_slice() != include_bytes!("../../../../../fixtures/gate6-vp8-black-640x360.vp8") {
                                return Err("private decoded source differs from the selected original programme".to_owned());
                            }
                            if frame.is_none() {
                                frame = Some(actual);
                            }
                        }
                    }
                }
            }
        }
        Ok::<(), String>(())
    }).await;
    observed
        .expect("original replacement expired before actual private decoder input")
        .unwrap();
    assert!(input_packets > 0 && audio_packets >= 3);
    let video_tail = publisher.as_ref().map(|source| {
        let first = first_video_sequence.unwrap();
        let count = source.next_sequence.wrapping_sub(first);
        assert!(count > 0 && usize::from(count) <= 2048);
        let timestamps = (0..count)
            .map(|i| u32::from(first.wrapping_add(i)) * 9000)
            .collect();
        VideoTail::new(
            &prepared.readiness,
            video_ssrc.unwrap(),
            timestamps,
            assembler,
            private_video,
        )
    });
    let video = if let Some(frame) = frame {
        assert!(video_packets > 0 && frame.keyframe);
        // The bounded FFmpeg owner consumes the actual received frame, not the
        // source fixture, and is explicitly joined before publishing a proof.
        let output = video::decode(&frame, prepared.cutoff)
            .await
            .expect("original private VP8 keyframe must produce actual pixels");
        assert_eq!(output.pixels, 384);
        eprintln!(
            "native private decoder: pid={} joined=true forced={} output_pixels={} luma_sum={} received_encoded_bytes={}",
            output.pid,
            output.forced,
            output.pixels,
            output.luma_sum,
            frame.encoded.len()
        );
        Some(NativeDecodedVideoReadiness {
            decoded_frames: 1,
            decoded_keyframes: 1,
        })
    } else {
        None
    };
    let samples = prepared.audio.samples();
    // Actual input and decoder output have now occurred, but this fixture has
    // not submitted a signed observation. The original operation must remain
    // preparing; neither arrival nor local decode can implicitly commit it.
    let before_ready = timeout_at(
        prepared.cutoff,
        controller.operation_get(
            &command.operation_id,
            CommandScope {
                conversation_id: Some(command.conversation_id.clone()),
                session_id: Some(command.session_id.clone()),
                connection_id: command.connection_id.clone(),
                ..Default::default()
            },
        ),
    )
    .await
    .expect("original replacement expired before readiness gate observation")
    .unwrap();
    let before_ready = serde_json::to_value(before_ready).unwrap();
    assert_eq!(before_ready["record_id"], root.record_id);
    assert_eq!(before_ready["operation_id"], root.operation_id);
    assert_eq!(
        before_ready["state"], "preparing",
        "native movement committed before signed decoder proof"
    );
    assert!(before_ready["result"].is_null());
    let ready = prepared.readiness.with_decoded(samples, video).unwrap();
    eprintln!(
        "native private decoded readiness: source_generation={} target_generation={} revision={} actual_audio_samples={} selected_video={} input_packets={} received_audio_packets={} received_video_packets={}",
        prepared.readiness.source_generation(),
        prepared.readiness.target_generation(),
        prepared.readiness.stream_revision(),
        samples,
        video.is_some(),
        input_packets,
        audio_packets,
        video_packets
    );
    eprintln!("native private decoder discarded queued preparation packets={drained}");
    for attempt in 0..8 {
        assert!(
            Instant::now() < prepared.cutoff,
            "original replacement expired before frozen ready submission"
        );
        let response = timeout_at(
            prepared.cutoff,
            peer.client.confirm_native_replacement_ready(&ready),
        )
        .await
        .expect("original replacement expired during frozen ready submission");
        let replay_safe = match response {
            Ok(_) => return (ready, video_tail), // narrow capture ACK, not movement completion
            Err(error) => recovery(&error).unwrap_or_else(|| {
                panic!("actual decoded ready refused without retrying allocation: {error}")
            }),
        };
        let current = timeout_at(
            prepared.cutoff,
            controller.operation_get(
                &command.operation_id,
                CommandScope {
                    conversation_id: Some(command.conversation_id.clone()),
                    session_id: Some(command.session_id.clone()),
                    connection_id: command.connection_id.clone(),
                    ..Default::default()
                },
            ),
        )
        .await
        .expect("original replacement expired during status recovery")
        .unwrap();
        let current = serde_json::to_value(current).unwrap();
        assert_eq!(current["record_id"], root.record_id);
        assert_eq!(current["operation_id"], root.operation_id);
        match current["state"].as_str().unwrap() {
            "completed" => {
                assert_eq!(
                    current["result"]["connection_id"],
                    prepared.readiness.connection_id().unwrap()
                );
                assert_eq!(
                    current["result"]["generation"],
                    prepared.readiness.target_generation()
                );
                return (ready, video_tail);
            }
            "preparing" | "ready" | "committing" => {}
            other => panic!("original ready recovery refused in state {other}"),
        }
        // CanonicalClient ends this original physical peer on an uncertain
        // request timeout/transport loss. Its already completed operation may
        // be observed, but a pending one cannot be resumed by reopening it.
        assert!(
            replay_safe,
            "original physical peer ended while decoded readiness remained unresolved"
        );
        assert!(attempt < 7, "original frozen ready retry bound exhausted");
        timeout_at(
            prepared.cutoff,
            tokio::time::sleep(Duration::from_millis(20)),
        )
        .await
        .expect("original replacement expired before exact ready retry");
    }
    unreachable!()
}
/// Some(true): a signed transient refusal leaves the physical client usable
/// for the server's idempotent frozen-proof replay; pending does not prove the
/// first observation was uncaptured. Some(false): final status only; no replay.
fn recovery(error: &ClientError) -> Option<bool> {
    match error {
        ClientError::Application(problem)
            if matches!(
                problem.code.as_str(),
                "authority-busy" | "storage-unavailable"
            ) =>
        {
            Some(true)
        }
        ClientError::Timeout | ClientError::Transport => Some(false),
        _ => None,
    }
}
fn send_video(source: &mut VideoSource<'_>) -> std::result::Result<(), String> {
    let sequence = *source.next_sequence;
    let mut rtp = vec![0x80, 0x80 | 96];
    rtp.extend_from_slice(&sequence.to_be_bytes());
    rtp.extend_from_slice(&(u32::from(sequence) * 9000).to_be_bytes());
    rtp.extend_from_slice(&0x6476_7001u32.to_be_bytes());
    rtp.push(0x10);
    rtp.extend_from_slice(include_bytes!(
        "../../../../../fixtures/gate6-vp8-black-640x360.vp8"
    ));
    source
        .client
        .send_native_media_datagram(&pack(&MediaDatagram {
            flags: 0,
            stream_local_id: source.handle,
            seq: u32::from(sequence),
            payload: rtp.into(),
        }))
        .map_err(|e| e.to_string())?;
    *source.next_sequence = sequence.wrapping_add(1);
    Ok(())
}
