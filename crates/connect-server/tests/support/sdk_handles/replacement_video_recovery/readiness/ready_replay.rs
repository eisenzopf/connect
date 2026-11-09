//! Signed readiness replay retains its original owner across durable projection.
use super::surviving_epoch::{close_decoder, SqlOwner};
use super::*;

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
    cutoff: Instant,
) -> std::result::Result<(), String> {
    let preparation = client.native_replacement_readiness().unwrap();
    io.reset_video_receive_decoder(incoming).unwrap();
    let mut decoder = Decoder::open(encoder, cutoff);
    let mut sql = None;
    let mut pause = None;
    let mut refused_while_held = Vec::new();
    let mut preparing_replay = None;
    let mut completed_replay = None;
    let work = AssertUnwindSafe(async {
        let samples = audio(io, &preparation, cutoff).await;
        let key = encoder.frame(1, true).await;
        publish(source, &key, 1, sequence).await;
        let (frame, _, arrived) = video(io, incoming, 1, cutoff).await;
        assert_eq!(frame.encoded, key);
        decoder.frame(&frame, 1, arrived, cutoff).await;
        let ready = preparation
            .with_decoded(
                samples,
                Some(NativeDecodedVideoReadiness {
                    decoded_frames: 1,
                    decoded_keyframes: 1,
                }),
            )
            .unwrap();
        let record = root.record_id.as_ref().unwrap().as_str();
        pause = Some(
            f.server
                .service
                .pause_canonical_replacement_projection_for_test(record)
                .unwrap(),
        );
        submit(client, &ready, controller, command, root, cutoff).await;
        timeout_at(cutoff, pause.as_ref().unwrap().entered())
            .await
            .unwrap();
        // The existing pause follows the actual Core cut and precedes the
        // Known capture. Separate packet observers remain available to peers.
        let original = original_cutoff(f, root);
        let source_runtime: String = rusqlite::Connection::open(&f.config.database)
            .unwrap()
            .query_row(
                "SELECT runtime_connection_id FROM connection_offers WHERE connection_id=?1",
                [command.connection_id.as_ref().unwrap()],
                |row| row.get(0),
            )
            .unwrap();
        let session = f
            .server
            .service
            .canonical_session_metadata_for_test("native-session")
            .unwrap();
        {
            let current = session.read().unwrap();
            assert!(!current
                .connections
                .contains_key(&rvoip_core::ids::ConnectionId::from_string(source_runtime)));
            assert!(current
                .connections
                .contains_key(&rvoip_core::ids::ConnectionId::from_string(
                    original.candidate_runtime_id.clone()
                )));
        }
        let owners = f
            .server
            .service
            .canonical_replacement_weak_owners_for_test(record)
            .unwrap();
        assert!(owners.iter().all(|owner| owner.strong_count() > 0));
        sql = Some(SqlOwner::acquire(f.server.store.clone()));
        // Span several existing 100ms watcher turns. Every attempt is the
        // same frozen signed proof; none allocates or renews a preparation.
        let observe_until = cutoff.min(Instant::now() + Duration::from_millis(400));
        loop {
            let result = timeout_at(cutoff, client.confirm_native_replacement_ready(&ready))
                .await
                .unwrap();
            let code = match result {
                Err(ClientError::Application(problem)) => problem.code,
                Ok(_) => panic!("held original SQL unexpectedly accepted readiness replay"),
                Err(_) => panic!("held original SQL did not return an application refusal"),
            };
            refused_while_held.push(code);
            assert!(owners.iter().all(|owner| owner.strong_count() > 0));
            if Instant::now() >= observe_until {
                break;
            }
            timeout_at(cutoff, tokio::time::sleep(Duration::from_millis(40)))
                .await
                .unwrap();
        }
        assert!(sql.as_mut().unwrap().finish());
        sql.take();
        // Source retirement uses the same primary SQL mutex. Let its existing
        // custodian settle while the actual Core commit remains paused before
        // Known capture publication, then span the existing watcher cadence.
        timeout_at(cutoff, async {
            loop {
                let state: String = rusqlite::Connection::open(&f.config.database)
                    .unwrap()
                    .query_row(
                        "SELECT state FROM connections WHERE id=?1",
                        [command.connection_id.as_ref().unwrap()],
                        |row| row.get(0),
                    )
                    .unwrap();
                if state == "retired" {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("original source retirement did not settle while capture was paused");
        let observe_until = cutoff.min(Instant::now() + Duration::from_millis(220));
        loop {
            preparing_replay = Some(client.confirm_native_replacement_ready(&ready).await);
            if Instant::now() >= observe_until {
                break;
            }
            timeout_at(cutoff, tokio::time::sleep(Duration::from_millis(40)))
                .await
                .unwrap();
        }
        assert!(owners.iter().all(|owner| owner.strong_count() > 0));
        pause.as_ref().unwrap().release();
        timeout_at(cutoff, pause.as_ref().unwrap().left())
            .await
            .unwrap();
        let done = timeout_at(cutoff, completed(f, controller, command))
            .await
            .unwrap();
        assert_eq!(done.record_id, root.record_id);
        assert_eq!(done.operation_id, root.operation_id);
        completed_replay = Some(client.confirm_native_replacement_ready(&ready).await);
    })
    .catch_unwind()
    .await;
    // Release and join the exact SQL holder and decoder before the old-body
    // causal assertion. The outer fixture joins peers, clients and server.
    let sql_joined = sql.as_mut().is_none_or(SqlOwner::finish);
    drop(sql);
    drop(pause);
    let decoder_joined = close_decoder(decoder, cutoff).await;
    if !sql_joined || !decoder_joined {
        return Err("original SQL or decoder owner did not join naturally".into());
    }
    if let Err(panic) = work {
        return Err(panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| {
                panic
                    .downcast_ref::<&str>()
                    .map(|value| (*value).to_owned())
            })
            .unwrap_or_else(|| "original readiness replay audit panicked".into()));
    }
    let phases = [&preparing_replay, &completed_replay].map(|value| match value {
        Some(Ok(value)) => format!("ok:{}", serde_json::to_value(value).unwrap()["state"]),
        Some(Err(ClientError::Application(problem))) => format!("refused:{}", problem.code),
        Some(Err(_)) => "client-error".into(),
        None => "not-attempted".into(),
    });
    eprintln!(
        "signed original readiness replay: held_codes={refused_while_held:?} preparing={} completed={}",
        phases[0], phases[1],
    );
    if refused_while_held.is_empty()
        || refused_while_held
            .iter()
            .any(|code| !matches!(code.as_str(), "authority-busy" | "storage-unavailable"))
    {
        return Err(format!(
            "committed original readiness lost registry custody: held={refused_while_held:?}"
        ));
    }
    let preparing = preparing_replay
        .ok_or_else(|| "preparing replay was not attempted".to_owned())?
        .map_err(|error| format!("preparing original readiness replay refused: {error}"))?;
    let completed = completed_replay
        .ok_or_else(|| "completed replay was not attempted".to_owned())?
        .map_err(|error| format!("completed original readiness replay refused: {error}"))?;
    for value in [&preparing, &completed] {
        if value.record_id != root.record_id || value.operation_id != root.operation_id {
            return Err("readiness replay changed the original logical root".into());
        }
    }
    if serde_json::to_value(preparing).unwrap()["state"] != "preparing"
        || serde_json::to_value(completed).unwrap()["state"] != "completed"
    {
        return Err("readiness replay changed the actual pending/completed phase".into());
    }
    Ok(())
}

pub(in super::super) async fn cancelled_owner(
    f: &Fixture,
    original: &OriginalPreparation,
    probe: &std::sync::atomic::AtomicUsize,
    encoder: &mut Encoder,
    source: &sdk::Stream,
    source_receiver: &sdk::Stream,
    retained: &sdk::Stream,
    retained_receiver: &sdk::Stream,
    source_sequence: &mut u16,
    retained_sequence: &mut u16,
    cutoff: Instant,
) -> std::result::Result<(), String> {
    let work = AssertUnwindSafe(async {
        timeout_at(cutoff, async {
            while probe.load(std::sync::atomic::Ordering::Acquire) == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("scoped original native owner was not reached");
        original_target_retired_under(f, original, true).await;
        let root = rusqlite::Connection::open(&f.config.database).unwrap();
        let committed: usize = root
            .query_row(
                "SELECT count(*) FROM operation_effects WHERE operation_record_id=?1 AND provider='native-replacement-commit'",
                [&original.record_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(committed, 0, "cancelled preparation reached Core commit");
        // Both original public directions remain real and current after the
        // candidate's existing watcher finishes durable compensation.
        let key = encoder.frame(1, true).await;
        publish(source, &key, 1, source_sequence).await;
        let (actual, _) = receive(source_receiver, 1).await;
        assert_eq!(actual, key);
        let key = encoder.frame(2, true).await;
        publish(retained, &key, 2, retained_sequence).await;
        let (actual, _) = receive(retained_receiver, 2).await;
        assert_eq!(actual, key);
    })
    .catch_unwind()
    .await;
    work.map_err(|panic| {
        panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| {
                panic
                    .downcast_ref::<&str>()
                    .map(|value| (*value).to_owned())
            })
            .unwrap_or_else(|| "original native owner cancellation audit panicked".into())
    })
}
