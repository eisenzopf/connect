//! Actual both-ends SDK selection, original-result recovery and decoded A/V.
use super::*;
use sdk::{Direction, Medium, StreamSelector};
pub(super) fn stream(
    id: &str,
    kind: Medium,
    direction: Direction,
    codec: &str,
) -> sdk::OfferedStream {
    sdk::OfferedStream {
        id: id.parse().unwrap(),
        kind,
        direction,
        codecs: NonEmpty {
            first: codec.into(),
            rest: vec![],
        },
    }
}
pub(super) fn revision(n: u64) -> sdk::StreamRevision {
    sdk::StreamRevision::try_from(n).unwrap()
}
fn selectors() -> NonEmpty<StreamSelector> {
    NonEmpty {
        first: StreamSelector::Stream("selection-audio".parse().unwrap()),
        rest: vec![StreamSelector::Stream("selection-video".parse().unwrap())],
    }
}
pub(super) async fn send(a: sdk::Stream, v: sdk::Stream, epoch: u16, clock: tokio::time::Instant) {
    for i in 0..100u16 {
        let pcmu = (0..160)
            .map(|n| if n % 16 < 8 { 0x80 } else { 0 })
            .collect::<Vec<_>>();
        a.send_rtp(
            sdk::RtpPacket::new(
                u32::from(epoch) + u32::from(i),
                media::rtp(
                    0,
                    epoch + i,
                    (clock.elapsed().as_micros() * 8 / 1000) as u32,
                    0x73656101,
                    false,
                    &pcmu,
                ),
            )
            .unwrap(),
            options(),
        )
        .await
        .unwrap();
        if i % 5 == 0 {
            let mut payload = vec![0x10];
            payload.extend_from_slice(media::VP8);
            v.send_rtp(
                sdk::RtpPacket::new(
                    u32::from(epoch / 5) + u32::from(i / 5),
                    media::rtp(
                        96,
                        epoch / 5 + i / 5,
                        (clock.elapsed().as_micros() * 90 / 1000) as u32,
                        0x73657601,
                        true,
                        &payload,
                    ),
                )
                .unwrap(),
                options(),
            )
            .await
            .unwrap();
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
pub(super) async fn receive(
    audio: sdk::Stream,
    video: sdk::Stream,
    want_video: bool,
) -> (Vec<u8>, Option<Vec<u8>>) {
    let audio_read = async move {
        let mut out = vec![];
        let until = tokio::time::Instant::now() + Duration::from_secs(5);
        while out.len() < 1600 {
            let wait = RequestOptions::new(
                until
                    .saturating_duration_since(tokio::time::Instant::now())
                    .max(Duration::from_millis(1)),
            )
            .unwrap();
            match audio.receive_packet(wait).await.unwrap() {
                sdk::MediaPacket::Rtp(p) => {
                    assert_eq!(p.header().payload_type, 0);
                    out.extend_from_slice(p.payload());
                }
                sdk::MediaPacket::Gap { .. } => (),
                _ => (),
            }
        }
        out
    };
    let video_read = async move {
        if !want_video {
            let wait = RequestOptions::new(Duration::from_millis(450)).unwrap();
            assert!(
                matches!(video.receive_packet(wait).await,Err(e) if e.kind==sdk::ErrorKind::Deadline),
                "unsubscribed video must remain absent while source sends"
            );
            return None;
        }
        let mut assembler = connect_client::VideoFrameAssembler::default();
        loop {
            match video.receive_packet(options()).await.unwrap() {
                sdk::MediaPacket::Rtp(p) => {
                    let frame =
                        connect_client::NativeVideoPacket::parse(p.bytes().to_vec()).unwrap();
                    if let Some(frame) = assembler.push(&frame).unwrap() {
                        assert!(frame.keyframe);
                        assert_eq!(frame.encoded, media::VP8);
                        return Some(frame.encoded);
                    }
                }
                sdk::MediaPacket::Gap { .. } => (),
                _ => (),
            }
        }
    };
    let (a, v) = tokio::join!(audio_read, video_read);
    (a, v)
}
pub(super) async fn drain(s: &sdk::Stream) {
    for _ in 0..40 {
        if matches!(s.receive_packet(RequestOptions::new(Duration::from_millis(25)).unwrap()).await,Err(e) if e.kind==sdk::ErrorKind::Deadline)
        {
            return;
        }
    }
    // The mixer may continue sending silence; bound discard work independently.
}
async fn exercise(recording_failure: bool) {
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_native_end_scope(algorithm, 16).await;
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
        ]);
        let receiver_config = super::super::canonical_native_connections::party_with_scope(
            &f,
            &f.peer.identity_id,
            191,
            vec!["audio".into(), "video".into()],
            scope,
        );
        let publisher = Client::connect(f.client.clone(), "tnt_default".parse().unwrap())
            .await
            .unwrap();
        let room = publisher
            .create_conversation(
                vec![f.peer.identity_id.parse().unwrap()],
                ConversationPolicy::Persistent,
                mutation("selection-room"),
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
                "conference".into(),
                caps.clone(),
                mutation("selection-invite"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        let subscriber = Client::connect(receiver_config.clone(), "tnt_default".parse().unwrap())
            .await
            .unwrap();
        let receiving_session = subscriber
            .conversation_by_id(room.id().clone())
            .session_by_id(session.id().clone());
        receiving_session
            .accept(
                caps,
                sdk::TopologyRevision::try_from(0).unwrap(),
                mutation("selection-accept"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap();
        let source = session
            .offer_connection(
                "selection-source".parse().unwrap(),
                NonEmpty {
                    first: stream(
                        "selection-audio",
                        Medium::Audio,
                        Direction::Sendonly,
                        "g.711-mu",
                    ),
                    rest: vec![stream(
                        "selection-video",
                        Medium::Video,
                        Direction::Sendonly,
                        "vp8",
                    )],
                },
                mutation("selection-source-offer"),
            )
            .await
            .unwrap_or_else(|e| {
                panic!(
                    "SDK source offer: kind={:?},certainty={:?},problem={:?}",
                    e.kind, e.certainty, e.problem
                )
            })
            .wait(options())
            .await
            .unwrap()
            .value;
        let target = receiving_session
            .offer_connection(
                "selection-target".parse().unwrap(),
                NonEmpty {
                    first: stream(
                        "selection-monitor",
                        Medium::Audio,
                        Direction::Recvonly,
                        "g.711-mu",
                    ),
                    rest: vec![stream(
                        "selection-monitor-video",
                        Medium::Video,
                        Direction::Recvonly,
                        "vp8",
                    )],
                },
                mutation("selection-target-offer"),
            )
            .await
            .unwrap_or_else(|e| {
                panic!(
                    "SDK target offer: kind={:?},certainty={:?},problem={:?}",
                    e.kind, e.certainty, e.problem
                )
            })
            .wait(options())
            .await
            .unwrap()
            .value;
        let before = target.native_negotiation().unwrap().unwrap();
        let db = rusqlite::Connection::open(&f.config.database).unwrap();
        if recording_failure {
            db.execute_batch("CREATE TRIGGER isolated_selection_record_failure BEFORE INSERT ON journal WHEN json_extract(NEW.data_json,'$.operation_id')='selection-start' AND json_extract(NEW.data_json,'$.state')='completed' BEGIN SELECT RAISE(ABORT,'isolated SDK selection recording failure'); END;").unwrap();
        }
        let original = target
            .subscribe(selectors(), revision(1), mutation("selection-start"))
            .await
            .unwrap();
        let saved = original.reference_json();
        if recording_failure {
            assert_eq!(
                original.status(options()).await.unwrap().state,
                sdk::OperationState::Preparing
            );
            assert_eq!(
                target
                    .native_negotiation()
                    .unwrap()
                    .unwrap()
                    .stream_revision,
                before.stream_revision
            );
            let error = target
                .subscribe(selectors(), revision(1), mutation("selection-busy"))
                .await
                .err()
                .unwrap();
            assert_eq!(error.kind, sdk::ErrorKind::Conflict);
            let rows: u64 = db
                .query_row(
                    "SELECT count(*) FROM operations WHERE operation_id='selection-busy'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(rows, 0);
            db.execute_batch("DROP TRIGGER isolated_selection_record_failure")
                .unwrap();
        }
        let receipt = original.wait(options()).await.unwrap().value;
        assert_eq!(receipt.stream_revision.get(), 2);
        assert!(receipt.resources_changed);
        assert!(receipt.bindings.is_some());
        let local = target.native_negotiation().unwrap().unwrap();
        assert_eq!(local.stream_revision.get(), 2);
        let audio = target.stream_by_id(
            local
                .streams
                .iter()
                .find(|s| {
                    s.medium == Medium::Audio
                        && matches!(s.direction, Direction::Recvonly | Direction::Sendrecv)
                })
                .unwrap()
                .id
                .clone(),
        );
        let video = target.stream_by_id(
            local
                .streams
                .iter()
                .find(|s| s.id.as_str() == "selection-video")
                .unwrap()
                .id
                .clone(),
        );
        let source_audio = source.stream_by_id("selection-audio".parse().unwrap());
        let source_video = source.stream_by_id("selection-video".parse().unwrap());
        let clock = tokio::time::Instant::now();
        let (_, received) = tokio::join!(
            send(source_audio.clone(), source_video.clone(), 0, clock),
            receive(audio.clone(), video.clone(), true)
        );
        let loud = media::decoded_audio(f.temp.path(), &received.0);
        let pixels = media::decoded_video(f.temp.path(), &received.1.unwrap());
        let stop = target
            .unsubscribe(
                NonEmpty {
                    first: "selection-video".parse().unwrap(),
                    rest: vec![],
                },
                revision(2),
                mutation("selection-stop-video"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        assert!(!stop.resources_changed);
        assert_eq!(stop.stream_revision.get(), 3);
        drain(&audio).await;
        drain(&video).await;
        let (_, received) = tokio::join!(
            send(source_audio.clone(), source_video.clone(), 100, clock),
            receive(audio.clone(), video.clone(), false)
        );
        media::decoded_audio(f.temp.path(), &received.0);
        let noop = target
            .unsubscribe(
                NonEmpty {
                    first: "selection-video".parse().unwrap(),
                    rest: vec![],
                },
                revision(3),
                mutation("selection-stop-again"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        assert_eq!(noop.stream_revision.get(), 3);
        let participant = source.refresh(options()).await.unwrap().participant_id;
        let restart = target
            .subscribe(
                NonEmpty {
                    first: StreamSelector::Participant {
                        participant_id: participant,
                        kinds: Some(NonEmpty {
                            first: Medium::Video,
                            rest: vec![],
                        }),
                    },
                    rest: vec![],
                },
                revision(3),
                mutation("selection-restore-video"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        assert!(!restart.resources_changed);
        assert_eq!(restart.stream_revision.get(), 4);
        drain(&audio).await;
        let (_, received) = tokio::join!(
            send(source_audio, source_video, 200, clock),
            receive(audio, video, true)
        );
        media::decoded_audio(f.temp.path(), &received.0);
        media::decoded_video(f.temp.path(), &received.1.unwrap());
        let restored = subscriber
            .restore(saved.clone(), subscriber.operation_kinds().stream_subscribe)
            .unwrap();
        assert_eq!(
            restored
                .wait(options())
                .await
                .unwrap()
                .value
                .stream_revision
                .get(),
            2
        );
        assert_eq!(
            target
                .native_negotiation()
                .unwrap()
                .unwrap()
                .stream_revision
                .get(),
            4
        );
        let replay = target
            .subscribe(selectors(), revision(1), mutation("selection-start"))
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        assert_eq!(replay.stream_revision.get(), 2);
        assert_eq!(
            target
                .native_negotiation()
                .unwrap()
                .unwrap()
                .stream_revision
                .get(),
            4
        );
        let fresh = Client::connect(receiver_config, "tnt_default".parse().unwrap())
            .await
            .unwrap();
        let historical = fresh
            .restore(saved, fresh.operation_kinds().stream_subscribe)
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        assert!(historical
            .connection
            .native_negotiation()
            .unwrap()
            .is_none());
        fresh.close(options()).await.unwrap();
        let effect:(u64,u64)=db.query_row("SELECT count(*),max(attempt) FROM operation_effects WHERE operation_record_id=(SELECT record_id FROM operations WHERE operation_id='selection-start')",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(effect, (1, 1));
        println!("SDK selection media {algorithm}: both SDK peers, decoded audio loud={loud},video pixels={pixels},unsubscribe absence,route restoration,noop,replay one effect,recording_failure={recording_failure}");
        source
            .end(
                1000,
                "selection source done".into(),
                None,
                mutation("selection-source-end"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap();
        target
            .end(
                1000,
                "selection target done".into(),
                None,
                mutation("selection-target-end"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap();
        subscriber.close(options()).await.unwrap();
        publisher.close(options()).await.unwrap();
        f.server.shutdown().await.unwrap();
        println!("SDK selection {algorithm}: both SDK peers, decoded audio loud={loud},video pixels={pixels},resource and routing commits,unsubscribe silence,noop,restoration,replay one effect,recording_failure={recording_failure}");
    }
}
#[tokio::test]
async fn public_sdk_selections_both_ends_decode_audio_video_and_preserve_route_only_recovery() {
    exercise(false).await;
}
#[tokio::test]
async fn public_sdk_selection_recording_failure_keeps_old_parameters_and_recovers_one_commit() {
    exercise(true).await;
}
