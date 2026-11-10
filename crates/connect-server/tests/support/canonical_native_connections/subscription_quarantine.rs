//! Actual Service/UCTP resource wait, real Store writer and fresh activation.
use super::*;
use connect_store::{AuthorityMode, AuthorityScope, AuthorityWait};

#[tokio::test]
async fn production_live_subscription_prepares_without_domain_reader_then_activates_original_once()
{
    scenario(false).await;
}
#[tokio::test]
async fn production_live_subscription_fresh_video_withdrawal_refuses_cut_and_preserves_original_audio(
) {
    scenario(true).await;
}

fn local_handle(answer: &Envelope, stream: &str) -> u16 {
    answer.original()["payload"]["substrate_setup"]["streams"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["strm_id"] == stream)
        .unwrap()["stream_local_id"]
        .as_u64()
        .unwrap() as u16
}
async fn observe(
    f: &Fixture,
    credential: &str,
    until: tokio::time::Instant,
) -> connect_protocol::transitions::OperationRecord {
    assert!(
        tokio::time::Instant::now() < until,
        "original subscription observation cutoff"
    );
    let store = f.server.store.clone();
    let credential = credential.to_owned();
    // Join the actual read even if its bounded Store wait crosses the original
    // cutoff. Dropping a timeout-wrapped JoinHandle would detach that owner.
    let observed = tokio::task::spawn_blocking(move || {
        store.execute_operation_action(
            &credential,
            connect_protocol::transitions::OperationAction::Get {
                operation_id: "quarantine-add-video".into(),
            },
        )
    })
    .await
    .expect("original subscription observation task joined");
    assert!(
        tokio::time::Instant::now() < until,
        "original subscription observation cutoff"
    );
    observed.expect("observe same original operation")
}
fn target_state(f: &Fixture) -> (bool, u64, String, String) {
    rusqlite::Connection::open_with_flags(&f.config.database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .unwrap().query_row(
        "SELECT state IN ('ready','active'),generation,observed_runtime_id,readiness_json FROM connections WHERE id='quarantine-target'",
        [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
    ).unwrap()
}

async fn scenario(withdraw_video: bool) {
    let f = Fixture::with_native_stream_scope("EdDSA", 12).await;
    ready_endpoint(&f).await;
    let config = party_with_capabilities(
        &f,
        &f.peer.identity_id,
        211,
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
    let target_answer = negotiate_av(&mut target, &cid, "quarantine-target-offer", "quarantine-target",
        json!([{"id":"quarantine-monitor","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}])).await;
    let source_answer = negotiate_av(&mut source, &cid, "quarantine-source-offer", "quarantine-source",
        json!([{"id":"quarantine-voice","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]},
               {"id":"quarantine-camera","kind":"video","direction":"sendonly","codec_preferences":["vp8"]}])).await;
    wait_native_operation(&f, &f.client.credential, &cid, "quarantine-target-offer").await;
    wait_native_operation(&f, &config.credential, &cid, "quarantine-source-offer").await;
    stream_control(
        &mut target,
        &cid,
        "quarantine-target",
        "quarantine-select-audio",
        1,
        "stream.subscribe",
        json!({"subscriptions":[{"strm_id":"quarantine-voice"}]}),
    )
    .await;
    let sh = local_handle(&source_answer, "quarantine-voice");
    let th = local_handle(&target_answer, "quarantine-monitor");
    let vh = local_handle(&source_answer, "quarantine-camera");
    let mut clients = vec![source, target];
    assert_audio_reaches_other_participant(&mut clients, sh, &[th], 41000).await;
    let before = target_state(&f);
    let credential = f
        .server
        .store
        .authenticate(&f.client.credential)
        .unwrap()
        .credential_id;
    let scope = CommandScope {
        conversation_id: Some(cid.clone()),
        session_id: Some("native-session".into()),
        connection_id: Some("quarantine-target".into()),
        operation_id: Some("quarantine-add-video".into()),
        expected_revision: Some(2),
    };
    // One existing eight-second control budget: no extra offer, resubmission or
    // renewed deadline while the actual native adapter awaits its signed answer.
    let until = tokio::time::Instant::now() + Duration::from_secs(8);
    let input =
        json!({"subscriptions":[{"strm_id":"quarantine-voice"},{"strm_id":"quarantine-camera"}]});
    let request = tokio::time::timeout_at(
        until,
        clients[1].send_value("stream.subscribe", input.clone(), scope.clone()),
    )
    .await
    .unwrap()
    .unwrap();
    let callback = loop {
        let frame = tokio::time::timeout_at(until, clients[1].receive_control())
            .await
            .unwrap()
            .unwrap();
        match frame.kind() {
            "connection.offer" => break frame,
            "operation.updated" => {
                assert_eq!(frame.original()["in_reply_to"], request);
                assert_eq!(
                    frame.original()["payload"]["operation_id"],
                    "quarantine-add-video"
                );
                assert!(matches!(
                    frame.original()["payload"]["state"].as_str(),
                    Some("accepted" | "preparing")
                ));
            }
            "ack" => assert_eq!(frame.original()["in_reply_to"], request),
            _ => panic!("unexpected original subscription frame: {}", frame.kind()),
        }
    };
    assert_eq!(
        callback.scope(rvoip_uctp::v2::Scope::Operation),
        Some("quarantine-add-video")
    );
    assert_eq!(
        callback.original()["payload"]["substrate_setup"]["revision"],
        3
    );
    assert_eq!(
        target_state(&f),
        before,
        "unanswered resource preparation cannot publish a new route"
    );
    let preparing = observe(&f, &credential, until).await;
    assert_eq!(
        preparing.state,
        connect_protocol::transitions::OperationState::Preparing
    );
    assert_eq!(preparing.effects.len(), 1);
    assert_eq!(preparing.effects[0].attempt, 1);
    // Actual Audio remains active while the adapter's real Video preparation is
    // stalled on the SAME signed answer, rather than a synthetic test callback.
    tokio::time::timeout_at(
        until,
        assert_audio_reaches_other_participant(&mut clients, sh, &[th], 41200),
    )
    .await
    .unwrap();

    let store = f.server.store.clone();
    let identity = f.client.identity_id.clone();
    let room = cid.clone();
    let writer = tokio::task::spawn_blocking(move || {
        let held = store.acquire_authority(
            &[
                AuthorityScope::identity(&identity, AuthorityMode::Write)?,
                AuthorityScope::conversation(&room, AuthorityMode::Write)?,
            ],
            &AuthorityWait::new(Duration::from_millis(500)),
        )?;
        if withdraw_video {
            store.withdraw_native_session_video_for_test(&held, "quarantine-target")?;
        }
        drop(held);
        connect_protocol::Result::Ok(())
    })
    .await
    .expect("original writer task joined");
    let writer_progressed = writer.is_ok();
    // Counterfactual writer refusal is asserted only AFTER original peers and
    // server join, so even the old held-reader implementation retains custody.
    let withdrawn = withdraw_video && writer_progressed;
    let p = &callback.original()["payload"];
    let streams = p["substrate_setup"]["streams"].as_array().unwrap().iter().map(|s| json!({
        "id":s["strm_id"], "kind":s["kind"], "direction":match s["direction"].as_str().unwrap() {"sendonly"=>"recvonly","recvonly"=>"sendonly",_=>"sendrecv"}, "codec":s["codec"]
    })).collect::<Vec<_>>();
    let answer = serde_json::from_value(json!({"by_participant":p["by_participant"],"substrate":"quic","capabilities":av_capabilities(),"streams_answered":streams,"substrate_setup":p["substrate_setup"]})).unwrap();
    tokio::time::timeout_at(
        until,
        clients[1].answer_connection_offer(&callback, answer, &scope),
    )
    .await
    .unwrap()
    .unwrap();
    if withdrawn {
        // The resource token cannot act as permission; a fresh ordinary authority
        // plus the exact original plan must refuse newly withdrawn Video.
        loop {
            let frame = tokio::time::timeout_at(until, clients[1].receive_control())
                .await
                .unwrap()
                .unwrap();
            assert_ne!(
                frame.kind(),
                "connection.offer",
                "no second preparation/offer after refusal"
            );
            assert_eq!(frame.original()["in_reply_to"], request);
            if frame.kind() == "error" {
                break;
            }
            assert_eq!(frame.kind(), "operation.updated");
            assert_ne!(frame.original()["payload"]["state"], "completed");
        }
        // A response may precede completion of the same owned historical
        // recorder under transient metadata pressure. Observe only its SAME
        // operation within the already-running original control deadline.
        let after = loop {
            assert!(
                tokio::time::Instant::now() < until,
                "original subscription outcome cutoff"
            );
            let db = f.server.store.clone();
            let key = credential.clone();
            let observed = tokio::task::spawn_blocking(move || {
                db.execute_operation_action(
                    &key,
                    connect_protocol::transitions::OperationAction::Get {
                        operation_id: "quarantine-add-video".into(),
                    },
                )
            })
            .await
            .expect("same original read-only observation joined");
            assert!(
                tokio::time::Instant::now() < until,
                "original subscription outcome cutoff"
            );
            match observed {
                Ok(value) => {
                    assert_eq!(value.record_id, preparing.record_id);
                    assert_eq!(value.effects.len(), 1);
                    assert_eq!(value.effects[0].id, preparing.effects[0].id);
                    if value.state == connect_protocol::transitions::OperationState::Unknown {
                        break value;
                    }
                    assert_eq!(
                        value.state,
                        connect_protocol::transitions::OperationState::Preparing
                    );
                }
                Err(error)
                    if matches!(
                        error.code.as_str(),
                        "authority-busy" | "storage-unavailable"
                    ) =>
                {
                    ()
                }
                Err(error) => panic!("original subscription observation refused: {error:?}"),
            }
            tokio::time::sleep_until(
                (tokio::time::Instant::now() + Duration::from_millis(20)).min(until),
            )
            .await;
        };
        assert_eq!(after.record_id, preparing.record_id);
        assert_eq!(after.effects.len(), 1);
        assert_eq!(after.effects[0].id, preparing.effects[0].id);
        assert_eq!(after.effects[0].attempt, 1);
        assert_eq!(after.effects[0].submission_generation, 1);
        assert_eq!(
            target_state(&f),
            before,
            "refused activation leaves old bindings/generation intact"
        );
    } else {
        let (extra_offer, complete) = finish_original_control(
            &mut clients[1],
            scope,
            request,
            "stream.subscribe",
            input,
            "completed",
            None,
            until,
        )
        .await;
        assert!(
            extra_offer.is_none(),
            "same prepared resources activate without another offer"
        );
        assert_eq!(
            complete.original()["payload"]["record_id"],
            preparing.record_id
        );
        let after = observe(&f, &credential, until).await;
        assert_eq!(after.effects.len(), 1);
        assert_eq!(after.effects[0].id, preparing.effects[0].id);
        assert_eq!(after.result.as_ref().unwrap()["resources_changed"], true);
        assert_eq!(after.result.as_ref().unwrap()["stream_revision"], 3);
        let video = local_handle(&callback, "quarantine-camera");
        let (sources, targets) = clients.split_at_mut(1);
        assert_vp8_routes(&sources[0], &mut targets[0], vh, video).await;
    }
    assert_audio_reaches_other_participant(&mut clients, sh, &[th], 41400).await;
    wait_slots(&f, 2).await;
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let counts: (u64,u64) = db.query_row("SELECT COUNT(DISTINCT o.record_id),COUNT(e.id) FROM operations o LEFT JOIN operation_effects e ON e.operation_record_id=o.record_id WHERE o.operation_id='quarantine-add-video'", [], |r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(
        counts,
        (1, 1),
        "same original logical reservation and effect only"
    );
    drop(db);
    for client in clients {
        client.shutdown().await.unwrap();
    }
    f.server.shutdown().await.unwrap();
    assert!(
        writer_progressed,
        "actual domain writer or current session acceptance mutation failed during original resource wait: {writer:?}"
    );
}

#[path = "subscription_quarantine/overlap.rs"]
mod overlap;
