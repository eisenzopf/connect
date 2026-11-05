//! Real private ancestry, never a fabricated floor or a public fallback.
//! These controls deliberately fail if the existing signed private replacement
//! cannot reach the actual move or cannot retain its ordinary private audience.
use super::*;
#[path = "private_continuation/publication_retirement.rs"]
mod publication_retirement;

#[derive(Clone, Copy)]
enum Case {
    OriginalPrivate,
    OrdinaryReplacement,
    PausedReplacement,
    RevokedPartnerAfterCommit,
    RevokedRemoteAfterCommit,
}

#[tokio::test]
async fn production_original_private_consultation_retains_ordinary_policy_control() {
    exercise(Case::OriginalPrivate).await;
}

#[tokio::test]
async fn production_completed_private_replacement_retains_partner_and_excludes_public_remote() {
    exercise(Case::OrdinaryReplacement).await;
}

#[tokio::test]
async fn production_paused_private_replacement_refusal_never_falls_back_to_public_media() {
    exercise(Case::PausedReplacement).await;
}

async fn exercise(case: Case) {
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
        .find(|room| room.id == cid)
        .unwrap();
    let participant = |identity: &str| {
        room.participants
            .iter()
            .find(|p| p.identity_id == identity)
            .unwrap()
            .participant_id
            .clone()
    };
    let primary = clients.pop().unwrap();
    let public_remote = clients.pop().unwrap();
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
            public_remote,
            f.peer.identity_id.clone(),
            participant(&f.peer.identity_id),
            "codec-source",
            sh,
        ),
        make(
            primary,
            f.client.identity_id.clone(),
            participant(&f.client.identity_id),
            "codec-target",
            th,
        ),
    ];
    let private = enroll(
        &f,
        &mut peers[1].client,
        &cid,
        "replacement-private-partner",
        485,
        false,
    )
    .await;
    peers.push(private);
    // Public signed consult + actual native private invitation establishes the
    // original physical pair. No test SQL changes create or alter its floor.
    let group = begin(
        &f,
        &mut peers,
        &cid,
        "private-before-device-replacement",
        1,
        0,
        2,
    )
    .await;
    assert_private_ancestry(&f, &group.record, &peers[1].id, &peers[2].id);
    probe(&mut peers, 1, &[2], "private-original-primary-to-partner").await;
    probe(&mut peers, 2, &[1], "private-original-partner-to-primary").await;
    probe(
        &mut peers,
        0,
        &[],
        "private-original-public-remote-excluded",
    )
    .await;

    let consultation = wait_state(&f, &group.key.id, &group.command.operation_id, true).await;
    assert_eq!(consultation.record_id, group.record);
    let db = rusqlite::Connection::open(&f.config.database).unwrap();
    let established: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM native_private_lifetimes WHERE operation_record_id=?1 AND runtime_epoch=?2 AND restored_at IS NULL)",
        rusqlite::params![group.record, f.server.store.runtime_epoch()], |r| r.get(0),
    ).unwrap();
    assert!(
        established,
        "the original consultation must have its completed ordinary lifetime before replacement"
    );
    drop(db);

    if matches!(case, Case::OriginalPrivate) {
        restore(&f, &mut peers, &cid, &group).await;
        probe(
            &mut peers,
            0,
            &[1],
            "private-original-return-retains-public-remote",
        )
        .await;
        probe(
            &mut peers,
            1,
            &[0],
            "private-original-return-retains-primary",
        )
        .await;
        for peer in peers {
            peer.client.shutdown().await.unwrap();
        }
        f.server.shutdown().await.unwrap();
        return;
    }

    let mut target_scope = Scope::collaboration(vec!["*".into()]);
    target_scope.operations.push("stream.subscribe".into());
    let target = party_with_scope(
        &f,
        &f.client.identity_id,
        486,
        vec!["audio".into()],
        target_scope,
    );
    let db = rusqlite::Connection::open(&f.config.database).unwrap();
    let endpoint: String = db
        .query_row(
            "SELECT id FROM endpoints WHERE device_id=?1",
            [&target.device_id],
            |r| r.get(0),
        )
        .unwrap();
    let old_core: String = db
        .query_row(
            "SELECT runtime_connection_id FROM connection_offers WHERE connection_id=?1",
            [&peers[1].id],
            |r| r.get(0),
        )
        .unwrap();
    drop(db);
    let old_owners = if matches!(
        case,
        Case::RevokedPartnerAfterCommit | Case::RevokedRemoteAfterCommit
    ) {
        publication_retirement::observe(
            &f,
            &old_core,
            "old",
            tokio::time::Instant::now() + Duration::from_secs(2),
        )
        .await
    } else {
        Vec::new()
    };
    let mut grant = Scope::collaboration(vec!["*".into()]);
    grant.operations.extend([
        "connection.replace".into(),
        "operation.cancel".into(),
        "session.consult.return".into(),
    ]);
    let key = f
        .server
        .store
        .issue(
            &f.client.identity_id,
            grant,
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let mut cfg = canonical_http_config(&f);
    cfg.credential = key.token.clone();
    let mut controller = CanonicalHttpClient::open(cfg).await.unwrap();
    let command = TransitionCommand {
        operation_id: "actual-private-primary-replacement".into(),
        conversation_id: cid.clone(),
        session_id: "native-session".into(),
        connection_id: Some(peers[1].id.clone()),
        expected_revision: revision(&f),
        intent: TransitionIntent::Replace {
            endpoint_id: Some(endpoint),
            candidate_connid: None,
            // Keep the actual private partner. The held public remote cannot
            // serve as evidence for a private replacement audience.
            retained_connid: peers[2].id.clone(),
            video: None,
        },
        payload_extensions: Default::default(),
    };
    let root = controller
        .transition(command.clone())
        .await
        .expect("actual private source must be accepted without rewriting its context");
    let prepared = wait_prepared(&f, &key.id, &command.operation_id).await;
    let cutoff = decoded_readiness::original_cutoff(&prepared);
    let db = rusqlite::Connection::open(&f.config.database).unwrap();
    let plan: String = db
        .query_row(
            "SELECT plan_json FROM native_replacements WHERE operation_record_id=?1",
            [&prepared.record_id],
            |r| r.get(0),
        )
        .unwrap();
    let plan: Value = serde_json::from_str(&plan).unwrap();
    let candidate = plan["candidate_connection_id"].as_str().unwrap().to_owned();
    let target_core = plan["candidate_runtime_id"].as_str().unwrap().to_owned();
    assert_eq!(plan["generation"], 2);
    drop(db);
    let mut client = CanonicalClient::open(target.clone()).await.unwrap();
    let offer = json!({"substrate":"quic", "capabilities":capabilities(),
        "streams_offered":[{"id":"private-replacement-voice", "kind":"audio", "direction":"sendrecv", "codec_preferences":["g.711-mu"]}],
        "replacement_preparation":{"profile":"connect-replacement-preparation-v1", "operation_record_id":prepared.record_id}});
    client
        .allocate_native_replacement_target(
            &command,
            &root,
            serde_json::from_value(offer.clone()).unwrap(),
        )
        .await
        .unwrap();
    let (request, scope) = client
        .begin_native_replacement_media(&command, &root, serde_json::from_value(offer).unwrap())
        .await
        .unwrap();
    let answer = tokio::time::timeout_at(cutoff, client.receive_control())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        answer.kind(),
        "connection.answer",
        "private replacement start: {:?}",
        answer.original()
    );
    assert_eq!(answer.original()["in_reply_to"], request);
    let target_handle = handle(&answer);
    let callback = tokio::time::timeout_at(cutoff, client.receive_control())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        callback.kind(),
        "connection.offer",
        "{:?}",
        callback.original()
    );
    let decoder = decoded_readiness::answer(&mut client, &callback, &scope, cutoff).await;
    let pause = if matches!(
        case,
        Case::PausedReplacement | Case::RevokedPartnerAfterCommit | Case::RevokedRemoteAfterCommit
    ) {
        Some(
            f.server
                .service
                .pause_canonical_replacement_projection_for_test(&prepared.record_id)
                .unwrap(),
        )
    } else {
        None
    };
    peers.push(Peer {
        client,
        config: Some(target),
        identity: f.client.identity_id.clone(),
        participant: participant(&f.client.identity_id),
        id: candidate.clone(),
        handle: target_handle,
        clock: tokio::time::Instant::now(),
        sequence: 1,
        live: true,
    });
    if matches!(
        case,
        Case::RevokedPartnerAfterCommit | Case::RevokedRemoteAfterCommit
    ) {
        publication_retirement::finish(
            f,
            peers,
            group,
            controller,
            command,
            key.id,
            prepared,
            candidate,
            old_core,
            target_core,
            cutoff,
            pause.unwrap(),
            old_owners,
            decoder,
            case,
        )
        .await;
        return;
    }
    let frozen = decoded_readiness::confirm(
        &mut peers[3],
        decoder,
        None,
        &mut controller,
        &command,
        &prepared,
    )
    .await;
    if let Some(pause) = pause {
        if tokio::time::timeout_at(
            cutoff.min(tokio::time::Instant::now() + Duration::from_secs(5)),
            pause.entered(),
        )
        .await
        .is_err()
        {
            panic!("actual private replacement never reached Core commit; refusal witness is NOT qualified: {:?}", status(&f, &key.id, &command.operation_id).await);
        }
        let current = status(&f, &key.id, &command.operation_id).await;
        assert_eq!(current.state, OperationState::Preparing);
        assert!(current.result.is_none());
        unpublished(&f, &candidate, &prepared.record_id);
        let session = f
            .server
            .service
            .canonical_session_metadata_for_test("native-session")
            .unwrap();
        {
            let current = session.read().unwrap();
            assert!(
                !current
                    .connections
                    .contains_key(&rvoip_core::ids::ConnectionId::from_string(
                        old_core.clone()
                    )),
                "private old Core route must really be retired"
            );
            assert!(
                current
                    .connections
                    .contains_key(&rvoip_core::ids::ConnectionId::from_string(
                        target_core.clone()
                    )),
                "private target must really own its Core route"
            );
        }
        assert_private_ancestry(&f, &group.record, &peers[1].id, &peers[2].id);
        let until = cutoff.min(tokio::time::Instant::now() + Duration::from_secs(2));
        let refusal = loop {
            let refusal = f
                .server
                .service
                .canonical_replacement_continuation_refusal_for_test(&prepared.record_id)
                .unwrap();
            if refusal != "authority-busy" {
                break refusal;
            }
            assert!(
                tokio::time::Instant::now() < until,
                "private continuation stayed metadata-busy; no definitive refusal observed"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        assert_eq!(
            refusal, "forbidden",
            "actual private ancestry must definitively refuse public continuation"
        );
        peers[1].live = false;
        // After the actual definitive private-domain refusal, exercise both
        // pending source and pending target lookup. No public fallback may
        // deliver a marked sample, including to the previously held remote.
        probe(
            &mut peers,
            3,
            &[],
            "paused-private-target-never-falls-back-public",
        )
        .await;
        probe(
            &mut peers,
            2,
            &[],
            "paused-private-partner-never-falls-back-public",
        )
        .await;
        probe(
            &mut peers,
            0,
            &[],
            "paused-private-held-remote-never-enters-target",
        )
        .await;
        unpublished(&f, &candidate, &prepared.record_id);
        pause.release();
        tokio::time::timeout_at(cutoff, pause.left()).await.unwrap();
    }
    let done =
        tokio::time::timeout_at(cutoff, wait_state(&f, &key.id, &command.operation_id, true))
            .await
            .expect("private replacement must complete within its original deadline");
    assert_eq!(done.record_id, prepared.record_id);
    assert_eq!(done.result.as_ref().unwrap()["connection_id"], candidate);
    assert_eq!(done.result.as_ref().unwrap()["generation"], 2);
    assert!(done
        .effects
        .iter()
        .all(|effect| effect.state == EffectState::Confirmed && effect.attempt == 1));
    peers[1].live = false;
    // This is a required existing-policy control, not permission to invent a
    // public route for a private source. A stale primary floor must fail here;
    // such a failure is a separate real projection gap, not an allowed skip.
    probe(
        &mut peers,
        2,
        &[3],
        "completed-private-partner-to-replacement",
    )
    .await;
    probe(
        &mut peers,
        3,
        &[2],
        "completed-private-replacement-to-partner",
    )
    .await;
    probe(
        &mut peers,
        0,
        &[],
        "completed-private-public-remote-remains-excluded",
    )
    .await;
    let replay = controller.transition(command.clone()).await.unwrap();
    assert_eq!(
        replay.record_id.as_ref().map(|id| id.as_str()),
        Some(done.record_id.as_str())
    );
    assert_eq!(replay.result, done.result);
    let replayed = status(&f, &key.id, &command.operation_id).await;
    assert_eq!(replayed.effects, done.effects);
    let returning = TransitionCommand {
        operation_id: "private-successor-return-to-held-remote".into(),
        conversation_id: cid.clone(),
        session_id: "native-session".into(),
        connection_id: Some(candidate.clone()),
        expected_revision: revision(&f),
        intent: TransitionIntent::ConsultReturn {
            consultation_record_id: group.record.clone(),
            retained_connid: peers[0].id.clone(),
        },
        payload_extensions: Default::default(),
    };
    controller.transition(returning.clone()).await.unwrap();
    let returned = wait_state(&f, &key.id, &returning.operation_id, true).await;
    assert_eq!(
        returned.result.as_ref().unwrap()["retired_connection_ids"],
        json!([peers[2].id])
    );
    peers[2].live = false;
    probe(
        &mut peers,
        0,
        &[3],
        "private-successor-return-held-remote-to-new-device",
    )
    .await;
    probe(
        &mut peers,
        3,
        &[0],
        "private-successor-return-new-device-to-held-remote",
    )
    .await;
    let return_replay = controller.transition(returning.clone()).await.unwrap();
    assert_eq!(
        return_replay.record_id.as_ref().map(|id| id.as_str()),
        Some(returned.record_id.as_str())
    );
    assert_eq!(return_replay.result, returned.result);
    let return_replayed = status(&f, &key.id, &returning.operation_id).await;
    assert_eq!(return_replayed.effects, returned.effects);
    drop(frozen);
    for peer in peers {
        peer.client.shutdown().await.unwrap();
    }
    drop(controller);
    f.server.shutdown().await.unwrap();
}

fn assert_private_ancestry(f: &Fixture, record: &str, primary: &str, partner: &str) {
    let db = rusqlite::Connection::open(&f.config.database).unwrap();
    let pairs: Vec<(String, String)> = db.prepare("SELECT connection_id,partner_connection_id FROM native_private_content_floors WHERE operation_record_id=?1 ORDER BY connection_id").unwrap()
        .query_map([record], |r| Ok((r.get(0)?, r.get(1)?))).unwrap()
        .collect::<std::result::Result<_, _>>().unwrap();
    let mut expected = vec![
        (primary.to_owned(), partner.to_owned()),
        (partner.to_owned(), primary.to_owned()),
    ];
    expected.sort();
    assert_eq!(
        pairs, expected,
        "real established private context must precede replacement"
    );
    let applied: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM native_consultation_attempts n JOIN operation_effects e ON e.id=n.effect_id WHERE n.operation_record_id=?1 AND n.started=1 AND n.applied_json IS NOT NULL AND e.state='confirmed')", [record], |r| r.get(0)).unwrap();
    assert!(
        applied,
        "private audience must be a real captured consult, never synthetic SQL"
    );
}
