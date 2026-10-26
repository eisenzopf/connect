//! Actual SDK-owned runtime controls, receipt replay and observation-only recovery.
use super::*;
fn targets(id: &str) -> NonEmpty<sdk::StreamId> {
    NonEmpty {
        first: id.parse().unwrap(),
        rest: vec![],
    }
}
fn revision(n: u64) -> sdk::StreamRevision {
    sdk::StreamRevision::try_from(n).unwrap()
}
async fn update(
    connection: &sdk::Connection,
    input: sdk::ConnectionUpdate,
    expected: u64,
    id: &str,
) -> sdk::ConnectionUpdateReceipt {
    connection
        .update(input, revision(expected), mutation(id))
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap()
        .value
}
async fn send(audio: &sdk::Stream, sequence: u16) -> sdk::Result<sdk::LocalMediaSubmission> {
    let pcmu = (0..160)
        .map(|i| if i % 16 < 8 { 0x80 } else { 0x00 })
        .collect::<Vec<_>>();
    audio
        .send_rtp(
            sdk::RtpPacket::new(
                u32::from(sequence),
                super::media::rtp(
                    0,
                    sequence,
                    u32::from(sequence) * 160,
                    0x735f6101,
                    false,
                    &pcmu,
                ),
            )
            .unwrap(),
            options(),
        )
        .await
}
async fn receiving_is_allowed(audio: &sdk::Stream) {
    let result = audio
        .receive_packet(RequestOptions::new(Duration::from_millis(30)).unwrap())
        .await;
    assert!(
        matches!(result, Ok(_)) || matches!(result,Err(ref e) if e.kind==sdk::ErrorKind::Deadline),
        "resumed native read must stay available: {result:?}"
    );
}
pub(super) async fn exercise(
    f: &Fixture,
    client: &Client,
    connection: &sdk::Connection,
    audio: &sdk::Stream,
    video: &sdk::Stream,
) -> u16 {
    let first = audio.refresh(options()).await.unwrap();
    let untouched = video.refresh(options()).await.unwrap();
    let quality = update(
        connection,
        sdk::ConnectionUpdate::Quality { interval_ms: 250 },
        1,
        "sdk-runtime-quality",
    )
    .await;
    assert_eq!(quality.stream_revision.get(), 1);
    assert!(matches!(
        quality.outcome,
        sdk::ConnectionUpdateOutcome::Quality { interval_ms: 250 }
    ));
    assert_eq!(
        connection
            .native_negotiation()
            .unwrap()
            .unwrap()
            .stream_revision
            .get(),
        1
    );
    let hold = sdk::ConnectionUpdate::Hold {
        streams: Some(targets(audio.id().as_str())),
        direction: sdk::HoldDirection::Inactive,
    };
    let pending = audio.receive_packet(options());
    tokio::pin!(pending);
    assert!(
        tokio::time::timeout(Duration::from_millis(10), &mut pending)
            .await
            .is_err()
    );
    let op = connection
        .update(hold.clone(), revision(1), mutation("sdk-runtime-hold"))
        .await
        .unwrap();
    let saved = op.reference_json();
    let held = op.wait(options()).await.unwrap().value;
    assert_eq!(held.stream_revision.get(), 2);
    assert_eq!(held.connection.id(), connection.id());
    assert!(
        matches!(held.outcome,sdk::ConnectionUpdateOutcome::Gates{action:sdk::GateAction::Hold,ref streams} if streams.len()==1 && streams[0].state==sdk::GateState::Paused && streams[0].held_direction.is_none())
    );
    assert!(
        matches!(tokio::time::timeout(Duration::from_millis(500),pending).await.expect("actual hold wakes local reader"),Err(e) if e.kind==sdk::ErrorKind::Unsupported)
    );
    let error = send(audio, 120).await.unwrap_err();
    assert_eq!(error.kind, sdk::ErrorKind::Validation);
    assert_eq!(error.certainty, sdk::Certainty::NotSubmitted);
    let paused = audio.refresh(options()).await.unwrap();
    assert_eq!(paused.state, "paused");
    assert!(paused.revision.get() > first.revision.get());
    assert_eq!(
        video.refresh(options()).await.unwrap().revision,
        untouched.revision
    );
    let receive = update(
        connection,
        sdk::ConnectionUpdate::Hold {
            streams: Some(targets(audio.id().as_str())),
            direction: sdk::HoldDirection::Recvonly,
        },
        2,
        "sdk-runtime-receive-only",
    )
    .await;
    assert!(
        matches!(receive.outcome,sdk::ConnectionUpdateOutcome::Gates{ref streams,..} if streams[0].state==sdk::GateState::Active && streams[0].held_direction==Some(sdk::Direction::Recvonly))
    );
    receiving_is_allowed(audio).await;
    assert!(matches!(send(audio,120).await,Err(e) if e.kind==sdk::ErrorKind::Validation));
    let sending = update(
        connection,
        sdk::ConnectionUpdate::Hold {
            streams: Some(targets(audio.id().as_str())),
            direction: sdk::HoldDirection::Sendonly,
        },
        3,
        "sdk-runtime-send-only",
    )
    .await;
    assert!(
        matches!(sending.outcome,sdk::ConnectionUpdateOutcome::Gates{ref streams,..} if streams[0].held_direction==Some(sdk::Direction::Sendonly))
    );
    assert!(
        matches!(audio.receive_packet(options()).await,Err(e) if e.kind==sdk::ErrorKind::Unsupported)
    );
    send(audio, 120).await.unwrap();
    let muted = update(
        connection,
        sdk::ConnectionUpdate::Mute {
            streams: targets(audio.id().as_str()),
        },
        4,
        "sdk-runtime-mute",
    )
    .await;
    assert!(
        matches!(muted.outcome,sdk::ConnectionUpdateOutcome::Gates{ref streams,..} if streams[0].source_muted)
    );
    assert!(matches!(send(audio,121).await,Err(e) if e.kind==sdk::ErrorKind::Validation));
    let unmuted = update(
        connection,
        sdk::ConnectionUpdate::Unmute {
            streams: targets(audio.id().as_str()),
        },
        5,
        "sdk-runtime-unmute",
    )
    .await;
    assert!(
        matches!(unmuted.outcome,sdk::ConnectionUpdateOutcome::Gates{ref streams,..} if !streams[0].source_muted && streams[0].held_direction==Some(sdk::Direction::Sendonly))
    );
    send(audio, 121).await.unwrap();
    let resumed = update(
        connection,
        sdk::ConnectionUpdate::Resume {
            streams: Some(targets(audio.id().as_str())),
        },
        6,
        "sdk-runtime-resume",
    )
    .await;
    assert_eq!(resumed.stream_revision.get(), 7);
    assert!(
        matches!(resumed.outcome,sdk::ConnectionUpdateOutcome::Gates{ref streams,..} if streams[0].state==sdk::GateState::Active && streams[0].held_direction.is_none())
    );
    receiving_is_allowed(audio).await;
    assert_eq!(audio.refresh(options()).await.unwrap().state, "active");
    let again = update(connection, hold, 1, "sdk-runtime-hold").await;
    assert_eq!(again.stream_revision.get(), 2);
    assert_eq!(
        connection
            .native_negotiation()
            .unwrap()
            .unwrap()
            .stream_revision
            .get(),
        7
    );
    receiving_is_allowed(audio).await;
    let changed = connection
        .update(
            sdk::ConnectionUpdate::Hold {
                streams: Some(targets(audio.id().as_str())),
                direction: sdk::HoldDirection::Recvonly,
            },
            revision(1),
            mutation("sdk-runtime-hold"),
        )
        .await;
    assert!(
        matches!(changed,Err(e) if e.certainty==sdk::Certainty::Refused && e.operation.as_ref().unwrap().operation_id.as_str()=="sdk-runtime-hold")
    );
    let stale = connection
        .update(
            sdk::ConnectionUpdate::Quality { interval_ms: 500 },
            revision(1),
            mutation("sdk-runtime-stale"),
        )
        .await
        .unwrap();
    let Err(error) = stale.wait(options()).await else {
        panic!("stale native revision must not execute")
    };
    assert_eq!(error.certainty, sdk::Certainty::Recorded);
    assert_eq!(
        error.observation.as_ref().unwrap().state,
        sdk::OperationState::Failed
    );
    assert_eq!(
        error.operation.as_ref().unwrap().operation_id.as_str(),
        "sdk-runtime-stale"
    );
    let db = rusqlite::Connection::open(&f.config.database).unwrap();
    let effects:i64=db.query_row("SELECT COUNT(*) FROM operation_effects e JOIN operations o ON o.record_id=e.operation_record_id WHERE o.operation_id='sdk-runtime-stale'",[],|r|r.get(0)).unwrap();
    assert_eq!(effects, 0, "stale failure reserves no engine effect");
    let interval:u64=db.query_row("SELECT json_extract(readiness_json,'$.quality_interval_ms') FROM connections WHERE id=?1",[connection.id().as_str()],|r|r.get(0)).unwrap();
    assert_eq!(
        interval, 250,
        "stale quality update cannot replace the actual setting"
    );
    drop(db);
    let fresh = Client::connect(f.client.clone(), "tnt_default".parse().unwrap())
        .await
        .unwrap();
    let old = fresh
        .restore(saved, fresh.operation_kinds().connection_update)
        .unwrap()
        .wait(options())
        .await
        .unwrap()
        .value;
    assert!(old.connection.native_negotiation().unwrap().is_none());
    assert_eq!(old.stream_revision.get(), 2);
    assert!(
        matches!(old.outcome,sdk::ConnectionUpdateOutcome::Gates{ref streams,..} if streams[0].state==sdk::GateState::Paused)
    );
    assert_eq!(
        connection
            .native_negotiation()
            .unwrap()
            .unwrap()
            .stream_revision
            .get(),
        7
    );
    assert_eq!(
        video.refresh(options()).await.unwrap().revision,
        untouched.revision
    );
    fresh.close(options()).await.unwrap();
    println!("SDK runtime {}:quality_revision=1,hold_revision=2,resume_revision=7,old_receipt_revision=2;directional_hold_mute_unmute_current_replay_and_fresh_recovery",f.client.signing_algorithm);
    122
}
