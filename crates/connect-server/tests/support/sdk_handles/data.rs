//! Both owned SDK peers exchange native data while real audio and recovery run.
use super::selections::{revision, stream};
use super::*;
use sdk::{DataAcceptanceBoundary, DataPayload, Direction, Medium, StreamSelector};
fn body(format: &str, index: usize) -> DataPayload {
    if format == "text" {
        DataPayload::Text(format!("native 🦀 {index}"))
    } else if format == "binary" {
        DataPayload::Binary(vec![0, 255, 128, index as u8])
    } else {
        DataPayload::Json(json!({"sequence":index,"text":"native 🦀"}))
    }
}
async fn audio(
    source: sdk::Stream,
    target: sdk::Stream,
    path: &std::path::Path,
    clock: tokio::time::Instant,
    epoch: u16,
) {
    let sending = async move {
        for i in 0..50u16 {
            let bytes = (0..160)
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
                            0x64617461,
                            false,
                            &bytes,
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
            if let sdk::MediaPacket::Rtp(p) = target.receive_packet(options()).await.unwrap() {
                bytes.extend_from_slice(p.payload());
            }
        }
        bytes
    };
    let (_, bytes) = tokio::join!(sending, receiving);
    media::decoded_audio(path, &bytes);
}
async fn exercise(recording_failure: bool) {
    for algorithm in ["EdDSA", "ES256"] {
        for format in ["json", "text", "binary"] {
            let f = Fixture::with_native_end_scope(algorithm, 16).await;
            super::super::canonical_native_connections::ready_endpoint_with_capabilities(
                &f,
                vec!["audio".into(), "data".into()],
            )
            .await;
            let mut scope = Scope::collaboration(vec!["*".into()]);
            scope.operations.extend([
                "stream.subscribe".into(),
                "stream.unsubscribe".into(),
                "connection.end".into(),
            ]);
            let target_config = super::super::canonical_native_connections::party_with_scope(
                &f,
                &f.peer.identity_id,
                211,
                vec!["audio".into(), "data".into()],
                scope,
            );
            let publisher = Client::connect(f.client.clone(), "tnt_default".parse().unwrap())
                .await
                .unwrap();
            let subscriber = Client::connect(target_config.clone(), "tnt_default".parse().unwrap())
                .await
                .unwrap();
            let room = publisher
                .create_conversation(
                    vec![f.peer.identity_id.parse().unwrap()],
                    ConversationPolicy::Persistent,
                    mutation("data-sdk-create"),
                )
                .await
                .unwrap()
                .wait(options())
                .await
                .unwrap()
                .value;
            let invited = room
                .refresh(options())
                .await
                .unwrap()
                .participants
                .into_iter()
                .find(|p| p.identity_id.as_str() == f.peer.identity_id)
                .unwrap()
                .id;
            let session = room
                .invite(
                    "data-sdk-session".parse().unwrap(),
                    vec![invited],
                    sdk::SessionMedium::Mixed,
                    "conference".into(),
                    json!({"audio":["g.711-mu"],"data":["reliable-ordered"]}),
                    mutation("data-sdk-invite"),
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
                    json!({"audio":["g.711-mu"],"data":["reliable-ordered"]}),
                    sdk::TopologyRevision::try_from(0).unwrap(),
                    mutation("data-sdk-accept"),
                )
                .await
                .unwrap()
                .wait(options())
                .await
                .unwrap();
            let source = session
                .offer_connection(
                    "sdk-data-source".parse().unwrap(),
                    NonEmpty {
                        first: stream("audio", Medium::Audio, Direction::Sendonly, "g.711-mu"),
                        rest: vec![stream(
                            "application",
                            Medium::Data,
                            Direction::Sendonly,
                            format,
                        )],
                    },
                    mutation("data-sdk-source-offer"),
                )
                .await
                .unwrap()
                .wait(options())
                .await
                .unwrap()
                .value;
            let target = target_session
                .offer_connection(
                    "sdk-data-target".parse().unwrap(),
                    NonEmpty {
                        first: stream("audio", Medium::Audio, Direction::Recvonly, "g.711-mu"),
                        rest: vec![stream(
                            "application",
                            Medium::Data,
                            Direction::Recvonly,
                            format,
                        )],
                    },
                    mutation("data-sdk-target-offer"),
                )
                .await
                .unwrap()
                .wait(options())
                .await
                .unwrap()
                .value;
            target
                .subscribe(
                    NonEmpty {
                        first: StreamSelector::Stream("audio".parse().unwrap()),
                        rest: vec![StreamSelector::Stream("application".parse().unwrap())],
                    },
                    revision(1),
                    mutation("data-sdk-select"),
                )
                .await
                .unwrap()
                .wait(options())
                .await
                .unwrap();
            let from = source.refresh(options()).await.unwrap().participant_id;
            let recipient = target.refresh(options()).await.unwrap().participant_id;
            let send = source.stream_by_id("application".parse().unwrap());
            let receive = target.stream_by_id("application".parse().unwrap());
            let db = rusqlite::Connection::open(&f.config.database).unwrap();
            if recording_failure {
                db.execute_batch("CREATE TRIGGER isolated_sdk_data_record_failure BEFORE INSERT ON journal WHEN json_extract(NEW.data_json,'$.state')='completed' AND EXISTS(SELECT 1 FROM operations o WHERE o.operation_id=json_extract(NEW.data_json,'$.operation_id') AND o.command_type='message.send' AND o.connection_id='sdk-data-source') BEGIN SELECT RAISE(ABORT,'isolated SDK data completion failure'); END;").unwrap();
            }
            let clock = tokio::time::Instant::now();
            let first = send
                .send_data(
                    "sdk-data-message-0".parse().unwrap(),
                    NonEmpty {
                        first: recipient.clone(),
                        rest: vec![],
                    },
                    body(format, 0),
                    mutation("sdk-data-send-0"),
                )
                .await
                .unwrap();
            let saved = first.reference_json();
            let delivered = receive.receive_data(options()).await.unwrap();
            assert_eq!(delivered.payload, body(format, 0));
            assert_eq!(delivered.from, from);
            assert_eq!(delivered.operation_id.as_str(), "sdk-data-send-0");
            if recording_failure {
                assert_ne!(
                    first.status(options()).await.unwrap().state,
                    sdk::OperationState::Completed
                );
                let competing = send
                    .send_data(
                        "sdk-data-competing".parse().unwrap(),
                        NonEmpty {
                            first: recipient.clone(),
                            rest: vec![],
                        },
                        body(format, 99),
                        mutation("sdk-data-competing"),
                    )
                    .await
                    .unwrap_err();
                assert_eq!(competing.kind, sdk::ErrorKind::Conflict);
                assert_eq!(competing.certainty, sdk::Certainty::Refused);
                assert_eq!(
                    db.query_row(
                        "SELECT COUNT(*) FROM operations WHERE operation_id='sdk-data-competing'",
                        [],
                        |r| r.get::<_, u64>(0)
                    )
                    .unwrap(),
                    0
                );
                audio(
                    source.stream_by_id("audio".parse().unwrap()),
                    target.stream_by_id("audio".parse().unwrap()),
                    f.temp.path(),
                    clock,
                    0,
                )
                .await;
                assert_ne!(
                    first.status(options()).await.unwrap().state,
                    sdk::OperationState::Completed
                );
                db.execute_batch("DROP TRIGGER isolated_sdk_data_record_failure")
                    .unwrap();
                first.wait(options()).await.unwrap();
            }
            let exchange = async {
                for index in 1..9 {
                    let message = format!("sdk-data-message-{index}");
                    let operation = format!("sdk-data-send-{index}");
                    let pending = send
                        .send_data(
                            message.parse().unwrap(),
                            NonEmpty {
                                first: recipient.clone(),
                                rest: vec![],
                            },
                            body(format, index),
                            mutation(&operation),
                        )
                        .await
                        .unwrap();
                    let delivered = receive.clone().receive_data(options()).await.unwrap();
                    assert_eq!(delivered.message_id.as_str(), message);
                    assert_eq!(delivered.operation_id.as_str(), operation);
                    assert_eq!(delivered.payload, body(format, index));
                    {
                        assert_eq!(
                            pending.wait(options()).await.unwrap().value.boundary,
                            DataAcceptanceBoundary::CoreQueueAdmission
                        );
                    }
                }
            };
            tokio::join!(
                exchange,
                audio(
                    source.stream_by_id("audio".parse().unwrap()),
                    target.stream_by_id("audio".parse().unwrap()),
                    f.temp.path(),
                    clock,
                    if recording_failure { 100 } else { 0 }
                )
            );
            let result = first.wait(options()).await.unwrap().value;
            assert_eq!(result.message_id.as_str(), "sdk-data-message-0");
            assert_eq!(result.boundary, DataAcceptanceBoundary::CoreQueueAdmission);
            // Fresh status recovery carries no original native Stream ownership.
            let fresh = Client::connect(f.client.clone(), "tnt_default".parse().unwrap())
                .await
                .unwrap();
            let original = fresh
                .restore(saved, fresh.operation_kinds().data_send)
                .unwrap();
            assert_eq!(
                original.wait(options()).await.unwrap().value.message_id,
                result.message_id
            );
            let foreign = fresh
                .conversation_by_id(room.id().clone())
                .session_by_id(session.id().clone())
                .connection_by_id(source.id().clone())
                .stream_by_id("application".parse().unwrap());
            assert!(
                matches!(foreign.send_data("foreign-message".parse().unwrap(),NonEmpty {first:recipient.clone(),rest:vec![]},body(format,0),mutation("foreign-native-data")).await,Err(e) if e.kind==sdk::ErrorKind::Unsupported && e.certainty==sdk::Certainty::NotSubmitted)
            );
            assert!(
                matches!(foreign.receive_data(options()).await,Err(e) if e.kind==sdk::ErrorKind::Unsupported)
            );
            assert_eq!(
                db.query_row(
                    "SELECT COUNT(*) FROM operations WHERE operation_id='foreign-native-data'",
                    [],
                    |r| r.get::<_, u64>(0)
                )
                .unwrap(),
                0
            );
            send.send_data(
                "sdk-data-message-0".parse().unwrap(),
                NonEmpty {
                    first: recipient.clone(),
                    rest: vec![],
                },
                body(format, 0),
                mutation("sdk-data-send-0"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap();
            assert!(
                matches!(receive.receive_data(RequestOptions::new(Duration::from_millis(180)).unwrap()).await,Err(e) if e.kind==sdk::ErrorKind::Deadline)
            );
            // Frozen original-intent mismatch is an authorization preflight
            // refusal; it cannot borrow the original publication's grant.
            let changed = send
                .send_data(
                    "sdk-data-message-0".parse().unwrap(),
                    NonEmpty {
                        first: recipient,
                        rest: vec![],
                    },
                    body(format, 99),
                    mutation("sdk-data-send-0"),
                )
                .await
                .unwrap_err();
            assert_eq!(changed.kind, sdk::ErrorKind::Authentication, "{changed:?}");
            assert_eq!(changed.certainty, sdk::Certainty::Refused);
            assert_eq!(db.query_row("SELECT COUNT(*) FROM operation_effects e JOIN operations o ON o.record_id=e.operation_record_id WHERE o.operation_id='sdk-data-send-0' AND o.connection_id='sdk-data-source'",[],|r|r.get::<_,u64>(0)).unwrap(),1);
            assert_eq!(
                db.query_row(
                    "SELECT COUNT(*) FROM messages WHERE id='sdk-data-message-0'",
                    [],
                    |r| r.get::<_, u64>(0)
                )
                .unwrap(),
                1
            );
            let waiting = receive.receive_data(options());
            let ending = async {
                target
                    .end(
                        200,
                        "normal-clearing".into(),
                        None,
                        mutation("sdk-data-end"),
                    )
                    .await
                    .unwrap()
                    .wait(options())
                    .await
                    .unwrap();
            };
            let (waiting, _) = tokio::join!(waiting, ending);
            assert!(matches!(waiting,Err(e) if e.kind==sdk::ErrorKind::Unsupported));
            assert_eq!(subscriber.data_stats().unwrap().queued_bytes, 0);
            eprintln!("SDK native data {algorithm}/{format}, recording_failure={recording_failure}: ordered9 exact payloads, original-ID replay/fresh recovery, one effect, decoded concurrent PCMU, scoped custody and cleanup");
            fresh.close(options()).await.unwrap();
            subscriber.close(options()).await.unwrap();
            publisher.close(options()).await.unwrap();
            f.server.shutdown().await.unwrap();
        }
    }
}
#[tokio::test]
async fn public_sdk_both_ends_reliable_json_binary_data_keeps_decoded_audio_and_original_recovery()
{
    exercise(false).await;
}
#[tokio::test]
async fn public_sdk_native_data_journal_failure_preserves_one_publication_and_live_media() {
    exercise(true).await;
}
