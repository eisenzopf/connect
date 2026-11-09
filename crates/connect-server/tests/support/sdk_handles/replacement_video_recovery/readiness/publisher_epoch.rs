//! The admitted public publisher can honor current PLI during capture projection.
use super::surviving_epoch::{close_decoder, SqlOwner};
use super::*;

async fn current_frame(
    stream: &sdk::Stream,
    index: u32,
    cutoff: Instant,
) -> (NativeVideoFrame, sdk::VideoReceiveEpoch, Instant) {
    let mut assembler = VideoFrameAssembler::default();
    let mut history = None;
    loop {
        let remaining = cutoff
            .checked_duration_since(Instant::now())
            .expect("original frame expired");
        let packet = match stream
            .receive_packet(RequestOptions::new(remaining).unwrap())
            .await
            .unwrap()
        {
            sdk::MediaPacket::Rtp(packet) => packet,
            sdk::MediaPacket::Clock { .. } => continue,
            sdk::MediaPacket::Gap { dropped_packets } => {
                panic!("publisher recovery lost {dropped_packets} packets")
            }
        };
        let arrived = Instant::now();
        assert_eq!(packet.header().timestamp, index * 3000);
        let epoch = packet
            .video_epoch()
            .expect("public packet lacks its signed history")
            .clone();
        if let Some(old) = &history {
            assert_eq!(old, &epoch);
        } else {
            history = Some(epoch);
        }
        if let Some(frame) = assembler
            .push(&NativeVideoPacket::parse(packet.bytes().to_vec()).unwrap())
            .unwrap()
        {
            return (frame, history.unwrap(), arrived);
        }
    }
}

async fn current_hint(
    io: &mut NativeVideoDatagrams,
    receiver: &sdk::Stream,
    camera: u16,
    cutoff: Instant,
) -> bool {
    let mut next = Instant::now();
    loop {
        if io.take_keyframe_request(camera, 0x74657374).unwrap() {
            return true;
        }
        let now = Instant::now();
        if now >= cutoff {
            return false;
        }
        if now >= next {
            receiver
                .request_keyframe(RequestOptions::new(cutoff - now).unwrap())
                .await
                .unwrap();
            next = now + Duration::from_millis(100);
        }
        // claim_native_video_datagrams already owns the signaling reader.
        // Pump the same physical datagram reader so original hints are parsed;
        // a cancelled read retains their original arrival and one-use state.
        match timeout_at(
            cutoff.min(Instant::now() + Duration::from_millis(10)),
            io.receive(),
        )
        .await
        {
            Ok(Ok(_)) | Err(_) => {}
            Ok(Err(error)) => panic!("original publisher reader refused: {error}"),
        }
    }
}

pub(in super::super) async fn audit(
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
    camera: u16,
    receiver: &sdk::Stream,
    target_sequence: &mut u16,
    previous: &sdk::VideoReceiveEpoch,
    cutoff: Instant,
) -> std::result::Result<(), String> {
    let preparation = client.native_replacement_readiness().unwrap();
    let original = original_cutoff(f, root);
    io.reset_video_receive_decoder(incoming).unwrap();
    let mut decoder = Decoder::open(encoder, cutoff);
    let mut sql = None;
    let mut pause = None;
    let work = AssertUnwindSafe(async {
        let samples = audio(io, &preparation, cutoff).await;
        let private_key = encoder.frame(1, true).await;
        publish(source, &private_key, 1, sequence).await;
        let (frame, _, arrived) = video(io, incoming, 1, cutoff).await;
        decoder.frame(&frame, 1, arrived, cutoff).await;
        let ready = preparation.with_decoded(samples, Some(NativeDecodedVideoReadiness {
            decoded_frames: 1, decoded_keyframes: 1,
        })).unwrap();
        pause = Some(f.server.service.pause_canonical_replacement_projection_for_test(
            root.record_id.as_ref().unwrap().as_str(),
        ).unwrap());
        submit(client, &ready, controller, command, root, cutoff).await;
        timeout_at(cutoff, pause.as_ref().unwrap().entered()).await.unwrap();
        sql = Some(SqlOwner::acquire(f.server.store.clone()));
        pause.as_ref().unwrap().release();
        timeout_at(cutoff, pause.as_ref().unwrap().left()).await.unwrap();

        let began = Instant::now();
        let until = cutoff.min(began + Duration::from_millis(200));
        let recovered = timeout_at(until, async {
            let delta = encoder.frame(2, false).await;
            assert_eq!(delta[0] & 1, 1, "the counterexample must start mid-GOP");
            raw_publish(io, camera, &delta, 2, target_sequence);
            let (frame, epoch, _) = current_frame(receiver, 2, until).await;
            assert_eq!(frame.encoded, delta);
            assert!(!frame.keyframe);
            assert_ne!(epoch.epoch, previous.epoch);
            assert_eq!(epoch.incarnation, previous.incarnation);
            assert!(epoch.revision > previous.revision);
            assert!(current_hint(io, receiver, camera, until).await,
                "admitted publisher consumed zero current PLI while capture SQL held");
            // A true result is a consumed original hint under this publisher's
            // actual signed epoch, SSRC, allocation and current control version.
            let key = encoder.frame(3, true).await;
            assert!(!io.take_keyframe_request(camera, 0x74657374).unwrap(), "hint was replayed");
            raw_publish(io, camera, &key, 3, target_sequence);
            let (frame, current, arrived) = current_frame(receiver, 3, until).await;
            assert_eq!(current, epoch);
            assert_eq!(frame.encoded, key);
            assert!(frame.keyframe);
            decoder.frame(&frame, 3, arrived, until).await;
        }).await;
        assert!(recovered.is_ok(), "admitted publisher had no usable PLI/keyframe before original 200ms cut");
        assert_eq!(decoder.outputs, 2);
        let raw = rusqlite::Connection::open(&f.config.database).unwrap();
        let (state, ordinary): (String, bool) = raw.query_row(
            "SELECT o.state,EXISTS(SELECT 1 FROM native_replacement_lifetimes l WHERE l.operation_record_id=o.record_id) FROM operations o WHERE o.record_id=?1",
            [root.record_id.as_ref().unwrap().as_str()], |row| Ok((row.get(0)?,row.get(1)?)),
        ).unwrap();
        assert_eq!(state, "preparing");
        assert!(!ordinary, "epoch metadata prematurely published an ordinary lifetime");
        eprintln!("admitted publisher honored one current PLI and decoded fresh key while capture SQL held: elapsed_us={} ordinary_lifetime=false", began.elapsed().as_micros());
    }).catch_unwind().await;

    let sql_joined = sql.as_mut().is_none_or(SqlOwner::finish);
    drop(sql);
    drop(pause);
    let done =
        AssertUnwindSafe(async { timeout_at(cutoff, completed(f, controller, command)).await })
            .catch_unwind()
            .await;
    // After successful publication, the unchanged signed hold must still deny
    // actual new RTP. No metadata notice or previously consumed hint bypasses it.
    let denied = if work.is_ok() && matches!(&done, Ok(Ok(_))) {
        AssertUnwindSafe(async {
            let scope = CommandScope { conversation_id: Some(command.conversation_id.clone()),
                session_id: Some(command.session_id.clone()), connection_id: Some(original.candidate_connection_id.clone()), ..Default::default() };
            let revision: u64 = rusqlite::Connection::open(&f.config.database).unwrap().query_row(
                "SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1",
                [&original.candidate_connection_id], |row| row.get(0),
            ).unwrap();
            let held = gate(client, &scope, "admitted-publisher-current-hold", revision, "hold").await;
            assert_eq!(held, revision + 1);
            let delta = encoder.frame(4, false).await;
            raw_publish(io, camera, &delta, 4, target_sequence);
            let until = cutoff.min(Instant::now() + Duration::from_millis(100));
            loop {
                let Some(remaining) = until.checked_duration_since(Instant::now()) else { break; };
                match receiver.receive_packet(RequestOptions::new(remaining).unwrap()).await {
                    Ok(sdk::MediaPacket::Clock { .. }) => {},
                    Ok(sdk::MediaPacket::Rtp(_)) => panic!("current held publisher reached receiver"),
                    Ok(sdk::MediaPacket::Gap { dropped_packets }) => panic!("hold check lost {dropped_packets} packets"),
                    Err(error) if error.certainty == sdk::Certainty::ReadOnly && matches!(error.kind, sdk::ErrorKind::Deadline | sdk::ErrorKind::ResyncRequired) => break,
                    Err(error) => panic!("unexpected current hold result: {error}"),
                }
            }
            assert!(!io.take_keyframe_request(camera, 0x74657374).unwrap());
        }).catch_unwind().await
    } else {
        Ok(())
    };
    let decoder_joined = close_decoder(decoder, cutoff).await;
    if !sql_joined || !decoder_joined {
        return Err("original publisher SQL or C decoder owner did not join naturally".into());
    }
    for outcome in [work, denied] {
        if let Err(panic) = outcome {
            return Err(panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .unwrap_or_else(|| "original admitted publisher audit panicked".into()));
        }
    }
    let done = done
        .map_err(|_| "same original root observation panicked".to_owned())?
        .map_err(|_| "same original root did not finish after SQL release".to_owned())?;
    if done.record_id != root.record_id || done.operation_id != root.operation_id {
        return Err("publisher audit changed the original replacement root".into());
    }
    Ok(())
}
