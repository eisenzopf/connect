//! Both owned SDK peers follow future publication through the current journal.
use super::selections::{drain, receive, revision, send, stream};
use super::*;
use sdk::{Direction, Medium, NativeReconciliationPhase as Phase, StreamSelector};
async fn observed(target: &sdk::Connection, phase: Phase) -> sdk::NativeReconciliation {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    loop {
        if let Some(status) = target.native_reconciliation().unwrap() {
            if status.phase == phase {
                return status;
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "SDK reconciliation missing: {:?}",
            target.native_reconciliation().unwrap()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
async fn audio_only(
    source: sdk::Stream,
    target: sdk::Stream,
    epoch: u16,
    clock: tokio::time::Instant,
    directory: &std::path::Path,
) {
    let sending = async move {
        for i in 0..50u16 {
            let payload = (0..160)
                .map(|n| if n % 16 < 8 { 0x80 } else { 0 })
                .collect::<Vec<_>>();
            source
                .send_rtp(
                    sdk::RtpPacket::new(
                        u32::from(epoch + i),
                        media::rtp(
                            0,
                            epoch + i,
                            (clock.elapsed().as_micros() * 8 / 1000) as u32,
                            0x73656101,
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
    };
    let receiving = async move {
        let mut bytes = vec![];
        while bytes.len() < 1600 {
            if let sdk::MediaPacket::Rtp(packet) = target.receive_packet(options()).await.unwrap() {
                bytes.extend_from_slice(packet.payload());
            }
        }
        bytes
    };
    let (_, bytes) = tokio::join!(sending, receiving);
    media::decoded_audio(directory, &bytes);
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
                    rest: vec![],
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

        let participant = source.refresh(options()).await.unwrap().participant_id;
        let origin = target
            .subscribe(
                NonEmpty {
                    first: StreamSelector::Participant {
                        participant_id: participant.clone(),
                        kinds: None,
                    },
                    rest: vec![],
                },
                revision(1),
                mutation("future-sdk-selector"),
            )
            .await
            .unwrap();
        origin.wait(options()).await.unwrap();
        let before = target.native_negotiation().unwrap().unwrap();
        let audio = target.stream_by_id(
            before
                .streams
                .iter()
                .find(|s| s.medium == Medium::Audio && s.direction == Direction::Recvonly)
                .unwrap()
                .id
                .clone(),
        );
        let clock = tokio::time::Instant::now();
        audio_only(
            source.stream_by_id("selection-audio".parse().unwrap()),
            audio.clone(),
            0,
            clock,
            f.temp.path(),
        )
        .await;
        drain(&audio).await;
        let db = rusqlite::Connection::open(&f.config.database).unwrap();
        if recording_failure {
            db.execute_batch("CREATE TRIGGER isolated_sdk_future_record_failure BEFORE INSERT ON journal WHEN json_extract(NEW.data_json,'$.state')='completed' AND EXISTS(SELECT 1 FROM operations o WHERE o.operation_id=json_extract(NEW.data_json,'$.operation_id') AND o.command_type='stream.reconcile' AND o.connection_id='selection-target') BEGIN SELECT RAISE(ABORT,'isolated SDK automatic reconciliation recording failure'); END;").unwrap();
        }
        let camera = session
            .offer_connection(
                "future-sdk-camera".parse().unwrap(),
                NonEmpty {
                    first: stream("selection-video", Medium::Video, Direction::Sendonly, "vp8"),
                    rest: vec![],
                },
                mutation("future-sdk-camera-offer"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        if recording_failure {
            let pending = observed(&target, Phase::AwaitingCompletion).await;
            assert_eq!(
                target
                    .native_negotiation()
                    .unwrap()
                    .unwrap()
                    .stream_revision,
                before.stream_revision
            );
            assert_eq!(
                pending
                    .operation(&subscriber)
                    .status(options())
                    .await
                    .unwrap()
                    .state,
                sdk::OperationState::Preparing
            );
            let error = target
                .unsubscribe(
                    NonEmpty {
                        first: "selection-monitor-video".parse().unwrap(),
                        rest: vec![],
                    },
                    before.stream_revision,
                    mutation("future-sdk-competing"),
                )
                .await
                .err()
                .unwrap();
            assert_eq!(error.kind, sdk::ErrorKind::Conflict);
            let count: u64 = db
                .query_row(
                    "SELECT COUNT(*) FROM operations WHERE operation_id='future-sdk-competing'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 0);
            // A public read remains usable while the known native effect waits
            // for durable completion; it cannot adopt unrecorded new ports.
            origin.status(options()).await.unwrap();
            audio_only(
                source.stream_by_id("selection-audio".parse().unwrap()),
                audio.clone(),
                100,
                clock,
                f.temp.path(),
            )
            .await;
            assert_eq!(
                target
                    .native_negotiation()
                    .unwrap()
                    .unwrap()
                    .stream_revision,
                before.stream_revision
            );
            drain(&audio).await;
            db.execute_batch("DROP TRIGGER isolated_sdk_future_record_failure")
                .unwrap();
        }
        let status = observed(&target, Phase::Recorded).await;
        assert!(status.bindings_adopted);
        let operation = status.operation(&subscriber);
        let saved = operation.reference_json();
        let receipt = operation.wait(options()).await.unwrap().value;
        assert!(receipt.resources_changed);
        assert_eq!(
            receipt.stream_revision.get(),
            before.stream_revision.get() + 1
        );
        assert_eq!(
            receipt
                .bindings
                .as_ref()
                .unwrap()
                .iter()
                .find(|b| b.parameters.id.as_str() == "selection-video")
                .unwrap()
                .stream_generation,
            1
        );
        let camera_receive = target.stream_by_id("selection-video".parse().unwrap());
        let (_, captured) = tokio::join!(
            send(
                source.stream_by_id("selection-audio".parse().unwrap()),
                camera.stream_by_id("selection-video".parse().unwrap()),
                200,
                clock
            ),
            receive(audio.clone(), camera_receive.clone(), true)
        );
        media::decoded_audio(f.temp.path(), &captured.0);
        media::decoded_video(f.temp.path(), &captured.1.unwrap());
        let effects: u64 = db
            .query_row(
                "SELECT COUNT(*) FROM operation_effects WHERE operation_record_id=?1",
                [status.record_id.as_ref().unwrap().as_str()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(effects, 1);
        origin.wait(options()).await.unwrap();
        assert_eq!(
            target
                .native_negotiation()
                .unwrap()
                .unwrap()
                .stream_revision
                .get(),
            receipt.stream_revision.get()
        );
        let fresh = Client::connect(receiver_config, "tnt_default".parse().unwrap())
            .await
            .unwrap();
        let recovered = fresh
            .restore(saved, fresh.operation_kinds().stream_reconcile)
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        assert!(recovered.connection.native_negotiation().unwrap().is_none());
        assert!(recovered
            .connection
            .native_reconciliation()
            .unwrap()
            .is_none());
        assert_eq!(recovered.stream_revision, receipt.stream_revision);
        if recording_failure {
            db.execute_batch("CREATE TRIGGER isolated_sdk_route_record_failure BEFORE INSERT ON journal WHEN json_extract(NEW.data_json,'$.state')='completed' AND EXISTS(SELECT 1 FROM operations o WHERE o.operation_id=json_extract(NEW.data_json,'$.operation_id') AND o.command_type='stream.reconcile' AND o.connection_id='selection-target') BEGIN SELECT RAISE(ABORT,'isolated SDK route-only recording failure'); END;").unwrap();
        }
        camera
            .end(
                200,
                "normal-clearing".into(),
                Some(revision(1)),
                mutation("future-sdk-camera-end"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap();
        if recording_failure {
            let limit = tokio::time::Instant::now() + Duration::from_secs(5);
            loop {
                let pending:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM operations WHERE command_type='stream.reconcile' AND connection_id='selection-target' AND expected_revision=?1 AND state='preparing')",[receipt.stream_revision.get()],|r|r.get(0)).unwrap();
                if pending {
                    break;
                }
                assert!(
                    tokio::time::Instant::now() < limit,
                    "routing-only recording did not start"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            assert_eq!(
                target
                    .native_negotiation()
                    .unwrap()
                    .unwrap()
                    .stream_revision,
                receipt.stream_revision
            );
            audio_only(
                source.stream_by_id("selection-audio".parse().unwrap()),
                audio.clone(),
                400,
                clock,
                f.temp.path(),
            )
            .await;
            assert_eq!(
                target
                    .native_negotiation()
                    .unwrap()
                    .unwrap()
                    .stream_revision,
                receipt.stream_revision,
                "hint must not outrun durable completion"
            );
            db.execute_batch("DROP TRIGGER isolated_sdk_route_record_failure")
                .unwrap();
        }
        let limit = tokio::time::Instant::now() + Duration::from_secs(8);
        let route = loop {
            if let Some(progress) = target.native_reconciliation().unwrap() {
                if progress.phase == Phase::Recorded
                    && progress.key != status.key
                    && target
                        .native_negotiation()
                        .unwrap()
                        .unwrap()
                        .stream_revision
                        .get()
                        == receipt.stream_revision.get() + 1
                {
                    break progress;
                }
            }
            if tokio::time::Instant::now() >= limit {
                let mut statement=db.prepare("SELECT operation_id,state,expected_revision,result_json FROM operations WHERE command_type='stream.reconcile' AND connection_id='selection-target' ORDER BY rowid").unwrap();
                let rows = statement
                    .query_map([], |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, Option<u64>>(2)?,
                            r.get::<_, Option<String>>(3)?,
                        ))
                    })
                    .unwrap()
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .unwrap();
                eprintln!("Controlled routing journal state: {rows:?}");
            }
            assert!(
                tokio::time::Instant::now() < limit,
                "routing-only SDK journal sync missing: {:?}",
                target.native_reconciliation().unwrap()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        let routed = route
            .operation(&subscriber)
            .wait(options())
            .await
            .unwrap()
            .value;
        assert!(!routed.resources_changed);
        assert_eq!(
            routed.stream_revision.get(),
            receipt.stream_revision.get() + 1
        );
        operation.wait(options()).await.unwrap();
        assert_eq!(
            target
                .native_negotiation()
                .unwrap()
                .unwrap()
                .stream_revision,
            routed.stream_revision,
            "historical resource receipt must not rewind route-only counter"
        );
        let after = target
            .subscribe(
                NonEmpty {
                    first: StreamSelector::Participant {
                        participant_id: participant,
                        kinds: None,
                    },
                    rest: vec![],
                },
                routed.stream_revision,
                mutation("after-route-journal-sync"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        assert!(!after.resources_changed);
        assert_eq!(
            target
                .native_negotiation()
                .unwrap()
                .unwrap()
                .stream_revision,
            after.stream_revision
        );
        drain(&audio).await;
        drain(&camera_receive).await;
        let (_, absent) = tokio::join!(
            audio_only(
                source.stream_by_id("selection-audio".parse().unwrap()),
                audio.clone(),
                600,
                clock,
                f.temp.path()
            ),
            camera_receive.receive_packet(RequestOptions::new(Duration::from_millis(450)).unwrap())
        );
        assert!(matches!(absent,Err(e) if e.kind==sdk::ErrorKind::Deadline));
        let effects: u64 = db
            .query_row(
                "SELECT COUNT(*) FROM operation_effects WHERE operation_record_id=?1",
                [route.record_id.as_ref().unwrap().as_str()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(effects, 1);
        eprintln!("SDK route-only journal hint {algorithm}, recording_failure={recording_failure}, old={}, new={}, no new parameters, continued decoded audio, no camera, one effect",receipt.stream_revision.get(),routed.stream_revision.get());
        eprintln!("SDK automatic reconciliation {algorithm}, recording_failure={recording_failure}, generation={}, revision={}, one effect, decoded audio/video",receipt.connection_generation,receipt.stream_revision.get());
        fresh.close(options()).await.unwrap();
        subscriber.close(options()).await.unwrap();
        publisher.close(options()).await.unwrap();
        f.server.shutdown().await.unwrap();
    }
}
#[tokio::test]
async fn public_sdk_future_publication_automatically_checks_journal_and_decodes_both_ends_media() {
    exercise(false).await;
}
#[tokio::test]
async fn public_sdk_automatic_reconciliation_waits_for_durable_capture_and_recovers_one_effect() {
    exercise(true).await;
}
