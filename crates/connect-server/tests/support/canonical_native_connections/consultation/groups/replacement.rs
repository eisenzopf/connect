//! Public replacement now owns a real Core ticket, with original media intact.
use super::*;
async fn wait_prepared(f: &Fixture, key: &str, operation: &str) -> OperationRecord {
    let until = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let record = status(f, key, operation).await;
        if record
            .effects
            .first()
            .is_some_and(|e| e.state == EffectState::Confirmed)
        {
            return record;
        }
        assert!(
            tokio::time::Instant::now() < until,
            "preparation did not settle: {record:?}"
        );
        assert!(
            !matches!(
                record.state,
                OperationState::Failed | OperationState::Unknown | OperationState::Cancelled
            ),
            "{record:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
async fn stopped(f: &Fixture, key: &str, op: &str, state: OperationState) {
    let until = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let r = status(f, key, op).await;
        if r.state == state
            && f.server
                .service
                .canonical_runtime_snapshot()
                .unwrap()
                .replacement_preparations
                == 0
        {
            assert!(r.result.is_none());
            return;
        }
        assert!(
            tokio::time::Instant::now() < until,
            "preparation not stopped: {r:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
#[tokio::test]
async fn production_replacement_prepares_actual_core_once_preserves_audio_and_cancels_without_allocating(
) {
    exercise_replacement(false).await;
}
#[tokio::test]
async fn production_replacement_allocates_exact_core_reservation_from_signed_target_without_media_or_duplicate_io(
) {
    exercise_replacement(true).await;
}
#[tokio::test]
async fn production_replacement_negotiates_generation_two_resources_with_signed_target_confirmation_and_initial_mute(
) {
    exercise_replacement_stage(true, true).await;
}
async fn exercise_replacement(allocate: bool) {
    exercise_replacement_stage(allocate, false).await;
}
#[tokio::test]
async fn production_replacement_bad_signed_parameters_retire_uncertain_native_start_without_retry()
{
    exercise_replacement_phase(true, true, true).await;
}
async fn exercise_replacement_stage(allocate: bool, start_media: bool) {
    exercise_replacement_phase(allocate, start_media, false).await;
}
#[tokio::test]
async fn production_replacement_actual_movement_keeps_generation_two_audio_after_old_authority_retires(
) {
    exercise_replacement_mode(true, true, false, true, false).await;
}
async fn exercise_replacement_phase(allocate: bool, start_media: bool, reject_answer: bool) {
    exercise_replacement_mode(allocate, start_media, reject_answer, false, false).await;
}
async fn exercise_replacement_mode(
    allocate: bool,
    start_media: bool,
    reject_answer: bool,
    movement: bool,
    deferred_projection: bool,
) {
    exercise_replacement_with_sdk(
        allocate,
        start_media,
        reject_answer,
        movement,
        deferred_projection,
        None,
    )
    .await
}
pub(super) async fn exercise_replacement_with_sdk(
    allocate: bool,
    start_media: bool,
    reject_answer: bool,
    movement: bool,
    deferred_projection: bool,
    sdk_algorithm: Option<&'static str>,
) {
    let cases = if deferred_projection {
        vec![(false, false, false)]
    } else if movement {
        vec![
            (false, false, false),
            (true, false, false),
            (false, true, false),
            (false, false, true),
        ]
    } else if reject_answer {
        vec![(false, false, false)]
    } else if allocate {
        vec![
            (false, false, false),
            (true, false, false),
            (false, true, false),
            (false, false, true),
        ]
    } else {
        vec![
            (false, false, false),
            (true, false, false),
            (false, true, false),
        ]
    };
    for (observer, revoke_endpoint, revoke_target_key) in cases {
        let codecs::Running {
            f,
            mut clients,
            cid,
            sh,
            th,
        } = if let Some(algorithm) = sdk_algorithm {
            codecs::setup_started(
                Fixture::with_native_end_scope(algorithm, 16).await,
                (),
                "codec-target-offer",
                "codec-source-offer",
            )
            .await
            .0
        } else {
            codecs::setup().await
        };
        let room = clients[1]
            .list_conversations(Default::default())
            .await
            .unwrap()
            .conversations
            .into_iter()
            .find(|r| r.id == cid)
            .unwrap();
        let owner = room
            .participants
            .iter()
            .find(|p| p.identity_id == f.client.identity_id)
            .unwrap()
            .participant_id
            .clone();
        let remote = room
            .participants
            .iter()
            .find(|p| p.identity_id == f.peer.identity_id)
            .unwrap()
            .participant_id
            .clone();
        if observer {
            clients[1]
                .session_command(Request {
                    operation_id: "replacement-source-observer".into(),
                    conversation_id: cid.clone(),
                    session_id: "native-session".into(),
                    by_participant: None,
                    expected_revision: Some(revision(&f)),
                    action: Action::Role {
                        participant_id: owner.clone(),
                        role: "observer".into(),
                    },
                })
                .await
                .unwrap();
        }
        let mut target_scope = Scope::collaboration(vec!["*".into()]);
        if movement {
            target_scope.operations.push("stream.subscribe".into());
        }
        let target = party_with_scope(
            &f,
            &f.client.identity_id,
            281,
            vec!["audio".into()],
            target_scope,
        );
        let endpoint: String = rusqlite::Connection::open(&f.config.database)
            .unwrap()
            .query_row(
                "SELECT id FROM endpoints WHERE device_id=?1",
                [&target.device_id],
                |r| r.get(0),
            )
            .unwrap();
        let primary = clients.pop().unwrap();
        let other = clients.pop().unwrap();
        let make = |client, identity, participant, id: &str, handle| Peer {
            client,
            config: None,
            identity,
            participant,
            id: id.into(),
            handle,
            clock: tokio::time::Instant::now(),
            sequence: 1,
            live: true,
        };
        let mut peers = vec![
            make(
                other,
                f.peer.identity_id.clone(),
                remote,
                "codec-source",
                sh,
            ),
            make(
                primary,
                f.client.identity_id.clone(),
                owner.clone(),
                "codec-target",
                th,
            ),
        ];
        probe(&mut peers, 0, &[1], "replace-before-prepare-remote").await;
        probe(
            &mut peers,
            1,
            if observer { &[] } else { &[0] },
            "replace-before-prepare-source",
        )
        .await;
        let baseline = f.server.service.canonical_runtime_snapshot().unwrap();
        let mut scope = Scope::collaboration(vec!["*".into()]);
        scope
            .operations
            .extend(["connection.replace".into(), "operation.cancel".into()]);
        let mut key = f
            .server
            .store
            .issue(
                &f.client.identity_id,
                scope.clone(),
                chrono::Utc::now().timestamp() + 600,
            )
            .unwrap();
        let mut cfg = canonical_http_config(&f);
        cfg.credential = key.token.clone();
        let mut controller = CanonicalHttpClient::open(cfg).await.unwrap();
        let command = TransitionCommand {
            operation_id: "actual-managed-replacement".into(),
            conversation_id: cid.clone(),
            session_id: "native-session".into(),
            connection_id: Some("codec-target".into()),
            expected_revision: revision(&f),
            intent: TransitionIntent::Replace {
                endpoint_id: Some(endpoint.clone()),
                candidate_connid: None,
                retained_connid: "codec-source".into(),
            },
            payload_extensions: Default::default(),
        };
        let sdk_probe = if sdk_algorithm.is_some() {
            Some(super::sdk_movements::replacement(&f, &command, &key.token).await)
        } else {
            None
        };
        let public_root = if sdk_probe.is_some() {
            controller
                .operation_get(
                    &command.operation_id,
                    CommandScope {
                        conversation_id: Some(cid.clone()),
                        session_id: Some(command.session_id.clone()),
                        connection_id: command.connection_id.clone(),
                        ..Default::default()
                    },
                )
                .await
                .unwrap()
        } else {
            controller.transition(command.clone()).await.unwrap()
        };
        let prepared = wait_prepared(&f, &key.id, &command.operation_id).await;
        assert_eq!(prepared.state, OperationState::Preparing);
        assert!(prepared.result.is_none());
        assert_eq!(prepared.effects.len(), 1);
        assert_eq!(prepared.effects[0].attempt, 1);
        controller.transition(command.clone()).await.unwrap();
        assert_eq!(
            status(&f, &key.id, &command.operation_id).await.effects,
            prepared.effects
        );
        let current = f.server.service.canonical_runtime_snapshot().unwrap();
        assert_eq!(current.replacement_preparations, 1);
        let before = sdk_probe.as_ref().map(|p| p.baseline).unwrap_or((
            baseline.connections,
            baseline.physical_peers,
            baseline.live_sessions,
        ));
        assert_eq!(current.connections, before.0);
        assert_eq!(current.physical_peers, before.1);
        assert_eq!(current.live_sessions, before.2);
        let c = rusqlite::Connection::open(&f.config.database).unwrap();
        let (plan,observation):(String,String)=c.query_row("SELECT n.plan_json,s.observation_json FROM native_replacements n JOIN native_replacement_steps s ON s.operation_record_id=n.operation_record_id AND s.phase='prepare'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        let plan: Value = serde_json::from_str(&plan).unwrap();
        let observation: Value = serde_json::from_str(&observation).unwrap();
        assert_eq!(observation, plan["expected_preparation"]);
        assert_eq!(plan["generation"], 2);
        let candidate = plan["candidate_connection_id"].as_str().unwrap();
        let exists: bool = c
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM connections WHERE id=?1)",
                [candidate],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!exists);
        drop(c);
        // Real ticket ownership retains no idle Store grant. A bounded writer
        // may temporarily deny fresh polling without destroying the ticket.
        let until = tokio::time::Instant::now() + Duration::from_secs(2);
        let writer =
            loop {
                match f.server.store.try_acquire_authority(&[
                    connect_store::AuthorityScope::identity(
                        &f.client.identity_id,
                        connect_store::AuthorityMode::Write,
                    )
                    .unwrap(),
                ]) {
                    Ok(w) => break w,
                    Err(_) => {
                        assert!(tokio::time::Instant::now() < until);
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                }
            };
        tokio::time::sleep(Duration::from_millis(250)).await;
        drop(writer);
        assert_eq!(
            f.server
                .service
                .canonical_runtime_snapshot()
                .unwrap()
                .replacement_preparations,
            1
        );
        let mut replacement_handle = None;
        let mut allocated_target = if allocate {
            let mut client = CanonicalClient::open(target.clone()).await.unwrap();
            let offer = json!({"substrate":"quic","capabilities":capabilities(),"streams_offered":[{"id":"replacement-voice","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}],"replacement_preparation":{"profile":"connect-replacement-preparation-v1","operation_record_id":prepared.record_id}});
            let offer_scope = CommandScope {
                conversation_id: Some(cid.clone()),
                session_id: Some("native-session".into()),
                connection_id: Some(candidate.into()),
                operation_id: Some(command.operation_id.clone()),
                expected_revision: Some(0),
            };
            client
                .allocate_native_replacement_target(
                    &command,
                    &public_root,
                    serde_json::from_value(offer.clone()).unwrap(),
                )
                .await
                .unwrap();
            let allocated = status(&f, &key.id, &command.operation_id).await;
            assert_eq!(allocated.state, OperationState::Preparing);
            assert_eq!(allocated.effects.len(), 2);
            assert!(allocated
                .effects
                .iter()
                .all(|e| e.state == EffectState::Confirmed && e.attempt == 1));
            assert!(allocated.result.is_none());
            client
                .request_value("connection.offer", offer.clone(), offer_scope.clone())
                .await
                .unwrap()
                .ack()
                .unwrap();
            assert_eq!(
                status(&f, &key.id, &command.operation_id).await.effects,
                allocated.effects
            );
            let mut wrong = offer.clone();
            wrong["streams_offered"][0]["direction"] = json!("sendonly");
            assert!(client
                .request_value("connection.offer", wrong, offer_scope.clone())
                .await
                .unwrap()
                .ack()
                .is_err());
            let mut other = CanonicalClient::open(target.clone()).await.unwrap();
            assert!(other
                .request_value("connection.offer", offer.clone(), offer_scope.clone())
                .await
                .unwrap()
                .ack()
                .is_err());
            other.shutdown().await.unwrap();
            let snapshot = f.server.service.canonical_runtime_snapshot().unwrap();
            assert_eq!(snapshot.connections, baseline.connections + 1);
            assert_eq!(snapshot.live_sessions, baseline.live_sessions);
            assert_eq!(snapshot.replacement_preparations, 1);
            let c = rusqlite::Connection::open(&f.config.database).unwrap();
            let captured: String = c
                .query_row(
                    "SELECT observation_json FROM native_replacement_steps WHERE phase='allocate'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            let captured: Value = serde_json::from_str(&captured).unwrap();
            assert_eq!(captured["candidate"], plan["candidate_runtime_id"]);
            assert_eq!(captured["generation"], 2);
            assert_eq!(captured["allocated"], true);
            assert_eq!(
                c.query_row("SELECT COUNT(*) FROM connections", [], |r| r
                    .get::<_, u64>(0))
                    .unwrap(),
                2
            );
            drop(c);
            if start_media {
                let (request_id, scope) = client
                    .begin_native_replacement_media(
                        &command,
                        &public_root,
                        serde_json::from_value(offer.clone()).unwrap(),
                    )
                    .await
                    .unwrap();
                let answer = client.receive_control().await.unwrap();
                assert_eq!(
                    answer.kind(),
                    "connection.answer",
                    "{:?}",
                    answer.original()
                );
                assert_eq!(answer.original()["in_reply_to"], request_id);
                if !reject_answer {
                    replacement_handle = Some(handle(&answer));
                }
                let callback = client.receive_control().await.unwrap();
                assert_eq!(
                    callback.kind(),
                    "connection.offer",
                    "{:?}",
                    callback.original()
                );
                let payload = &callback.original()["payload"];
                let streams=payload["substrate_setup"]["streams"].as_array().unwrap().iter().map(|v|json!({"id":v["strm_id"],"kind":v["kind"],"direction":match v["direction"].as_str().unwrap(){"sendonly"=>"recvonly","recvonly"=>"sendonly",_=>"sendrecv"},"codec":v["codec"]})).collect::<Vec<_>>();
                let mut response = json!({"by_participant":payload["by_participant"],"substrate":"quic","capabilities":capabilities(),"streams_answered":streams,"substrate_setup":payload["substrate_setup"]});
                if reject_answer {
                    response["substrate_setup"]["revision"] = json!(999);
                }
                client
                    .answer_connection_offer(
                        &callback,
                        serde_json::from_value(response).unwrap(),
                        &scope,
                    )
                    .await
                    .unwrap();
                if reject_answer {
                    stopped(&f, &key.id, &command.operation_id, OperationState::Failed).await;
                } else {
                    // The offer was answered during negotiation; it has no invented
                    // final ACK/connection.ready. Resource capture remains preparing
                    // and is observed through the original operation status.
                    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
                    let started = loop {
                        let current = status(&f, &key.id, &command.operation_id).await;
                        if current.effects.len() == 5
                            && current
                                .effects
                                .iter()
                                .all(|e| e.state == EffectState::Confirmed)
                        {
                            break current;
                        }
                        assert!(
                            tokio::time::Instant::now() < deadline,
                            "Replacement resource capture did not settle: {current:?}"
                        );
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    };
                    assert_eq!(started.state, OperationState::Preparing);
                    assert!(started.result.is_none());
                    assert_eq!(started.effects.len(), 5);
                    assert!(started
                        .effects
                        .iter()
                        .all(|e| e.state == EffectState::Confirmed && e.attempt == 1));
                    let c = rusqlite::Connection::open(&f.config.database).unwrap();
                    let actual: String = c
                    .query_row(
                        "SELECT observation_json FROM native_replacement_steps WHERE phase='start'",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap();
                    let actual: Value = serde_json::from_str(&actual).unwrap();
                    assert_eq!(actual["generation"], 2);
                    assert_eq!(actual["stream_revision"], 2);
                    assert_eq!(actual["prepared_muted"], true);
                    assert_eq!(actual["media_ready"], false);
                    let mut replay = offer.clone();
                    replay["replacement_preparation"]["profile"] =
                        json!("connect-replacement-media-v1");
                    drop(c);
                    client
                        .request_value("connection.offer", replay, scope)
                        .await
                        .unwrap()
                        .ack()
                        .unwrap();
                    assert_eq!(
                        status(&f, &key.id, &command.operation_id).await.effects,
                        started.effects
                    );
                }
            }
            Some(client)
        } else {
            None
        };
        probe(
            &mut peers,
            0,
            &[1],
            "replace-real-preparation-retains-remote-audio",
        )
        .await;
        probe(
            &mut peers,
            1,
            if observer { &[] } else { &[0] },
            "replace-real-preparation-retains-source-role",
        )
        .await;
        if movement {
            let handle = replacement_handle.unwrap();
            let client = allocated_target.take().unwrap();
            peers.push(Peer {
                client,
                config: None,
                identity: f.client.identity_id.clone(),
                participant: owner.clone(),
                id: "provisional-replacement".into(),
                handle,
                clock: tokio::time::Instant::now(),
                sequence: 1,
                live: true,
            });
            if deferred_projection {
                rusqlite::Connection::open(&f.config.database).unwrap().execute_batch("CREATE TRIGGER fixture_projection_block BEFORE INSERT ON native_replacement_lifetimes BEGIN SELECT RAISE(ABORT,'isolated projection failure'); END;").unwrap();
            }
            send_readiness(&mut peers[2]).await;
            if deferred_projection {
                let until = tokio::time::Instant::now() + Duration::from_secs(5);
                loop {
                    let r = status(&f, &key.id, &command.operation_id).await;
                    if r.effects.iter().any(|e| {
                        e.provider == "native-replacement-commit"
                            && e.state == EffectState::Confirmed
                    }) {
                        assert_eq!(r.state, OperationState::Preparing);
                        assert!(r.result.is_none());
                        break;
                    }
                    assert!(
                        tokio::time::Instant::now() < until,
                        "Known movement was not captured: {r:?}"
                    );
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                let admin = f.server.store.authenticate(&f.admin).unwrap();
                let credential = f
                    .server
                    .store
                    .authenticate(&target.credential)
                    .unwrap()
                    .credential_id;
                f.server.store.revoke(&admin, &credential).unwrap();
                rusqlite::Connection::open(&f.config.database)
                    .unwrap()
                    .execute_batch("DROP TRIGGER fixture_projection_block")
                    .unwrap();
                let until = tokio::time::Instant::now() + Duration::from_secs(10);
                loop {
                    let r = status(&f, &key.id, &command.operation_id).await;
                    if r.state == OperationState::Completed {
                        assert_eq!(r.effects.len(), 7);
                        assert!(r
                            .effects
                            .iter()
                            .all(|e| e.state == EffectState::Confirmed && e.attempt == 1));
                        break;
                    }
                    assert!(
                        tokio::time::Instant::now() < until,
                        "Captured movement projection did not recover: {r:?}"
                    );
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                if let Some(probe) = sdk_probe {
                    let value = status(&f, &key.id, &command.operation_id)
                        .await
                        .result
                        .unwrap();
                    super::sdk_movements::replacement_completed(&f, probe, &command, &value).await;
                }
                wait_slots(&f, 1).await;
                let until = tokio::time::Instant::now() + Duration::from_secs(5);
                loop {
                    let c = rusqlite::Connection::open(&f.config.database).unwrap();
                    let retired:bool=c.query_row("SELECT c.state='retired' AND l.retired=1 FROM connections c JOIN native_connection_lifetimes l ON l.connection_id=c.id WHERE c.id=?1",[candidate],|r|r.get(0)).unwrap();
                    drop(c);
                    if retired {
                        break;
                    }
                    assert!(
                        tokio::time::Instant::now() < until,
                        "Revoked target remained after captured commit"
                    );
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                peers[1].live = false;
                peers[2].live = false;
                probe(
                    &mut peers,
                    0,
                    &[],
                    "replace-known-commit-key-loss-during-projection-preserves-retained",
                )
                .await;
                controller.transition(command.clone()).await.unwrap();
                assert_eq!(
                    status(&f, &key.id, &command.operation_id)
                        .await
                        .effects
                        .len(),
                    7
                );
                for peer in peers {
                    let _ = peer.client.shutdown().await;
                }
                wait_slots(&f, 0).await;
                f.server.shutdown().await.unwrap();
                eprintln!("actual captured movement projection failure recovered: known commit, revoked target before adoption, exact retirement, retained peer, no second attempt");
                continue;
            }

            let until = tokio::time::Instant::now() + Duration::from_secs(10);
            let done = loop {
                let r = status(&f, &key.id, &command.operation_id).await;
                if r.state == OperationState::Completed {
                    break r;
                }
                assert!(
                    !matches!(
                        r.state,
                        OperationState::Failed
                            | OperationState::Unknown
                            | OperationState::Cancelled
                    ),
                    "Movement failed: {r:?}"
                );
                assert!(
                    tokio::time::Instant::now() < until,
                    "Movement did not commit: {r:?}"
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            };
            if let Some(probe) = sdk_probe {
                super::sdk_movements::replacement_completed(
                    &f,
                    probe,
                    &command,
                    done.result.as_ref().unwrap(),
                )
                .await;
            }
            assert_eq!(done.effects.len(), 7);
            assert!(done
                .effects
                .iter()
                .all(|e| e.state == EffectState::Confirmed && e.attempt == 1));
            assert_eq!(done.result.as_ref().unwrap()["connection_id"], candidate);
            assert_eq!(done.result.as_ref().unwrap()["generation"], 2);
            peers[1].live = false;
            wait_slots(&f, 2).await;
            probe(
                &mut peers,
                0,
                &[2],
                "replace-moved-remote-reaches-only-target",
            )
            .await;
            probe(
                &mut peers,
                2,
                if observer { &[] } else { &[0] },
                "replace-moved-target-preserves-original-role",
            )
            .await;
            let remote_revision = stream_revision(&f, "codec-source");
            stream_control(
                &mut peers[0].client,
                &cid,
                "codec-source",
                "control-retained-after-move",
                remote_revision,
                "stream.subscribe",
                json!({"subscriptions":[{"strm_id":"replacement-voice"}]}),
            )
            .await;
            probe(
                &mut peers,
                2,
                if observer { &[] } else { &[0] },
                "replace-retained-control-uses-current-durable-revision",
            )
            .await;
            let admin = f.server.store.authenticate(&f.admin).unwrap();
            let old = f
                .server
                .store
                .authenticate(&f.client.credential)
                .unwrap()
                .credential_id;
            f.server.store.revoke(&admin, &old).unwrap();
            f.server.store.revoke(&admin, &key.id).unwrap();
            key = f
                .server
                .store
                .issue(
                    &f.client.identity_id,
                    scope.clone(),
                    chrono::Utc::now().timestamp() + 600,
                )
                .unwrap();
            let mut cfg = canonical_http_config(&f);
            cfg.credential = key.token.clone();
            controller = CanonicalHttpClient::open(cfg).await.unwrap();
            controller
                .request_value(
                    "operation.cancel",
                    json!({"operation_id":command.operation_id}),
                    CommandScope {
                        conversation_id: Some(cid.clone()),
                        session_id: Some("native-session".into()),
                        connection_id: command.connection_id.clone(),
                        operation_id: Some("late-cancel-moved-endpoint".into()),
                        ..Default::default()
                    },
                )
                .await
                .unwrap()
                .ack()
                .unwrap();
            probe(
                &mut peers,
                0,
                &[2],
                "replace-moved-survives-old-source-and-controller-revoke",
            )
            .await;
            probe(
                &mut peers,
                2,
                if observer { &[] } else { &[0] },
                "replace-moved-target-still-current",
            )
            .await;
            assert_eq!(
                status(&f, &key.id, &command.operation_id).await.state,
                OperationState::Completed
            );
            let c = rusqlite::Connection::open(&f.config.database).unwrap();
            let (state, generation): (String, u64) = c
                .query_row(
                    "SELECT state,generation FROM connections WHERE id=?1",
                    [candidate],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!((state, generation), ("ready".into(), 2));
            let role:String=c.query_row("SELECT role FROM session_participants WHERE session_id='native-session' AND participant_id=?1",[&owner],|r|r.get(0)).unwrap();
            assert_eq!(
                role,
                if observer {
                    "observer"
                } else {
                    plan["source_role"].as_str().unwrap()
                }
            );
            drop(c);
            let active = if !observer && !revoke_endpoint && !revoke_target_key {
                subsequent_move(
                    &f,
                    &mut controller,
                    &key,
                    &cid,
                    &owner,
                    candidate,
                    &target.credential,
                    &mut peers,
                )
                .await
            } else {
                candidate.to_owned()
            };
            if revoke_target_key {
                let credential = f
                    .server
                    .store
                    .authenticate(&target.credential)
                    .unwrap()
                    .credential_id;
                f.server.store.revoke(&admin, &credential).unwrap();
            } else if revoke_endpoint {
                let revision: u64 = rusqlite::Connection::open(&f.config.database)
                    .unwrap()
                    .query_row(
                        "SELECT revision FROM endpoints WHERE id=?1",
                        [&endpoint],
                        |r| r.get(0),
                    )
                    .unwrap();
                f.server
                    .store
                    .execute_provisioning(
                        &admin.credential_id,
                        connect_protocol::provisioning::ProvisioningCommand::RevokeEndpoint {
                            operation_id: "revoke-established-replacement-endpoint".into(),
                            endpoint_id: endpoint.clone(),
                            expected_revision: revision,
                        },
                    )
                    .unwrap();
            } else {
                let target_peer = peers.pop().unwrap();
                let _ = target_peer.client.shutdown().await;
            }
            wait_slots(&f, 1).await;
            let until = tokio::time::Instant::now() + Duration::from_secs(5);
            loop {
                let c = rusqlite::Connection::open(&f.config.database).unwrap();
                let retired:bool=c.query_row("SELECT c.state='retired' AND l.retired=1 FROM connections c JOIN native_connection_lifetimes l ON l.connection_id=c.id WHERE c.id=?1",[&active],|r|r.get(0)).unwrap();
                drop(c);
                if retired {
                    break;
                }
                assert!(
                    tokio::time::Instant::now() < until,
                    "Established retirement was not recorded"
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            for p in peers.iter_mut().skip(1) {
                p.live = false;
            }
            probe(
                &mut peers,
                0,
                &[],
                "replace-target-end-preserves-only-retained-peer",
            )
            .await;
            for peer in peers {
                let _ = peer.client.shutdown().await;
            }
            wait_slots(&f, 0).await;
            f.server.shutdown().await.unwrap();
            eprintln!("actual managed movement passed: observer={observer}; generation2, original-only retirement, current independent audio, latecancel and original authority revocation");
            continue;
        }
        if let Some(client) = allocated_target.as_mut() {
            // Only separately authorized host silence may reach the isolated
            // target. Marked original call audio remains forbidden.
            let until = tokio::time::Instant::now() + Duration::from_millis(300);
            while tokio::time::Instant::now() < until {
                let Ok(packet) = tokio::time::timeout(
                    Duration::from_millis(100),
                    client.receive_native_media_datagram(),
                )
                .await
                else {
                    break;
                };
                let packet = rvoip_uctp::substrate::unpack_rtp_datagram(&packet.unwrap()).unwrap();
                assert!(
                    packet.rtp.payload.iter().all(|b| matches!(*b, 0xff | 0x7f)),
                    "Provisional candidate received call audio"
                );
            }
        }
        if reject_answer {
            stopped(&f, &key.id, &command.operation_id, OperationState::Failed).await;
        } else if revoke_target_key {
            let target_key = f
                .server
                .store
                .authenticate(&target.credential)
                .unwrap()
                .credential_id;
            let admin = f.server.store.authenticate(&f.admin).unwrap();
            let store = f.server.store.clone();
            let began = tokio::time::Instant::now();
            tokio::task::spawn_blocking(move || store.revoke(&admin, &target_key))
                .await
                .unwrap()
                .unwrap();
            assert!(
                began.elapsed() < Duration::from_secs(2),
                "Allocated candidate retained an idle target authority reader"
            );
            stopped(&f, &key.id, &command.operation_id, OperationState::Failed).await;
        } else if revoke_endpoint {
            let endpoint_revision: u64 = rusqlite::Connection::open(&f.config.database)
                .unwrap()
                .query_row(
                    "SELECT revision FROM endpoints WHERE id=?1",
                    [&endpoint],
                    |r| r.get(0),
                )
                .unwrap();
            let store = f.server.store.clone();
            let admin = f.server.store.authenticate(&f.admin).unwrap().credential_id;
            let endpoint = endpoint.clone();
            tokio::task::spawn_blocking(move || {
                store.execute_provisioning(
                    &admin,
                    connect_protocol::provisioning::ProvisioningCommand::RevokeEndpoint {
                        operation_id: "revoke-managed-replacement-target".into(),
                        endpoint_id: endpoint,
                        expected_revision: endpoint_revision,
                    },
                )
            })
            .await
            .unwrap()
            .unwrap();
            stopped(&f, &key.id, &command.operation_id, OperationState::Failed).await;
        } else if observer {
            let admin = f.server.store.authenticate(&f.admin).unwrap();
            f.server.store.revoke(&admin, &key.id).unwrap();
            key = f
                .server
                .store
                .issue(
                    &f.client.identity_id,
                    scope,
                    chrono::Utc::now().timestamp() + 600,
                )
                .unwrap();
            let mut cfg = canonical_http_config(&f);
            cfg.credential = key.token.clone();
            controller = CanonicalHttpClient::open(cfg).await.unwrap();
            stopped(&f, &key.id, &command.operation_id, OperationState::Failed).await;
        } else {
            controller.request_value("operation.cancel",json!({"operation_id":command.operation_id,"reason":"cancel actual Core preparation before allocation"}),CommandScope {conversation_id:Some(cid.clone()),session_id:Some("native-session".into()),connection_id:command.connection_id.clone(),operation_id:Some("cancel-actual-managed-preparation".into()),..Default::default()}).await.unwrap().ack().unwrap();
            stopped(
                &f,
                &key.id,
                &command.operation_id,
                OperationState::Cancelled,
            )
            .await;
        }
        let terminal = status(&f, &key.id, &command.operation_id).await;
        if let Some(probe) = sdk_probe {
            super::sdk_movements::replacement_stopped(
                &f,
                probe,
                terminal.state,
                &key.token,
                observer && !reject_answer && !revoke_endpoint && !revoke_target_key,
            )
            .await;
        }
        assert_eq!(
            terminal.effects.len(),
            if start_media && !reject_answer {
                10
            } else if start_media {
                6
            } else if allocate {
                4
            } else {
                2
            }
        );
        assert!(terminal.effects.iter().all(|e| e.attempt == 1));
        assert_eq!(
            terminal
                .effects
                .iter()
                .filter(|e| e.state == EffectState::Unknown)
                .count(),
            usize::from(reject_answer)
        );
        assert_eq!(
            terminal
                .effects
                .iter()
                .filter(|e| e.state == EffectState::Confirmed)
                .count(),
            terminal.effects.len() - usize::from(reject_answer)
        );
        controller.transition(command.clone()).await.unwrap();
        assert_eq!(
            status(&f, &key.id, &command.operation_id).await.state,
            terminal.state
        );
        probe(&mut peers, 0, &[1], "replace-after-stop-remote-audio").await;
        probe(
            &mut peers,
            1,
            if observer { &[] } else { &[0] },
            "replace-after-stop-source-role",
        )
        .await;
        if !revoke_endpoint {
            // A distinct later root can reserve the engine's one preparation slot.
            let mut next = command.clone();
            next.operation_id = "second-actual-managed-preparation".into();
            controller.transition(next.clone()).await.unwrap();
            wait_prepared(&f, &key.id, &next.operation_id).await;
            controller
                .request_value(
                    "operation.cancel",
                    json!({"operation_id":next.operation_id}),
                    CommandScope {
                        conversation_id: Some(cid.clone()),
                        session_id: Some("native-session".into()),
                        connection_id: next.connection_id.clone(),
                        operation_id: Some("cancel-second-managed-preparation".into()),
                        ..Default::default()
                    },
                )
                .await
                .unwrap()
                .ack()
                .unwrap();
            stopped(&f, &key.id, &next.operation_id, OperationState::Cancelled).await;
        }
        let c = rusqlite::Connection::open(&f.config.database).unwrap();
        let count: u64 = c
            .query_row("SELECT COUNT(*) FROM connections", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2);
        let role:String=c.query_row("SELECT role FROM session_participants WHERE session_id='native-session' AND participant_id=?1",[&owner],|r|r.get(0)).unwrap();
        assert_eq!(
            role,
            if observer {
                "observer"
            } else {
                plan["source_role"].as_str().unwrap()
            }
        );
        drop(c);
        if let Some(client) = allocated_target {
            let _ = client.shutdown().await;
        }
        for peer in peers {
            let _ = peer.client.shutdown().await;
        }
        wait_slots(&f, 0).await;
        f.server.shutdown().await.unwrap();
        eprintln!("actual Core replacement phase passed: observer={observer}, endpoint_revoked={revoke_endpoint}, target_key_revoked={revoke_target_key}, allocated={allocate}, resources_started={start_media}, rejected_answer={reject_answer}; one attempt per phase, provisional target media closed, original audio/role retained, actual cancellation/retirement and capacity reuse");
    }
}

async fn send_readiness(peer: &mut Peer) {
    use rvoip_uctp::substrate::{pack_rtp_datagram, RtpDatagram, RtpMediaPayload};
    for _ in 0..12 {
        peer.sequence += 1;
        let packet = pack_rtp_datagram(&RtpDatagram {
            flags: 0,
            stream_local_id: peer.handle,
            seq: peer.sequence,
            rtp: RtpMediaPayload {
                payload: vec![0xff; 160].into(),
                payload_type: 0,
                sequence_number: peer.sequence as u16,
                timestamp: (peer.clock.elapsed().as_micros() * 8 / 1000) as u32,
                ssrc: 0x4736_8002,
            },
        })
        .unwrap();
        peer.client.send_native_media_datagram(&packet).unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
async fn subsequent_move(
    f: &Fixture,
    controller: &mut CanonicalHttpClient,
    key: &connect_store::IssuedCredential,
    cid: &str,
    owner: &str,
    source: &str,
    old_credential: &str,
    peers: &mut Vec<Peer>,
) -> String {
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope.operations.push("stream.subscribe".into());
    let target = party_with_scope(f, &f.client.identity_id, 282, vec!["audio".into()], scope);
    let endpoint: String = rusqlite::Connection::open(&f.config.database)
        .unwrap()
        .query_row(
            "SELECT id FROM endpoints WHERE device_id=?1",
            [&target.device_id],
            |r| r.get(0),
        )
        .unwrap();
    let command = TransitionCommand {
        operation_id: "actual-managed-replacement-generation-three".into(),
        conversation_id: cid.into(),
        session_id: "native-session".into(),
        connection_id: Some(source.into()),
        expected_revision: revision(f),
        intent: TransitionIntent::Replace {
            endpoint_id: Some(endpoint),
            candidate_connid: None,
            retained_connid: "codec-source".into(),
        },
        payload_extensions: Default::default(),
    };
    let public = controller.transition(command.clone()).await.unwrap();
    let prepared = wait_prepared(f, &key.id, &command.operation_id).await;
    let raw: String = rusqlite::Connection::open(&f.config.database)
        .unwrap()
        .query_row(
            "SELECT plan_json FROM native_replacements WHERE operation_record_id=?1",
            [&prepared.record_id],
            |r| r.get(0),
        )
        .unwrap();
    let plan: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(plan["generation"], 3);
    let candidate = plan["candidate_connection_id"].as_str().unwrap().to_owned();
    let mut client = CanonicalClient::open(target).await.unwrap();
    let offer = json!({"substrate":"quic","capabilities":capabilities(),"streams_offered":[{"id":"replacement-third-voice","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}],"replacement_preparation":{"profile":"connect-replacement-preparation-v1","operation_record_id":prepared.record_id}});
    client
        .allocate_native_replacement_target(
            &command,
            &public,
            serde_json::from_value(offer.clone()).unwrap(),
        )
        .await
        .unwrap();
    let (_, scope) = client
        .begin_native_replacement_media(&command, &public, serde_json::from_value(offer).unwrap())
        .await
        .unwrap();
    let answer = client.receive_control().await.unwrap();
    assert_eq!(answer.kind(), "connection.answer");
    let handle = handle(&answer);
    let callback = client.receive_control().await.unwrap();
    assert_eq!(callback.kind(), "connection.offer");
    let payload = &callback.original()["payload"];
    let streams=payload["substrate_setup"]["streams"].as_array().unwrap().iter().map(|v|json!({"id":v["strm_id"],"kind":v["kind"],"direction":match v["direction"].as_str().unwrap(){"sendonly"=>"recvonly","recvonly"=>"sendonly",_=>"sendrecv"},"codec":v["codec"]})).collect::<Vec<_>>();
    client.answer_connection_offer(&callback,serde_json::from_value(json!({"by_participant":payload["by_participant"],"substrate":"quic","capabilities":capabilities(),"streams_answered":streams,"substrate_setup":payload["substrate_setup"]})).unwrap(),&scope).await.unwrap();
    let until = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let r = status(f, &key.id, &command.operation_id).await;
        if r.effects.len() == 5 && r.effects.iter().all(|e| e.state == EffectState::Confirmed) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < until,
            "Second replacement did not activate: {r:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    peers.push(Peer {
        client,
        config: None,
        identity: f.client.identity_id.clone(),
        participant: owner.into(),
        id: candidate.clone(),
        handle,
        clock: tokio::time::Instant::now(),
        sequence: 1,
        live: true,
    });
    send_readiness(&mut peers[3]).await;
    let until = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let r = status(f, &key.id, &command.operation_id).await;
        if r.state == OperationState::Completed {
            assert_eq!(r.result.as_ref().unwrap()["generation"], 3);
            assert_eq!(r.effects.len(), 7);
            break;
        }
        assert!(
            !matches!(r.state, OperationState::Unknown | OperationState::Failed),
            "Second movement failed: {r:?}"
        );
        assert!(
            tokio::time::Instant::now() < until,
            "Second movement did not complete: {r:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    peers[2].live = false;
    wait_slots(f, 2).await;
    probe(
        peers,
        0,
        &[3],
        "replace-generation-three-retains-main-remote-audio",
    )
    .await;
    probe(peers, 3, &[0], "replace-generation-three-target-publishes").await;
    let remote_revision = stream_revision(f, "codec-source");
    stream_control(
        &mut peers[0].client,
        cid,
        "codec-source",
        "control-retained-after-generation-three",
        remote_revision,
        "stream.subscribe",
        json!({"subscriptions":[{"strm_id":"replacement-third-voice"}]}),
    )
    .await;
    probe(
        peers,
        3,
        &[0],
        "replace-generation-three-retained-control-stays-current",
    )
    .await;
    let admin = f.server.store.authenticate(&f.admin).unwrap();
    let old = f
        .server
        .store
        .authenticate(old_credential)
        .unwrap()
        .credential_id;
    f.server.store.revoke(&admin, &old).unwrap();
    probe(
        peers,
        0,
        &[3],
        "replace-generation-three-survives-generation-two-key-loss",
    )
    .await;
    probe(
        peers,
        3,
        &[0],
        "replace-generation-three-keeps-current-target-key",
    )
    .await;
    eprintln!("actual generation-three movement passed: retained native remote, new logical stream, generation-two key loss, no replacement rollback");
    candidate
}

#[tokio::test]
async fn production_replacement_known_commit_metadata_failure_and_target_loss_retire_without_replay(
) {
    exercise_replacement_mode(true, true, false, true, true).await;
}

#[path = "replacement/telephone.rs"]
mod telephone;
