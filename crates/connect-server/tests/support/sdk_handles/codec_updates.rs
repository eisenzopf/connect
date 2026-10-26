//! Actual SDK codec replacement, captured recording failure and independent media continuation.
use super::*;
fn input(stream: &sdk::Stream) -> sdk::ConnectionUpdate {
    sdk::ConnectionUpdate::CodecRenegotiate {
        streams: Some(NonEmpty {
            first: stream.id().clone(),
            rest: vec![],
        }),
        codec_preferences: NonEmpty {
            first: "pcma".into(),
            rest: vec![],
        },
    }
}
pub(super) async fn replace(
    f: &Fixture,
    client: &Client,
    room: &sdk::Conversation,
    session: &sdk::Session,
    connection: &sdk::Connection,
    audio: &sdk::Stream,
    video: &sdk::Stream,
) {
    let before = connection.native_negotiation().unwrap().unwrap();
    assert_eq!(before.stream_revision.get(), 7);
    let old = before
        .streams
        .iter()
        .find(|s| s.id == *audio.id())
        .unwrap()
        .stream_local_id;
    let untouched = video.refresh(options()).await.unwrap();
    let db = rusqlite::Connection::open(&f.config.database).unwrap();
    db.execute_batch("CREATE TRIGGER isolated_sdk_codec_record_failure BEFORE INSERT ON journal WHEN json_extract(NEW.data_json,'$.operation_id')='sdk-media-codec-pcma' AND json_extract(NEW.data_json,'$.state')='completed' BEGIN SELECT RAISE(ABORT,'isolated SDK codec recording failure'); END;").unwrap();
    let operation = connection
        .update(
            input(audio),
            sdk::StreamRevision::try_from(7).unwrap(),
            mutation("sdk-media-codec-pcma"),
        )
        .await
        .unwrap();
    let saved = operation.reference_json();
    assert_eq!(
        operation.status(options()).await.unwrap().state,
        sdk::OperationState::Preparing
    );
    assert_eq!(
        connection
            .native_negotiation()
            .unwrap()
            .unwrap()
            .streams
            .iter()
            .find(|s| s.id == *audio.id())
            .unwrap()
            .stream_local_id,
        old,
        "confirmation alone cannot replace old local parameters"
    );
    let busy = connection
        .update(
            input(audio),
            sdk::StreamRevision::try_from(7).unwrap(),
            mutation("sdk-media-codec-busy"),
        )
        .await
        .unwrap_err();
    assert_eq!(busy.kind, sdk::ErrorKind::Conflict);
    assert_eq!(busy.certainty, sdk::Certainty::NotSubmitted);
    let busy_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM operations WHERE operation_id='sdk-media-codec-busy'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(busy_rows, 0);
    let current = session.refresh(options()).await.unwrap();
    let pid = connection.refresh(options()).await.unwrap().participant_id;
    tokio::time::timeout(Duration::from_secs(3), async {
        session
            .update(
                sdk::SessionUpdate::RoleChange {
                    participant_id: pid,
                    role: "speaker".into(),
                },
                current.topology_revision,
                mutation("sdk-codec-role-writer"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
    })
    .await
    .expect("captured metadata retry holds no idle role reader");
    assert_eq!(room.id(), connection.conversation_id());
    db.execute_batch("DROP TRIGGER isolated_sdk_codec_record_failure")
        .unwrap();
    let receipt = operation.wait(options()).await.unwrap().value;
    assert_eq!(receipt.stream_revision.get(), 8);
    let sdk::ConnectionUpdateOutcome::Codecs {
        streams,
        resources_changed,
    } = receipt.outcome
    else {
        panic!("actual codec receipt")
    };
    assert!(resources_changed);
    let changed = streams
        .iter()
        .find(|s| s.parameters.id == *audio.id())
        .unwrap();
    assert_eq!(changed.parameters.codec["name"], "pcma");
    assert_eq!(changed.stream_generation, 2);
    assert_ne!(changed.parameters.stream_local_id, old);
    assert_eq!(
        connection
            .native_negotiation()
            .unwrap()
            .unwrap()
            .stream_revision
            .get(),
        8
    );
    assert_eq!(
        video.refresh(options()).await.unwrap().revision,
        untouched.revision
    );
    let replay = connection
        .update(
            input(audio),
            sdk::StreamRevision::try_from(7).unwrap(),
            mutation("sdk-media-codec-pcma"),
        )
        .await
        .unwrap();
    assert_eq!(replay.reference_json(), saved);
    assert_eq!(
        replay
            .wait(options())
            .await
            .unwrap()
            .value
            .stream_revision
            .get(),
        8
    );
    let effects:i64=db.query_row("SELECT COUNT(*) FROM operation_effects e JOIN operations o ON o.record_id=e.operation_record_id WHERE o.operation_id='sdk-media-codec-pcma'",[],|r|r.get(0)).unwrap();
    assert_eq!(
        effects, 1,
        "same original codec effect after recording recovery and replay"
    );
    drop(db);
    let fresh = Client::connect(f.client.clone(), "tnt_default".parse().unwrap())
        .await
        .unwrap();
    let restored = fresh
        .restore(saved, fresh.operation_kinds().connection_update)
        .unwrap()
        .wait(options())
        .await
        .unwrap()
        .value;
    assert_eq!(restored.stream_revision.get(), 8);
    assert!(restored.connection.native_negotiation().unwrap().is_none());
    fresh.close(options()).await.unwrap();
    println!("SDK codec {}:one_effect,recording_failure_role_writer,recovered_revision=8,new_audio_generation=2,unchanged_video;fresh_receipt_has_no_ownership",f.client.signing_algorithm);
}
