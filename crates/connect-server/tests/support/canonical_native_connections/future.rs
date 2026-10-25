//! A retained Participant intent follows real later publication and removal.
use super::*;
#[tokio::test]
async fn production_native_future_camera_reoffers_on_publication_and_retires_source_without_losing_audio(
) {
    future_camera(false, false, "EdDSA").await;
}
#[tokio::test]
async fn production_native_future_camera_captured_receipt_recovers_without_retaining_command_readers(
) {
    future_camera(true, false, "EdDSA").await;
}
#[tokio::test]
async fn production_native_unfiltered_participant_selection_routes_current_audio_and_future_camera()
{
    future_camera(false, true, "EdDSA").await;
}
#[tokio::test]
async fn production_native_es256_unfiltered_participant_selection_preserves_explicit_removal() {
    future_camera(false, true, "ES256").await;
}
async fn answer(client: &mut CanonicalClient, offer: &Envelope, stale: bool) {
    let p = &offer.original()["payload"];
    let streams=p["substrate_setup"]["streams"].as_array().unwrap().iter().map(|s|json!({"id":s["strm_id"],"kind":s["kind"],"direction":match s["direction"].as_str().unwrap(){"sendonly"=>"recvonly","recvonly"=>"sendonly",_=>"sendrecv"},"codec":s["codec"]})).collect::<Vec<_>>();
    let mut payload = json!({"by_participant":p["by_participant"],"substrate":"quic","capabilities":av_capabilities(),"streams_answered":streams,"substrate_setup":p["substrate_setup"]});
    if stale {
        payload["substrate_setup"]["revision"] = json!(999);
    }
    client
        .answer_connection_offer(
            offer,
            serde_json::from_value(payload).unwrap(),
            &CommandScope {
                conversation_id: offer
                    .scope(rvoip_uctp::v2::Scope::Conversation)
                    .map(str::to_owned),
                session_id: offer
                    .scope(rvoip_uctp::v2::Scope::Session)
                    .map(str::to_owned),
                connection_id: offer
                    .scope(rvoip_uctp::v2::Scope::Connection)
                    .map(str::to_owned),
                operation_id: offer
                    .scope(rvoip_uctp::v2::Scope::Operation)
                    .map(str::to_owned),
                expected_revision: offer.expected_revision(),
            },
        )
        .await
        .unwrap();
}
async fn reoffer(client: &mut CanonicalClient) -> Envelope {
    let offer = tokio::time::timeout(Duration::from_secs(6), client.receive_control())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(offer.kind(), "connection.offer", "{:?}", offer.original());
    assert!(offer
        .scope(rvoip_uctp::v2::Scope::Operation)
        .unwrap()
        .starts_with("reconcile_"));
    offer
}
async fn future_camera(recording_failure: bool, all_streams: bool, algorithm: &str) {
    let f = Fixture::with_native_stream_scope(algorithm, 12).await;
    ready_endpoint(&f).await;
    let config = party_with_capabilities(
        &f,
        &f.peer.identity_id,
        98,
        false,
        vec!["audio".into(), "video".into()],
    );
    let mut target = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut source = CanonicalClient::open(config.clone()).await.unwrap();
    let cid = session_with_media(
        &f,
        &mut target,
        "voice+video",
        json!({"audio":["g.711-mu"],"video":["vp8"]}),
    )
    .await;
    let t=negotiate_av(&mut target,&cid,"future-target-audio","future-target",json!([{"id":"future-monitor","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}])).await;
    let s=negotiate_av(&mut source,&cid,"future-source-audio","future-source",json!([{"id":"future-voice","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}])).await;
    wait_native_operation(&f, &f.client.credential, &cid, "future-target-audio").await;
    wait_native_operation(&f, &config.credential, &cid, "future-source-audio").await;
    let sh = s.original()["payload"]["substrate_setup"]["streams"][0]["stream_local_id"]
        .as_u64()
        .unwrap() as u16;
    let th = t.original()["payload"]["substrate_setup"]["streams"][0]["stream_local_id"]
        .as_u64()
        .unwrap() as u16;
    let mut clients = vec![source, target];
    if all_streams {
        // No implicit receive route, including after an atomically refused
        // mixed valid/unknown selection or an unsupported explicit kind.
        for (operation, selectors) in [
            (
                "select-no-source",
                json!([{"strm_id":"future-voice"},{"strm_id":"missing-voice"}]),
            ),
            (
                "select-no-participant",
                json!([{"from_participant":"missing-participant"}]),
            ),
            (
                "select-unsupported-screen",
                json!([{"from_participant":s.original()["payload"]["by_participant"],"kinds":["screen"]}]),
            ),
        ] {
            assert!(stream_control_state(
                &mut clients[1],
                &cid,
                "future-target",
                operation,
                1,
                "stream.subscribe",
                json!({"subscriptions":selectors}),
                "failed"
            )
            .await
            .is_none());
        }
        let c = rusqlite::Connection::open(&f.config.database).unwrap();
        let effects: u64 = c.query_row("SELECT COUNT(*) FROM operation_effects e JOIN operations o ON o.record_id=e.operation_record_id WHERE o.operation_id IN ('select-no-source','select-no-participant','select-unsupported-screen')", [], |r| r.get(0)).unwrap();
        assert_eq!(effects, 0);
        assert_audio_is_silent_after_unsubscribe(&mut clients, sh, th).await;
    }
    if !all_streams {
        assert!(stream_control(
            &mut clients[1],
            &cid,
            "future-target",
            "future-select-voice",
            1,
            "stream.subscribe",
            json!({"subscriptions":[{"strm_id":"future-voice"}]})
        )
        .await
        .is_none());
    }
    let source_pid = s.original()["payload"]["by_participant"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(stream_control(
        &mut clients[1],
        &cid,
        "future-target",
        "future-select-participant-video",
        if all_streams { 1 } else { 2 },
        "stream.subscribe",
        if all_streams {
            json!({"subscriptions":[{"from_participant":source_pid}]})
        } else {
            json!({"subscriptions":[{"from_participant":source_pid,"kinds":["video"]}]})
        }
    )
    .await
    .is_none());
    assert_audio_reaches_other_participant(&mut clients, sh, &[th], 28000).await;
    // A distinct physical producer is added after the receive intent commits.
    // Its EOF must later remove only this camera, preserving the audio peer.
    let mut publisher = CanonicalClient::open(config.clone()).await.unwrap();
    let camera=negotiate_av(&mut publisher,&cid,"future-publish-camera","future-camera",json!([{"id":"later-camera","kind":"video","direction":"sendonly","codec_preferences":["vp8"]}])).await;
    wait_native_operation(&f, &config.credential, &cid, "future-publish-camera").await;
    let offer = reoffer(&mut clients[1]).await;
    let operation = offer
        .scope(rvoip_uctp::v2::Scope::Operation)
        .unwrap()
        .to_owned();
    let expected = offer.expected_revision().unwrap();
    let setup = &offer.original()["payload"]["substrate_setup"];
    let stream = setup["streams"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["strm_id"] == "later-camera")
        .unwrap();
    let vh = stream["stream_local_id"].as_u64().unwrap() as u16;
    let camera_handle = camera.original()["payload"]["substrate_setup"]["streams"][0]
        ["stream_local_id"]
        .as_u64()
        .unwrap() as u16;
    answer(&mut clients[1], &offer, true).await;
    assert_eq!(clients[1].receive_control().await.unwrap().kind(), "error");
    let c = rusqlite::Connection::open(&f.config.database).unwrap();
    let state: String = c
        .query_row(
            "SELECT state FROM operations WHERE operation_id=?1",
            [&operation],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "preparing");
    assert_audio_reaches_other_participant(&mut clients, sh, &[th], 28200).await;
    if recording_failure {
        assert!(operation
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_'));
        c.execute_batch(&format!("CREATE TRIGGER controlled_future_record_failure BEFORE INSERT ON journal WHEN json_extract(NEW.data_json,'$.operation_id')='{operation}' AND json_extract(NEW.data_json,'$.state')='completed' BEGIN SELECT RAISE(ABORT,'controlled captured reconciliation recording failure'); END;")).unwrap();
    }
    answer(&mut clients[1], &offer, false).await;
    assert_vp8_routes(&publisher, &mut clients[1], camera_handle, vh).await;
    if all_streams {
        // An explicit removal overrides the retained Participant selector.
        // Both a replay and a new no-op command must preserve that exception.
        for (operation, revision) in [
            ("future-remove-camera", expected + 1),
            ("future-remove-camera", expected + 1),
            ("future-remove-camera-again", expected + 2),
        ] {
            assert!(stream_control(
                &mut clients[1],
                &cid,
                "future-target",
                operation,
                revision,
                "stream.unsubscribe",
                json!({"strm_ids":["later-camera"]}),
            )
            .await
            .is_none());
        }
        assert_vp8_absent(&publisher, &mut clients[1], camera_handle, vh).await;
        assert_audio_reaches_other_participant(&mut clients, sh, &[th], 28300).await;
        // Explicit selection clears the exception without another physical
        // allocation: the already negotiated receiver binding is still valid.
        assert!(stream_control(
            &mut clients[1],
            &cid,
            "future-target",
            "future-restore-camera",
            expected + 2,
            "stream.subscribe",
            json!({"subscriptions":[{"strm_id":"later-camera"}]}),
        )
        .await
        .is_none());
        assert_vp8_routes_from(&publisher, &mut clients[1], camera_handle, vh, 200).await;
    }
    if recording_failure {
        let state: String = c
            .query_row(
                "SELECT state FROM operations WHERE operation_id=?1",
                [&operation],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state, "preparing");
        assert_audio_reaches_other_participant(&mut clients, sh, &[th], 28400).await;
        // This public writer needs current Conversation authority. It would
        // stall if the automatic receipt retry retained its old grant readers.
        let revision = c
            .query_row(
                "SELECT topology_revision FROM sessions WHERE id='native-session'",
                [],
                |r| r.get::<_, u64>(0),
            )
            .unwrap();
        tokio::time::timeout(
            Duration::from_secs(2),
            clients[1].session_command(Request {
                operation_id: "role-during-future-recording".into(),
                conversation_id: cid.clone(),
                session_id: "native-session".into(),
                by_participant: None,
                expected_revision: Some(revision),
                action: Action::Role {
                    participant_id: t.original()["payload"]["by_participant"]
                        .as_str()
                        .unwrap()
                        .into(),
                    role: "speaker".into(),
                },
            }),
        )
        .await
        .unwrap()
        .unwrap();
        c.execute_batch("DROP TRIGGER controlled_future_record_failure")
            .unwrap();
    }
    wait_native_operation(&f, &f.client.credential, &cid, &operation).await;
    let (effects,revision):(u64,u64)=c.query_row("SELECT (SELECT COUNT(*) FROM operation_effects e WHERE e.operation_record_id=o.record_id),json_extract(o.result_json,'$.stream_revision') FROM operations o WHERE o.operation_id=?1",[&operation],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(effects, 1);
    assert_eq!(revision, expected + 1);
    let before:u64=c.query_row("SELECT COALESCE(MAX(rowid),0) FROM operations WHERE command_type='stream.reconcile' AND connection_id='future-target'",[],|r|r.get(0)).unwrap();
    publisher.close();
    wait_slots(&f, 2).await;
    // Core retires the original publisher and reconciles the pinned intent.
    // Its already-negotiated unused receive port remains allocated; no new
    // physical parameters means no unnecessary reoffer on source EOF.
    let end = tokio::time::Instant::now() + Duration::from_secs(4);
    loop {
        let observed:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM operations WHERE rowid>?1 AND command_type='stream.reconcile' AND connection_id='future-target' AND state='completed' AND json_extract(result_json,'$.resources_changed')=0)",[before],|r|r.get(0)).unwrap();
        if observed {
            break;
        }
        assert!(
            tokio::time::Instant::now() < end,
            "source removal reconciliation not durably observed"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        f.server
            .service
            .canonical_runtime_snapshot()
            .unwrap()
            .connections,
        2
    );
    assert_audio_reaches_other_participant(&mut clients, sh, &[th], 28600).await;
    for client in &mut clients {
        client.close();
    }
    f.server.shutdown().await.unwrap();
}

async fn assert_vp8_absent(
    source: &CanonicalClient,
    target: &mut CanonicalClient,
    sh: u16,
    th: u16,
) {
    use rvoip_uctp::substrate::{observe_rtp_datagram, pack, MediaDatagram};
    let frame = include_bytes!("../../fixtures/gate6-vp8-black-640x360.vp8");
    let sending = async {
        for sequence in 100..140u16 {
            let mut rtp = vec![0x80, 0x80 | 96];
            rtp.extend_from_slice(&sequence.to_be_bytes());
            rtp.extend_from_slice(&(u32::from(sequence) * 9000).to_be_bytes());
            rtp.extend_from_slice(&0x6476_7001u32.to_be_bytes());
            rtp.push(0x10);
            rtp.extend_from_slice(frame);
            source
                .send_native_media_datagram(&pack(&MediaDatagram {
                    flags: 0,
                    stream_local_id: sh,
                    seq: u32::from(sequence),
                    payload: rtp.into(),
                }))
                .unwrap();
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    };
    let receiving = async {
        let start = tokio::time::Instant::now();
        while start.elapsed() < Duration::from_millis(1100) {
            match tokio::time::timeout(
                Duration::from_millis(40),
                target.receive_native_media_datagram(),
            )
            .await
            {
                Ok(Ok(bytes)) => {
                    let p = observe_rtp_datagram(&bytes).unwrap();
                    // Drain packets which crossed before the commit. Once
                    // drained, continued fresh video must remain absent.
                    assert!(
                        !(p.stream_local_id == th && start.elapsed() >= Duration::from_millis(300)),
                        "removed camera still routed"
                    );
                }
                Ok(Err(error)) => panic!("unsubscribe closed receiver: {error:?}"),
                Err(_) => (),
            }
        }
    };
    tokio::join!(sending, receiving);
}
