//! Actual generic prepared takeover, without a private-pair prerequisite.
use super::*;
use connect_protocol::transitions::{OutgoingDisposition, RoleScope};
#[tokio::test]
async fn production_prepared_handoff_ai_human_directions_promote_observer_only_at_commit() {
    for (ai_source, observer, disposition, role_scope) in [
        (
            false,
            false,
            OutgoingDisposition::Observe,
            RoleScope::Session,
        ),
        (true, true, OutgoingDisposition::Observe, RoleScope::Session),
        (
            false,
            true,
            OutgoingDisposition::Leave,
            RoleScope::Conversation,
        ),
        (
            true,
            false,
            OutgoingDisposition::Leave,
            RoleScope::Conversation,
        ),
    ] {
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
            .find(|c| c.id == cid)
            .unwrap();
        let owner_pid = room
            .participants
            .iter()
            .find(|p| p.identity_id == f.client.identity_id)
            .unwrap()
            .participant_id
            .clone();
        let remote_pid = room
            .participants
            .iter()
            .find(|p| p.identity_id == f.peer.identity_id)
            .unwrap()
            .participant_id
            .clone();
        let ai = if ai_source {
            Some(
                enroll_kind(
                    &f,
                    &mut clients[1],
                    &cid,
                    "outgoing-ai",
                    241,
                    true,
                    true,
                    IdentityKind::Ai,
                )
                .await,
            )
        } else {
            None
        };
        let target_kind = if ai_source {
            IdentityKind::Human
        } else {
            IdentityKind::Ai
        };
        let target = enroll_kind(
            &f,
            &mut clients[1],
            &cid,
            "incoming-prepared",
            242,
            false,
            true,
            target_kind.clone(),
        )
        .await;
        if observer {
            clients[1]
                .session_command(Request {
                    operation_id: "incoming-observer".into(),
                    conversation_id: cid.clone(),
                    session_id: "native-session".into(),
                    by_participant: None,
                    expected_revision: Some(revision(&f)),
                    action: Action::Role {
                        participant_id: target.participant.clone(),
                        role: "observer".into(),
                    },
                })
                .await
                .unwrap();
        }
        let target_pid = target.participant.clone();
        let target_id = target.id.clone();
        let (primary_identity, primary_pid, primary_id, primary_handle) = if let Some(ai) = ai {
            // Establish the actual AI/human call and retire the original human
            // endpoint before handoff. Every later operation is public native/
            // HTTP control; the AI's own credential supplies its named grant.
            stream_control(
                &mut clients[0],
                &cid,
                "codec-source",
                "select-ai",
                stream_revision(&f, "codec-source"),
                "stream.subscribe",
                json!({"subscriptions":[{"from_participant":ai.participant,"kinds":["audio"]}]}),
            )
            .await;
            let mut ai_client = ai.client;
            stream_control(
                &mut ai_client,
                &cid,
                &ai.id,
                "select-remote",
                stream_revision(&f, &ai.id),
                "stream.subscribe",
                json!({"subscriptions":[{"from_participant":remote_pid,"kinds":["audio"]}]}),
            )
            .await;
            clients[1]
                .session_command(Request {
                    operation_id: "original-human-leave".into(),
                    conversation_id: cid.clone(),
                    session_id: "native-session".into(),
                    by_participant: None,
                    expected_revision: Some(revision(&f)),
                    action: Action::Leave {
                        participant_id: owner_pid.clone(),
                    },
                })
                .await
                .unwrap();
            let old = clients.pop().unwrap();
            let _ = old.shutdown().await;
            clients.push(ai_client);
            wait_slots(&f, 2).await;
            (ai.identity, ai.participant, ai.id, ai.handle)
        } else {
            (
                f.client.identity_id.clone(),
                owner_pid,
                "codec-target".into(),
                th,
            )
        };
        let config = target.config.unwrap();
        let mut prepared=target.client.prepare_handoff_native_audio(CommandScope {conversation_id:Some(cid.clone()),session_id:Some("native-session".into()),connection_id:Some(target_id.clone()),operation_id:Some("prepare-public-incoming".into()),expected_revision:Some(0)},json!({"substrate":"quic","capabilities":capabilities(),"streams_offered":[{"id":"incoming-voice","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}],"handoff_preparation":{"profile":"connect-handoff-preparation-v1"}})).await.unwrap();
        let mut wire = WireClock::new();
        let late_ready = ai_source && observer && disposition == OutgoingDisposition::Observe;
        for source in 0..if late_ready { 2 } else { 3 } {
            let allowed = if source < 2 { vec![1 - source] } else { vec![] };
            route_probe(
                &mut clients,
                &mut prepared,
                [sh, primary_handle],
                [true, true],
                source,
                &allowed,
                &mut wire,
            )
            .await;
        }
        let desired_role:String=rusqlite::Connection::open(&f.config.database).unwrap().query_row("SELECT role FROM session_participants WHERE session_id='native-session' AND participant_id=?1",[&primary_pid],|r|r.get(0)).unwrap();
        if observer {
            let before:String=rusqlite::Connection::open(&f.config.database).unwrap().query_row("SELECT role FROM session_participants WHERE session_id='native-session' AND participant_id=?1",[&target_pid],|r|r.get(0)).unwrap();
            assert_eq!(before, "observer");
        }
        let mut scope = Scope::collaboration(vec!["*".into()]);
        scope
            .operations
            .extend(["session.handoff".into(), "operation.cancel".into()]);
        let mut key = f
            .server
            .store
            .issue(
                &primary_identity,
                scope.clone(),
                chrono::Utc::now().timestamp() + 300,
            )
            .unwrap();
        let mut cfg = canonical_http_config(&f);
        cfg.credential = key.token.clone();
        let mut controller = CanonicalHttpClient::open(cfg).await.unwrap();
        let before = clients[1]
            .list_conversations(Default::default())
            .await
            .unwrap()
            .conversations
            .into_iter()
            .find(|r| r.id == cid)
            .unwrap();
        let mut command = TransitionCommand {
            operation_id: "prepared-public-takeover".into(),
            conversation_id: cid.clone(),
            session_id: "native-session".into(),
            connection_id: Some(primary_id.clone()),
            expected_revision: revision(&f),
            intent: TransitionIntent::Handoff {
                source_participant: primary_pid.clone(),
                target_participant: target_pid.clone(),
                retained_connid: "codec-source".into(),
                candidate_connid: Some(target_id.clone()),
                outgoing: disposition,
                role_scope,
                expected_conversation_revision: (role_scope == RoleScope::Conversation)
                    .then_some(before.revision),
            },
            payload_extensions: std::collections::BTreeMap::from([(
                "policy_ref".into(),
                json!(prepared.policy_record_id()),
            )]),
        };
        controller.transition(command.clone()).await.unwrap();
        let (record, completed_command) = if late_ready {
            tokio::time::sleep(Duration::from_millis(250)).await;
            let pending = status(&f, &key.id, &command.operation_id).await;
            assert_eq!(pending.state, OperationState::Accepted);
            assert!(pending.effects.is_empty());
            assert!(pending.result.is_none());
            let before:String=rusqlite::Connection::open(&f.config.database).unwrap().query_row("SELECT role FROM session_participants WHERE session_id='native-session' AND participant_id=?1",[&target_pid],|r|r.get(0)).unwrap();
            assert_eq!(before, "observer");
            let mut competing = command.clone();
            competing.operation_id = "competing-prepared-takeover".into();
            let refused = controller.transition(competing).await.unwrap_err();
            let connect_client::ClientError::Application(problem) = refused else {
                panic!("unexpected competing refusal: {refused:?}");
            };
            assert_eq!(problem.code, "conflict");
            let competing_rows:u64=rusqlite::Connection::open(&f.config.database).unwrap().query_row("SELECT COUNT(*) FROM operations WHERE operation_id='competing-prepared-takeover'",[],|r|r.get(0)).unwrap();
            assert_eq!(competing_rows, 0);
            let cancel = controller
                .request_value(
                    "operation.cancel",
                    json!({"operation_id":command.operation_id,"reason":"cancel before target IO"}),
                    CommandScope {
                        conversation_id: Some(cid.clone()),
                        session_id: Some("native-session".into()),
                        connection_id: Some(primary_id.clone()),
                        operation_id: Some("cancel-unready-prepared-handoff".into()),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            cancel.ack().unwrap();
            let cancelled = status(&f, &key.id, &command.operation_id).await;
            assert_eq!(cancelled.state, OperationState::Cancelled);
            assert!(cancelled.effects.is_empty());
            command.operation_id = "prepared-after-cancel".into();
            controller.transition(command.clone()).await.unwrap();
            let before_revoke = status(&f, &key.id, &command.operation_id).await;
            assert!(before_revoke.effects.is_empty());
            let admin = f.server.store.authenticate(&f.admin).unwrap();
            f.server.store.revoke(&admin, &key.id).unwrap();
            // The media peer keeps its independently bound credential. Revoking
            // this controller cannot lend its old grant to the waiting job.
            tokio::time::sleep(Duration::from_millis(250)).await;
            key = f
                .server
                .store
                .issue(
                    &primary_identity,
                    scope.clone(),
                    chrono::Utc::now().timestamp() + 600,
                )
                .unwrap();
            let mut fresh = canonical_http_config(&f);
            fresh.credential = key.token.clone();
            controller = CanonicalHttpClient::open(fresh).await.unwrap();
            let stopped = status(&f, &key.id, &command.operation_id).await;
            assert!(stopped.effects.is_empty());
            assert!(stopped.result.is_none());
            route_probe(
                &mut clients,
                &mut prepared,
                [sh, primary_handle],
                [true, true],
                0,
                &[1],
                &mut wire,
            )
            .await;
            route_probe(
                &mut clients,
                &mut prepared,
                [sh, primary_handle],
                [true, true],
                1,
                &[0],
                &mut wire,
            )
            .await;
            controller.transition(command.clone()).await.unwrap();
            assert_eq!(
                status(&f, &key.id, &command.operation_id).await.expires_at,
                before_revoke.expires_at
            );
            // The only replay replaces a revoked control grant. Real neutral
            // input drives readiness automatically; no application commit call.

            for _ in 0..5 {
                prepared.send_pcmu(&[0xff; 160]).unwrap();
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            let completed = wait_state(&f, &key.id, &command.operation_id, true).await;
            eprintln!("late readiness: competing refusal, public cancel and controller revocation preserve original audio; fresh same-intent authority completes one attempt without readiness replay");
            (completed, command.clone())
        } else {
            (
                wait_state(&f, &key.id, &command.operation_id, true).await,
                command.clone(),
            )
        };
        assert_eq!(record.effects.len(), 1);
        assert_eq!(record.effects[0].attempt, 1);
        assert_eq!(
            record.result.as_ref().unwrap()["retained_connection_ids"],
            json!(["codec-source"])
        );
        assert_eq!(
            record.result.as_ref().unwrap()["admitted_connection_ids"],
            json!([target_id])
        );
        assert_eq!(
            controller
                .transition(completed_command)
                .await
                .unwrap()
                .result,
            record.result
        );
        let c = rusqlite::Connection::open(&f.config.database).unwrap();
        let role:String=c.query_row("SELECT role FROM session_participants WHERE session_id='native-session' AND participant_id=?1",[&target_pid],|r|r.get(0)).unwrap();
        assert_eq!(role, desired_role);
        drop(c);
        let observe = disposition == OutgoingDisposition::Observe;
        if observe {
            let mut stale = command.clone();
            stale.operation_id = "stale-prepared-topology".into();
            let rejected = controller.transition(stale).await.unwrap_err();
            assert!(
                matches!(rejected,connect_client::ClientError::Application(ref e) if e.code=="conflict")
            );
        }
        if !observe {
            wait_slots(&f, 2).await;
        }
        let allowed = if observe { vec![1, 2] } else { vec![2] };
        route_probe(
            &mut clients,
            &mut prepared,
            [sh, primary_handle],
            [true, observe],
            0,
            &allowed,
            &mut wire,
        )
        .await;
        let allowed = if observe { vec![0, 1] } else { vec![0] };
        route_probe(
            &mut clients,
            &mut prepared,
            [sh, primary_handle],
            [true, observe],
            2,
            &allowed,
            &mut wire,
        )
        .await;
        if observe {
            route_probe(
                &mut clients,
                &mut prepared,
                [sh, primary_handle],
                [true, true],
                1,
                &[],
                &mut wire,
            )
            .await;
        }
        let _ = prepared.shutdown().await;
        for client in clients {
            let _ = client.shutdown().await;
        }
        wait_slots(&f, 0).await;
        f.server.shutdown().await.unwrap();
        eprintln!("actual prepared handoff passed: ai_source={ai_source} incoming={target_kind:?} initially_observer={observer} disposition={disposition:?} role_scope={role_scope:?} own_endpoint={}",config.device_id);
    }
}
