//! Two distinct transfers retain one physical remote and never inherit an audience.
use super::*;
use connect_protocol::transitions::{OutgoingDisposition, RoleScope};
#[path = "chain/observation_host.rs"]
mod observation_host;
#[path = "chain/worker.rs"]
mod worker;

async fn prepare(peer: Peer, cid: &str, operation: &str) -> PreparedHandoffNativeAudio {
    peer.client.prepare_handoff_native_audio(CommandScope {
        conversation_id:Some(cid.into()),session_id:Some("native-session".into()),
        connection_id:Some(peer.id),operation_id:Some(operation.into()),expected_revision:Some(0),
    },json!({"substrate":"quic","capabilities":capabilities(),"streams_offered":[{"id":"chain-voice","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}],"handoff_preparation":{"profile":"connect-handoff-preparation-v1"}})).await.unwrap()
}

#[tokio::test]
async fn production_repeated_prepared_handoff_retains_remote_and_quarantines_previous_observer() {
    run(false).await;
    run(true).await;
}
async fn run(end_wins: bool) {
    let codecs::Running {
        f,
        mut clients,
        cid,
        sh,
        th,
    } = codecs::setup().await;
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
    let ai = enroll_kind(
        &f,
        &mut clients[1],
        &cid,
        "chain-ai",
        251,
        false,
        true,
        IdentityKind::Ai,
    )
    .await;
    let human = enroll_kind(
        &f,
        &mut clients[1],
        &cid,
        "chain-human",
        252,
        false,
        true,
        IdentityKind::Human,
    )
    .await;
    let ai_identity = ai.identity.clone();
    let ai_pid = ai.participant.clone();
    let ai_id = ai.id.clone();
    let human_identity = human.identity.clone();
    let human_pid = human.participant.clone();
    let human_id = human.id.clone();
    let mut ai = prepare(ai, &cid, "prepare-chain-ai").await;
    let mut human = prepare(human, &cid, "prepare-chain-human").await;
    let remote_incarnation = || {
        rusqlite::Connection::open(&f.config.database).unwrap().query_row("SELECT v.generation,m.runtime_connection_id FROM connections v JOIN connection_offers m ON m.connection_id=v.id WHERE v.id='codec-source'",[],|r|Ok((r.get::<_,u64>(0)?,r.get::<_,String>(1)?))).unwrap()
    };
    let original_remote = remote_incarnation();
    let before = clients[1]
        .conversation_snapshot(&cid, serde_json::from_value(json!({})).unwrap())
        .await
        .unwrap();
    assert!(!serde_json::to_string(&before).unwrap().contains(&ai_id));
    assert!(!serde_json::to_string(&before).unwrap().contains(&human_id));
    let mut wire = WireClock::new();
    // Both new targets prove duplex IO independently while the original call
    // remains intact. Neither target may hear the call or the other target.
    for source in 0..4 {
        let allowed = if source < 2 { vec![1 - source] } else { vec![] };
        route_probe_many(
            &mut clients,
            &mut [&mut ai, &mut human],
            [sh, th],
            [true, true],
            &[true, true],
            source,
            &allowed,
            &mut wire,
        )
        .await;
    }
    let mut commands = Vec::new();
    for (index, identity, source_pid, source_id, target_pid, target_id, policy) in [
        (
            0,
            f.client.identity_id.clone(),
            owner,
            "codec-target".to_string(),
            ai_pid.clone(),
            ai_id.clone(),
            ai.policy_record_id().to_string(),
        ),
        (
            1,
            ai_identity,
            ai_pid.clone(),
            ai_id.clone(),
            human_pid.clone(),
            human_id.clone(),
            human.policy_record_id().to_string(),
        ),
    ] {
        let mut scope = Scope::collaboration(vec!["*".into()]);
        scope.operations.push("session.handoff".into());
        let key = f
            .server
            .store
            .issue(&identity, scope, chrono::Utc::now().timestamp() + 600)
            .unwrap();
        let mut cfg = canonical_http_config(&f);
        cfg.credential = key.token.clone();
        let mut controller = CanonicalHttpClient::open(cfg).await.unwrap();
        let mut command = TransitionCommand {
            operation_id: format!("chain-handoff-{index}"),
            conversation_id: cid.clone(),
            session_id: "native-session".into(),
            connection_id: Some(source_id),
            expected_revision: revision(&f),
            intent: TransitionIntent::Handoff {
                source_participant: source_pid,
                target_participant: target_pid,
                retained_connid: "codec-source".into(),
                candidate_connid: Some(target_id.clone()),
                outgoing: OutgoingDisposition::Observe,
                role_scope: RoleScope::Session,
                expected_conversation_revision: None,
            },
            payload_extensions: std::collections::BTreeMap::from([(
                "policy_ref".into(),
                json!(policy),
            )]),
        };
        if index == 1 {
            use connect_protocol::events::{
                AgentActionPhase, AgentObservationContent, ObservationDraft,
            };
            let host = observation_host::prepare(
                &f,
                &identity,
                &cid,
                command.connection_id.as_deref().unwrap(),
            )
            .await;
            host.publish(&f).await;
            let observations = host.page(&f, &cid).await;
            assert_eq!(observations.len(), 8);
            let hidden = host.page_as(&f, &cid, &human_identity).await;
            assert!(
                hidden.observations.is_empty() && hidden.routes.is_empty(),
                "still-isolated future target received current provider observations or metadata"
            );
            assert_eq!(observations.iter().filter(|v|matches!(&v.data,ObservationDraft::Agent {observation} if matches!(observation.content,AgentObservationContent::Action {phase:AgentActionPhase::Requested,..}))).count(),4);
            let providers = observations
                .iter()
                .filter_map(|v| match &v.data {
                    ObservationDraft::Agent { observation } => {
                        observation.provider_session_id.clone()
                    }
                    _ => None,
                })
                .collect::<std::collections::BTreeSet<_>>();
            assert_eq!(providers.len(), 2);
            let handoffs = || {
                rusqlite::Connection::open(&f.config.database)
                    .unwrap()
                    .query_row(
                        "SELECT COUNT(*) FROM native_handoffs WHERE started=1",
                        [],
                        |r| r.get::<_, u64>(0),
                    )
                    .unwrap()
            };
            assert_eq!(
                handoffs(),
                1,
                "suggestions/reported results cannot dispatch effects"
            );
            command.operation_id = format!("agent-answer-{}", host.work.question_id);
            let path = f.temp.path().join("approved-agent-handoff.json");
            std::fs::write(
                &path,
                serde_json::to_vec(&worker::Decision {
                    work: host.work.clone(),
                    command: command.clone(),
                    reply_address: "connect:chain-human@local".into(),
                })
                .unwrap(),
            )
            .unwrap();
            let mut inspect_scope = Scope::collaboration(vec!["*".into()]);
            inspect_scope
                .operations
                .extend(["task_get", "task_effect_get"].map(str::to_owned));
            let inspection = f
                .server
                .store
                .issue(
                    &identity,
                    inspect_scope,
                    chrono::Utc::now().timestamp() + 600,
                )
                .unwrap();
            let mut inspect_cfg = canonical_http_config(&f);
            inspect_cfg.credential = inspection.token;
            let response = f
                .server
                .store
                .issue(
                    &human_identity,
                    Scope::collaboration(vec!["*".into()]),
                    chrono::Utc::now().timestamp() + 600,
                )
                .unwrap();
            let mut response_cfg = canonical_http_config(&f);
            response_cfg.credential = response.token;
            let mut execute_cfg = canonical_http_config(&f);
            execute_cfg.credential = key.token.clone();
            let action=observations.iter().find(|v|matches!(&v.data,ObservationDraft::Agent {observation} if matches!(observation.content,AgentObservationContent::Action {phase:AgentActionPhase::Requested,..}))).unwrap();
            let denied = f
                .server
                .store
                .issue(
                    &identity,
                    Scope::collaboration(vec!["*".into()]),
                    chrono::Utc::now().timestamp() + 600,
                )
                .unwrap();
            let mut denied_cfg = canonical_http_config(&f);
            denied_cfg.credential = denied.token;
            let mut refused =
                worker::Worker::open(&path, inspect_cfg.clone(), denied_cfg, response_cfg.clone())
                    .await
                    .unwrap();
            assert!(
                refused.observe(action).await.is_err(),
                "work data cannot supply a Core handoff grant"
            );
            assert_eq!(handoffs(), 1);
            let mut external = worker::Worker::open(
                &path,
                inspect_cfg.clone(),
                execute_cfg.clone(),
                response_cfg.clone(),
            )
            .await
            .unwrap();
            for observation in &observations {
                if !matches!(&observation.data,ObservationDraft::Agent {observation} if matches!(observation.content,AgentObservationContent::Action {phase:AgentActionPhase::Requested,..}))
                {
                    assert!(!external.observe(observation).await.unwrap());
                }
            }
            assert!(
                external.reply_completed().await.is_err(),
                "provider reported result cannot become actual completion"
            );
            let mut forged = action.clone();
            forged.work.as_mut().unwrap().task_id = "model-forged-task".into();
            assert!(external.observe(&forged).await.is_err());
            assert_eq!(handoffs(), 1);
            if end_wins {
                // The same otherwise valid approved decision loses to an
                // independent public Session-end before it can be admitted.
                let ended = clients[1]
                    .session_command(Request {
                        operation_id: "worker-session-end-wins".into(),
                        conversation_id: cid.clone(),
                        session_id: "native-session".into(),
                        by_participant: None,
                        expected_revision: Some(revision(&f)),
                        action: Action::End {
                            reason_code: 1000,
                            reason: "independent owner ended before worker action".into(),
                        },
                    })
                    .await
                    .unwrap();
                assert!(
                    matches!(ended,connect_protocol::Outcome::SessionControl(ref outcome) if outcome.session.state=="ended")
                );
                assert!(external.observe(action).await.is_err());
                assert_eq!(
                    handoffs(),
                    1,
                    "late suggestion after Session-end added a handoff"
                );
                host.close(&f);
                let _ = ai.shutdown().await;
                let _ = human.shutdown().await;
                for client in clients {
                    let _ = client.shutdown().await;
                }
                wait_slots(&f, 0).await;
                f.server.shutdown().await.unwrap();
                eprintln!("agent worker Session-end race: frozen valid decision refused after independent end; retained first handoff only; no effect from cached late suggestion");
                return;
            }
            assert!(external.observe(action).await.unwrap());
            // Recreate the worker after submission with only its persisted
            // approval. Repeated provider and tool IDs, and a recreated provider
            // reference, still replay this exact logical command.
            drop(external);
            external = worker::Worker::open(&path, inspect_cfg, execute_cfg, response_cfg)
                .await
                .unwrap();
            for observation in &observations {
                let _ = external.observe(observation).await.unwrap();
            }
            let until = tokio::time::Instant::now() + Duration::from_secs(10);
            loop {
                let observed = external.completion().await.unwrap();
                if observed["state"] == "completed" {
                    break;
                }
                assert!(
                    tokio::time::Instant::now() < until,
                    "public operation never completed"
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            let reply = external.reply_completed().await.unwrap();
            assert!(
                matches!(&reply,connect_protocol::tasks::TaskOutcome::ReplyAccepted {reply,..} if reply.task_id.as_ref()==Some(&host.work.task_id) && reply.reply_to.as_ref()==Some(&host.work.question_id))
            );
            assert_eq!(external.reply_completed().await.unwrap(), reply);
            let replies: u64 = rusqlite::Connection::open(&f.config.database)
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM task_replies WHERE task_id=?1 AND reply_to=?2",
                    rusqlite::params![host.work.task_id, host.work.question_id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(replies, 1);
            let admin = f.server.store.authenticate(&f.admin).unwrap();
            f.server.store.revoke(&admin, &inspection.id).unwrap();
            assert!(
                external.observe(action).await.is_err(),
                "cached decision must still require current task inspection"
            );
            assert_eq!(
                external.completion().await.unwrap()["state"],
                "completed",
                "independent Core grant remains current"
            );
            assert_eq!(handoffs(), 2);
            host.close(&f);
            eprintln!("agent worker public operations: actual question and effect inspected; forged arguments ignored; partial/reported results do not dispatch; explicit current Core grant; two providers and duplicate tools replay one persisted decision across worker recreation; actual completion replied to exact question once; revoked inspection refused");
        } else {
            controller.transition(command.clone()).await.unwrap();
        }
        let done = wait_state(&f, &key.id, &command.operation_id, true).await;
        assert_eq!(done.effects.len(), 1);
        assert_eq!(done.effects[0].attempt, 1);
        let result = done.result.as_ref().unwrap();
        assert_eq!(result["retained_connection_ids"], json!(["codec-source"]));
        assert_eq!(result["admitted_connection_ids"], json!([target_id]));
        assert_eq!(
            controller.transition(command.clone()).await.unwrap().result,
            done.result
        );
        commands.push((controller, command, done.result));
        let destinations: &[&[usize]] = if index == 0 {
            &[&[1, 2], &[], &[0, 1], &[]]
        } else {
            &[&[2, 3], &[], &[], &[0, 2]]
        };
        for (source, allowed) in destinations.iter().enumerate() {
            route_probe_many(
                &mut clients,
                &mut [&mut ai, &mut human],
                [sh, th],
                [true, true],
                &[true, true],
                source,
                allowed,
                &mut wire,
            )
            .await;
        }
        eprintln!("chained transfer {index}: same native remote and Session, one effect, exact observer audience");
    }
    // Old completed intent replay is observation only, even after its audience
    // has been consumed by a newer transfer.
    for (controller, command, result) in &mut commands {
        assert_eq!(
            controller.transition(command.clone()).await.unwrap().result,
            *result
        );
    }
    let c = rusqlite::Connection::open(&f.config.database).unwrap();
    let (state, generation): (String, u64) = c
        .query_row(
            "SELECT state,generation FROM connections WHERE id='codec-source'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(matches!(state.as_str(), "ready" | "active"));
    assert_eq!(generation, original_remote.0);
    assert_eq!(remote_incarnation(), original_remote);
    let attempts: u64 = c
        .query_row(
            "SELECT COUNT(*) FROM native_handoffs WHERE started=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(attempts, 2);
    drop(c);
    // Snapshots follow the latest captured audience; the first observer must
    // not learn the next target's physical metadata through the old receipt.
    let old = clients[1]
        .conversation_snapshot(&cid, serde_json::from_value(json!({})).unwrap())
        .await
        .unwrap();
    let leaked = old
        .items
        .iter()
        .filter(|v| v.to_string().contains(&human_id))
        .collect::<Vec<_>>();
    assert!(
        leaked.is_empty(),
        "later target metadata leaked: {leaked:?}"
    );
    let _ = ai.shutdown().await;
    let _ = human.shutdown().await;
    for client in clients {
        let _ = client.shutdown().await;
    }
    wait_slots(&f, 0).await;
    f.server.shutdown().await.unwrap();
}
