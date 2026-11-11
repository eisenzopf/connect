//! Authenticated epochs reach surviving SDK receivers during blocked metadata SQL.
use super::*;
use std::sync::mpsc;

pub(in super::super) struct SqlOwner {
    release: Option<mpsc::Sender<()>>,
    job: Option<std::thread::JoinHandle<connect_protocol::Result<()>>>,
}
impl SqlOwner {
    pub(in super::super) fn acquire(store: std::sync::Arc<connect_store::Store>) -> Self {
        let (entered, ready) = mpsc::channel();
        let (release, stopped) = mpsc::channel();
        let job = std::thread::spawn(move || {
            store.with_primary_sql_held_for_test(|| {
                let _ = entered.send(());
                let _ = stopped.recv_timeout(Duration::from_secs(3));
            })
        });
        let owner = Self {
            release: Some(release),
            job: Some(job),
        };
        ready
            .recv_timeout(Duration::from_secs(1))
            .expect("original primary SQL holder did not enter");
        owner
    }
    pub(in super::super) fn finish(&mut self) -> bool {
        self.release.take();
        self.job
            .take()
            .is_none_or(|job| matches!(job.join(), Ok(Ok(()))))
    }
}
impl Drop for SqlOwner {
    fn drop(&mut self) {
        self.finish();
    }
}

async fn current_key(
    stream: &sdk::Stream,
    index: u32,
    until: Instant,
) -> (NativeVideoFrame, sdk::VideoReceiveEpoch, Instant) {
    let mut assembler = VideoFrameAssembler::default();
    let mut history = None;
    loop {
        let remaining = until
            .checked_duration_since(Instant::now())
            .expect("original key frame expired");
        let packet = match stream
            .receive_packet(RequestOptions::new(remaining).unwrap())
            .await
            .unwrap()
        {
            sdk::MediaPacket::Rtp(packet) => packet,
            sdk::MediaPacket::Clock { .. } => continue,
            sdk::MediaPacket::Gap { dropped_packets } => {
                panic!("new epoch lost {dropped_packets} packets behind metadata")
            }
        };
        let arrived = Instant::now();
        assert_eq!(packet.header().timestamp, index * 3000);
        let epoch = packet
            .video_epoch()
            .expect("new packet has no authenticated history")
            .clone();
        if let Some(previous) = &history {
            assert_eq!(previous, &epoch);
        } else {
            history = Some(epoch);
        }
        let packet = NativeVideoPacket::parse(packet.bytes().to_vec()).unwrap();
        if let Some(frame) = assembler.push(&packet).unwrap() {
            return (frame, history.unwrap(), arrived);
        }
    }
}

pub(super) async fn close_decoder(decoder: Decoder, cutoff: Instant) -> bool {
    let Decoder {
        mut child,
        mut input,
        mut output,
        pid,
        outputs,
        ..
    } = decoder;
    let mut trailing = [0u8; 1];
    let stopped = timeout_at(cutoff, async {
        input.shutdown().await?;
        drop(input);
        let extra = output.read(&mut trailing).await?;
        let status = child.wait().await?;
        Ok::<_, std::io::Error>((extra, status))
    })
    .await;
    match stopped {
        Ok(Ok((extra, status))) => {
            eprintln!("surviving epoch decoder joined pid={pid} outputs={outputs} status={status} forced=false");
            extra == 0 && status.success()
        }
        error => {
            let kill = child.start_kill();
            let joined = child.wait().await;
            eprintln!("surviving epoch decoder forced pid={pid} result={error:?} kill={kill:?} joined={joined:?}");
            false
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
    io.reset_video_receive_decoder(incoming).unwrap();
    let mut decoder = Decoder::open(encoder, cutoff);
    let mut sql = None;
    let mut pause = None;
    let work = AssertUnwindSafe(async {
        let samples = audio(io, &preparation, cutoff).await;
        let key = encoder.frame(1, true).await;
        publish(source, &key, 1, sequence).await;
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
        // The holder owns the primary SQL mutex while independent packet
        // observers continue to derive current media authority.
        sql = Some(SqlOwner::acquire(f.server.store.clone()));
        pause.as_ref().unwrap().release();
        timeout_at(cutoff, pause.as_ref().unwrap().left()).await.unwrap();
        let began = Instant::now();
        let until = cutoff.min(began + Duration::from_millis(200));
        let key = encoder.frame(2, true).await;
        raw_publish(io, camera, &key, 2, target_sequence);
        let (frame, epoch, arrived) = current_key(receiver, 2, until).await;
        assert_eq!(frame.encoded, key);
        assert!(frame.keyframe);
        assert_ne!(epoch.epoch, previous.epoch);
        assert_eq!(epoch.incarnation, previous.incarnation);
        assert!(epoch.revision > previous.revision);
        decoder.frame(&frame, 2, arrived, until).await;
        assert_eq!(decoder.outputs, 2);
        let raw = rusqlite::Connection::open(&f.config.database).unwrap();
        let state: String = raw.query_row("SELECT state FROM operations WHERE record_id=?1",
            [root.record_id.as_ref().unwrap().as_str()], |row| row.get(0)).unwrap();
        assert_eq!(state, "preparing", "metadata completed despite the original held SQL mutex");
        eprintln!("surviving SDK epoch decoded while capture SQL held: prior_revision={} current_revision={} elapsed_us={} same_physical=true completed_inferred=false",
            previous.revision, epoch.revision, began.elapsed().as_micros());
    }).catch_unwind().await;
    // Release and join the original SQL holder before the expected
    // old-condition assertion. Dropping the pause releases its owner.
    let sql_joined = sql.as_mut().is_none_or(SqlOwner::finish);
    drop(sql);
    drop(pause);
    let done =
        AssertUnwindSafe(async { timeout_at(cutoff, completed(f, controller, command)).await })
            .catch_unwind()
            .await;
    let decoder_joined = close_decoder(decoder, cutoff).await;
    if !sql_joined || !decoder_joined {
        return Err("original SQL or C decoder owner did not join naturally".into());
    }
    if let Err(panic) = work {
        return Err(panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| {
                panic
                    .downcast_ref::<&str>()
                    .map(|message| (*message).to_owned())
            })
            .unwrap_or_else(|| "original epoch audit panicked".into()));
    }
    let done = done
        .map_err(|_| "same original root observation panicked after SQL release".to_owned())?
        .map_err(|_| "same original root failed to finish after SQL release".to_owned())?;
    if done.record_id != root.record_id || done.operation_id != root.operation_id {
        return Err("completion changed the original logical root".into());
    }
    Ok(())
}
