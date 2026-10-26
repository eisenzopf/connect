//! Actual native handoff from a decoded private audience through public clients.
use super::*;
use connect_protocol::transitions::{OutgoingDisposition, RoleScope};
#[tokio::test]
async fn production_native_private_handoff_commits_roles_and_retains_remote_audio() {
    for (disposition, role_scope) in [
        (OutgoingDisposition::Observe, RoleScope::Session),
        (OutgoingDisposition::Leave, RoleScope::Session),
        (OutgoingDisposition::Observe, RoleScope::Conversation),
        (OutgoingDisposition::Leave, RoleScope::Conversation),
    ] {
        handoff_case(disposition, role_scope, false).await;
    }
}
#[tokio::test]
async fn production_initial_activation_after_public_handoff_uses_observed_private_flow_without_new_packets(
) {
    handoff_case(OutgoingDisposition::Leave, RoleScope::Session, true).await;
}
async fn handoff_case(
    disposition: OutgoingDisposition,
    role_scope: RoleScope,
    initial_activation: bool,
) {
    handoff_case_with_sdk(disposition, role_scope, initial_activation, None).await
}
pub(super) async fn handoff_case_with_sdk(
    disposition: OutgoingDisposition,
    role_scope: RoleScope,
    initial_activation: bool,
    sdk_algorithm: Option<&'static str>,
) {
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
    let pid = |identity: &str| {
        room.participants
            .iter()
            .find(|p| p.identity_id == identity)
            .unwrap()
            .participant_id
            .clone()
    };
    let pa = clients.pop().unwrap();
    let ra = clients.pop().unwrap();
    let make = |client, identity: String, participant, id: &str, h| Peer {
        client,
        config: None,
        identity,
        participant,
        id: id.into(),
        handle: h,
        clock: tokio::time::Instant::now(),
        sequence: 1,
        live: true,
    };
    let mut peers = vec![
        make(
            ra,
            f.peer.identity_id.clone(),
            pid(&f.peer.identity_id),
            "codec-source",
            sh,
        ),
        make(
            pa,
            f.client.identity_id.clone(),
            pid(&f.client.identity_id),
            "codec-target",
            th,
        ),
    ];
    let candidate = enroll_scoped(
        &f,
        &mut peers[PA].client,
        &cid,
        "handoff-human",
        221,
        false,
        true,
    )
    .await;
    peers.push(candidate);
    if initial_activation {
        // The third Participant has accepted before either public source
        // supplies real flow. It cannot be covered by the other two.
        probe(&mut peers, RA, &[PA], "initial-handoff-public-forward").await;
        probe(&mut peers, PA, &[RA], "initial-handoff-public-reverse").await;
        let db = rusqlite::Connection::open(&f.config.database).unwrap();
        assert_eq!(
            db.query_row(
                "SELECT state FROM sessions WHERE id='native-session'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "accepted"
        );
    }
    let group = begin(&f, &mut peers, &cid, "private-before-handoff", PA, RA, CA).await;
    probe(&mut peers, PA, &[CA], "handoff-private-forward").await;
    probe(&mut peers, CA, &[PA], "handoff-private-reverse").await;
    wait_state(&f, &group.key.id, &group.command.operation_id, true).await;
    let initial_flow_count = if initial_activation {
        let db = rusqlite::Connection::open(&f.config.database).unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while db
                .query_row(
                    "SELECT COUNT(*) FROM native_connection_flow_facts",
                    [],
                    |r| r.get::<_, u64>(0),
                )
                .unwrap()
                != 3
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            db.query_row(
                "SELECT state FROM sessions WHERE id='native-session'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "accepted",
            "a private consultation is not public group readiness"
        );
        Some(3u64)
    } else {
        None
    };
    let target_op = format!("offer-{}", peers[CA].id);
    let policy = peers[CA]
        .client
        .operation_get(
            &target_op,
            CommandScope {
                conversation_id: Some(cid.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let policy = serde_json::to_value(policy).unwrap()["record_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope
        .operations
        .extend(["session.handoff".into(), "operation.cancel".into()]);
    let key = f
        .server
        .store
        .issue(
            &peers[PA].identity,
            scope,
            chrono::Utc::now().timestamp() + 300,
        )
        .unwrap();
    let mut cfg = canonical_http_config(&f);
    cfg.credential = key.token.clone();
    let mut controller = CanonicalHttpClient::open(cfg).await.unwrap();
    let before = peers[PA]
        .client
        .list_conversations(Default::default())
        .await
        .unwrap()
        .conversations
        .into_iter()
        .find(|r| r.id == cid)
        .unwrap();
    let command = TransitionCommand {
        operation_id: "native-public-handoff".into(),
        conversation_id: cid.clone(),
        session_id: "native-session".into(),
        connection_id: Some(peers[PA].id.clone()),
        expected_revision: revision(&f),
        intent: TransitionIntent::Handoff {
            source_participant: peers[PA].participant.clone(),
            target_participant: peers[CA].participant.clone(),
            retained_connid: peers[RA].id.clone(),
            candidate_connid: Some(peers[CA].id.clone()),
            outgoing: disposition,
            role_scope,
            expected_conversation_revision: (role_scope == RoleScope::Conversation)
                .then_some(before.revision),
        },
        payload_extensions: std::collections::BTreeMap::from([
            ("policy_ref".into(), json!(policy)),
            ("consultation_record_id".into(), json!(group.record)),
        ]),
    };
    if disposition == OutgoingDisposition::Observe && role_scope == RoleScope::Session {
        for (id, missing_private_reference, code) in [
            ("refused-copied-policy", false, "forbidden"),
            ("refused-missing-private-reference", true, "forbidden"),
        ] {
            let mut refused = command.clone();
            refused.operation_id = id.into();
            if !missing_private_reference {
                refused
                    .payload_extensions
                    .insert("policy_ref".into(), json!(group.record));
            } else {
                refused.payload_extensions.remove("consultation_record_id");
            }
            controller.transition(refused.clone()).await.unwrap();
            let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            let failed = loop {
                let value = controller
                    .operation_get(
                        id,
                        CommandScope {
                            conversation_id: Some(cid.clone()),
                            ..Default::default()
                        },
                    )
                    .await
                    .unwrap();
                let value = serde_json::to_value(value).unwrap();
                if value["state"] == "failed" {
                    break value;
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "preflight did not fail: {value}"
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            };
            assert_eq!(
                failed["problem"],
                json!({"code":code,"source":"native-handoff"})
            );
            let saved = status(&f, &key.id, id).await;
            assert!(saved.effects.is_empty());
            assert!(saved.result.is_none());
            let replay = controller.transition(refused).await.unwrap();
            assert_eq!(serde_json::to_value(replay).unwrap()["state"], "failed");
        }
        probe(
            &mut peers,
            PA,
            &[CA],
            "handoff-refused-keeps-private-forward",
        )
        .await;
        probe(
            &mut peers,
            CA,
            &[PA],
            "handoff-refused-keeps-private-reverse",
        )
        .await;
        probe(&mut peers, RA, &[], "handoff-refused-keeps-remote-held").await;
    }
    if disposition == OutgoingDisposition::Leave {
        // Force the actual native EOF/retirement observation to precede
        // durable role projection. A known leave must consume its exact
        // retired source fact instead of waiting forever on the old live
        // generation. This trigger affects only the isolated fixture.
        let c = rusqlite::Connection::open(&f.config.database).unwrap();
        c.execute_batch(&format!(
            "CREATE TRIGGER handoff_wait_for_native_retirement
                 BEFORE UPDATE OF state ON session_participants
                 WHEN NEW.participant_id='{}' AND NEW.state='left'
                   AND NOT EXISTS(SELECT 1 FROM connections WHERE id='{}' AND state='retired')
                 BEGIN SELECT RAISE(ABORT,'wait for actual native handoff retirement'); END;",
            peers[PA].participant, peers[PA].id
        ))
        .unwrap();
    }
    let control = async {
        let sdk_probe = if sdk_algorithm.is_some() {
            Some(super::sdk_movements::handoff(&f, &command, &key.token, &group).await)
        } else {
            controller.transition(command.clone()).await.unwrap();
            None
        };
        let record = wait_state(&f, &key.id, &command.operation_id, true).await;
        (sdk_probe, record)
    };
    let (sdk_probe, record) = if initial_activation {
        // This separate contract intentionally proves historical observed flow
        // without sending another packet before public activation.
        control.await
    } else {
        with_live_media(
            &mut peers,
            (disposition == OutgoingDisposition::Leave).then_some(PA),
            control,
        )
        .await
    };
    let wire = controller
        .operation_get(
            &command.operation_id,
            CommandScope {
                conversation_id: Some(cid.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let wire = serde_json::to_value(wire).unwrap();
    assert_eq!(wire["state"], "completed");
    assert_eq!(wire["record_id"], record.record_id);
    assert_eq!(wire["result"], record.result.as_ref().unwrap().clone());
    assert_eq!(record.effects.len(), 1);
    assert_eq!(record.effects[0].attempt, 1);
    if let Some(probe) = sdk_probe {
        super::sdk_movements::handoff_completed(
            &f,
            probe,
            &command,
            record.result.as_ref().unwrap(),
        )
        .await;
    }
    if let Some(flow_count) = initial_flow_count {
        let db = rusqlite::Connection::open(&f.config.database).unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while db
                .query_row(
                    "SELECT state FROM sessions WHERE id='native-session'",
                    [],
                    |r| r.get::<_, String>(0),
                )
                .unwrap()
                != "active"
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(db.query_row("SELECT COUNT(*) FROM native_connection_flow_facts",[],|r|r.get::<_,u64>(0)).unwrap(),flow_count,"promotion must reuse actual observed flow; no new native packet is sent before this assertion");
        let start = f
            .server
            .store
            .session_started_template(&cid, "native-session")
            .unwrap()
            .0;
        assert_eq!(
            start
                .participants_present
                .iter()
                .collect::<std::collections::BTreeSet<_>>(),
            std::collections::BTreeSet::from([&peers[RA].participant, &peers[CA].participant])
        );
        assert_eq!(
            start
                .active_connections
                .iter()
                .map(|c| c.connid.as_str())
                .collect::<std::collections::BTreeSet<_>>(),
            std::collections::BTreeSet::from([peers[RA].id.as_str(), peers[CA].id.as_str()])
        );
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM session_started_facts", [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            1
        );
    }
    let result = record.result.as_ref().unwrap();
    assert_eq!(result["retained_connection_ids"], json!([peers[RA].id]));
    assert_eq!(result["admitted_connection_ids"], json!([peers[CA].id]));
    assert_eq!(result["native_handoff"]["role_scope"], json!(role_scope));
    let after = peers[CA]
        .client
        .list_conversations(Default::default())
        .await
        .unwrap()
        .conversations
        .into_iter()
        .find(|r| r.id == cid)
        .unwrap();
    for index in [PA, CA] {
        let old = before
            .participants
            .iter()
            .find(|p| p.participant_id == peers[index].participant)
            .unwrap();
        let new = after
            .participants
            .iter()
            .find(|p| p.participant_id == peers[index].participant)
            .unwrap();
        let expected = if role_scope == RoleScope::Conversation {
            if index == PA {
                "observer"
            } else {
                before
                    .participants
                    .iter()
                    .find(|p| p.participant_id == peers[PA].participant)
                    .unwrap()
                    .role
                    .as_str()
            }
        } else {
            old.role.as_str()
        };
        assert_eq!(new.role, expected);
    }
    assert_eq!(
        after.revision,
        before.revision + u64::from(role_scope == RoleScope::Conversation)
    );
    if disposition == OutgoingDisposition::Leave {
        peers[PA].live = false;
    }
    let remote_receivers = if disposition == OutgoingDisposition::Observe {
        vec![CA, PA]
    } else {
        vec![CA]
    };
    let incoming_receivers = if disposition == OutgoingDisposition::Observe {
        vec![RA, PA]
    } else {
        vec![RA]
    };
    {
        use futures_util::FutureExt;
        let checked = std::panic::AssertUnwindSafe(probe(
            &mut peers,
            RA,
            &remote_receivers,
            "handoff-retained-remote",
        ))
        .catch_unwind()
        .await;
        if let Err(panic) = checked {
            // Preserve a consistent private snapshot before TempDir cleanup.
            // Do not retry the handoff or weaken the strict audio assertions.
            let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../.local/gate-6/sdk-external")
                .join(format!("handoff-failure-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&directory).unwrap();
            let source = rusqlite::Connection::open(&f.config.database).unwrap();
            let mut destination =
                rusqlite::Connection::open(directory.join("state.sqlite3")).unwrap();
            rusqlite::backup::Backup::new(&source, &mut destination)
                .unwrap()
                .run_to_completion(128, Duration::from_millis(1), None)
                .unwrap();
            let scope = json!({"algorithm":f.client.signing_algorithm,"command":command,"source":peers[RA].id,"incoming":peers[CA].id,"outgoing":peers[PA].id,"allowed":remote_receivers});
            std::fs::write(
                directory.join("case.json"),
                serde_json::to_vec_pretty(&scope).unwrap(),
            )
            .unwrap();
            eprintln!("Handoff failure private snapshot: {}", directory.display());
            std::panic::resume_unwind(panic);
        }
    }
    probe(
        &mut peers,
        CA,
        &incoming_receivers,
        "handoff-incoming-speaker",
    )
    .await;
    if disposition == OutgoingDisposition::Observe {
        probe(&mut peers, PA, &[], "handoff-observer-cannot-publish").await;
    }
    // Wait beyond both planning/attempt reader intervals. Continued audio
    // follows current physical lifetime policy, never retained command reads.
    tokio::time::sleep(Duration::from_secs(5)).await;
    probe(
        &mut peers,
        RA,
        &remote_receivers,
        "handoff-after-reader-expiry",
    )
    .await;
    if disposition == OutgoingDisposition::Observe {
        peers[PA].client.close();
        peers[PA].live = false;
        tokio::time::sleep(Duration::from_millis(300)).await;
        probe(
            &mut peers,
            RA,
            &[CA],
            "handoff-survives-outgoing-observer-eof",
        )
        .await;
    }
    wait_slots(&f, 2).await;
    let snapshot = controller
        .conversation_snapshot(&cid, serde_json::from_value(json!({})).unwrap())
        .await
        .unwrap();
    let snapshot = serde_json::to_value(snapshot).unwrap();
    let outgoing = snapshot["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["kind"] == "connection" && item["data"]["connection_id"] == peers[PA].id)
        .unwrap();
    assert_eq!(outgoing["data"]["state"], "retired");
    controller.transition(command).await.unwrap();
    assert_eq!(
        status(&f, &key.id, "native-public-handoff").await.effects[0].attempt,
        1
    );
    for p in peers {
        p.client.shutdown().await.unwrap();
    }
    wait_slots(&f, 0).await;
    f.server.shutdown().await.unwrap();
}
