//! Actual native handoff from a decoded private audience through public clients.
use super::*;
use connect_protocol::transitions::{OutgoingDisposition, RoleScope};

fn backup_handoff_state(
    source: &rusqlite::Connection,
    destination: &mut rusqlite::Connection,
    budget: Duration,
) -> anyhow::Result<()> {
    // Failure reporting must not wait indefinitely behind the failed runtime.
    // SQLite's run_to_completion retries Busy/Locked without a deadline.
    source.busy_timeout(Duration::ZERO)?;
    destination.busy_timeout(Duration::ZERO)?;
    let deadline = std::time::Instant::now() + budget;
    let backup = rusqlite::backup::Backup::new(source, destination)?;
    loop {
        anyhow::ensure!(
            std::time::Instant::now() < deadline,
            "handoff snapshot deadline"
        );
        match backup.step(128)? {
            rusqlite::backup::StepResult::Done => return Ok(()),
            rusqlite::backup::StepResult::More => {}
            rusqlite::backup::StepResult::Busy | rusqlite::backup::StepResult::Locked => {
                std::thread::sleep(Duration::from_millis(1));
            }
            _ => anyhow::bail!("unsupported handoff snapshot result"),
        }
    }
}

fn save_handoff_failure(
    f: &Fixture,
    context: &Value,
    peers: &[Peer],
    source_index: usize,
    allowed: &[usize],
    phase: &str,
) -> anyhow::Result<std::path::PathBuf> {
    let parent =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/gate-6/sdk-external");
    std::fs::create_dir_all(&parent)?;
    let directory = parent.join(format!("handoff-failure-{}", uuid::Uuid::new_v4()));
    init::private_directory(&directory)?;
    let database = directory.join("state.sqlite3");
    let source = rusqlite::Connection::open(&f.config.database)?;
    let mut destination = rusqlite::Connection::open(&database)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&database, std::fs::Permissions::from_mode(0o600))?;
    }
    // Metadata survives even when contention prevents a consistent database.
    let connections = peers
        .iter()
        .map(|peer| {
            json!({"id":peer.id,
                "participant":peer.participant,"handle":peer.handle,"live":peer.live,
                "sequence":peer.sequence,"elapsed_milliseconds":peer.clock.elapsed().as_millis()})
        })
        .collect::<Vec<_>>();
    let scope = json!({"case":context,"phase":phase,"source_index":source_index,
        "allowed_receiver_indices":allowed,"connections":connections,
        "captured_at_unix_milliseconds":chrono::Utc::now().timestamp_millis()});
    init::private_write(
        &directory.join("case.json"),
        &serde_json::to_vec_pretty(&scope)?,
    )?;
    let copied = backup_handoff_state(&source, &mut destination, Duration::from_secs(2));
    init::private_write(
        &directory.join("snapshot.json"),
        &serde_json::to_vec_pretty(&json!({"database_complete":copied.is_ok(),
            "error":copied.as_ref().err().map(ToString::to_string)}))?,
    )?;
    // The path identifies metadata/partial state as well as successful backups.
    // A partial SQLite file must never be reported as a consistent snapshot.
    if let Err(error) = copied {
        eprintln!(
            "Handoff failure incomplete private snapshot: {}",
            directory.display()
        );
        return Err(error);
    }
    Ok(directory)
}

async fn handoff_probe(
    f: &Fixture,
    context: &Value,
    peers: &mut [Peer],
    source_index: usize,
    allowed: &[usize],
    phase: &str,
) {
    use futures_util::FutureExt;
    let checked = std::panic::AssertUnwindSafe(super::probe(peers, source_index, allowed, phase))
        .catch_unwind()
        .await;
    if let Err(panic) = checked {
        // Every probe, including private preparation, needs its state before
        // TempDir cleanup. A snapshot failure must not replace the original panic.
        let saved = save_handoff_failure(f, context, peers, source_index, allowed, phase);
        match saved {
            Ok(directory) => eprintln!("Handoff failure private snapshot: {}", directory.display()),
            Err(error) => eprintln!("Handoff failure snapshot unavailable: {error:#}"),
        }
        std::panic::resume_unwind(panic);
    }
}

#[test]
fn handoff_failure_backup_refuses_a_locked_source_within_its_original_budget() {
    let source = rusqlite::Connection::open_in_memory().unwrap();
    source
        .execute_batch("CREATE TABLE witness(value INTEGER); INSERT INTO witness VALUES(1);")
        .unwrap();
    source
        .execute_batch("BEGIN IMMEDIATE; INSERT INTO witness VALUES(2);")
        .unwrap();
    let mut destination = rusqlite::Connection::open_in_memory().unwrap();
    let started = std::time::Instant::now();
    let error =
        backup_handoff_state(&source, &mut destination, Duration::from_millis(25)).unwrap_err();
    assert_eq!(error.to_string(), "handoff snapshot deadline");
    assert!(started.elapsed() < Duration::from_secs(1));
    source.execute_batch("ROLLBACK").unwrap();
    backup_handoff_state(&source, &mut destination, Duration::from_secs(1)).unwrap();
    let values: Vec<i64> = destination
        .prepare("SELECT value FROM witness ORDER BY value")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(values, [1]);
}

#[tokio::test]
async fn handoff_failure_snapshot_preserves_private_consistent_state_and_case_metadata() {
    let f = Fixture::with_native_mcp("EdDSA", 16).await;
    let context = json!({"diagnostic":"capture-self-check","simulated":true});
    let directory = save_handoff_failure(&f, &context, &[], 2, &[1], "capture-self-check").unwrap();
    let case: Value =
        serde_json::from_slice(&std::fs::read(directory.join("case.json")).unwrap()).unwrap();
    assert_eq!(case["case"], context);
    assert_eq!(case["source_index"], 2);
    assert_eq!(case["allowed_receiver_indices"], json!([1]));
    let status: Value =
        serde_json::from_slice(&std::fs::read(directory.join("snapshot.json")).unwrap()).unwrap();
    assert_eq!(status, json!({"database_complete":true,"error":null}));
    let saved = rusqlite::Connection::open(directory.join("state.sqlite3")).unwrap();
    assert_eq!(
        saved
            .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    let source = rusqlite::Connection::open(&f.config.database).unwrap();
    let identities = |c: &rusqlite::Connection| {
        c.query_row("SELECT COUNT(*) FROM identities", [], |r| {
            r.get::<_, u64>(0)
        })
        .unwrap()
    };
    assert!(identities(&saved) > 0);
    assert_eq!(identities(&source), identities(&saved));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        for name in ["state.sqlite3", "case.json", "snapshot.json"] {
            assert_eq!(
                std::fs::metadata(directory.join(name))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
    eprintln!(
        "Handoff snapshot self-check private state: {}",
        directory.display()
    );
    f.server.shutdown().await.unwrap();
}

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
    handoff_case_with_control(
        disposition,
        role_scope,
        initial_activation,
        sdk_algorithm,
        false,
        None,
    )
    .await;
}
async fn handoff_case_with_control(
    disposition: OutgoingDisposition,
    role_scope: RoleScope,
    initial_activation: bool,
    sdk_algorithm: Option<&'static str>,
    mcp: bool,
    host: Option<&crate::canonical_agent_rehearsal::Settings>,
) {
    if std::env::var_os("CONNECT_GATE6_NATIVE_DIAGNOSTICS").as_deref()
        == Some(std::ffi::OsStr::new("1"))
    {
        let _ = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "warn".into()),
            )
            .try_init();
    }
    let codecs::Running {
        f,
        mut clients,
        cid,
        sh,
        th,
    } = if let Some(algorithm) = sdk_algorithm {
        codecs::setup_started(
            if let Some(settings) = host {
                Fixture::with_native_mcp_in(algorithm, 16, settings.state_parent()).await
            } else if mcp {
                Fixture::with_native_mcp(algorithm, 16).await
            } else {
                Fixture::with_native_end_scope(algorithm, 16).await
            },
            (),
            "codec-target-offer",
            "codec-source-offer",
        )
        .await
        .0
    } else {
        codecs::setup().await
    };
    if let Some(settings) = host {
        settings.retain_state(&f);
    }
    let case_context = json!({"algorithm":f.client.signing_algorithm,"outgoing":disposition,
        "role_scope":role_scope,"initial_activation":initial_activation,"mcp":mcp,"actual_host":host.is_some()});
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
        handoff_probe(
            &f,
            &case_context,
            &mut peers,
            RA,
            &[PA],
            "initial-handoff-public-forward",
        )
        .await;
        handoff_probe(
            &f,
            &case_context,
            &mut peers,
            PA,
            &[RA],
            "initial-handoff-public-reverse",
        )
        .await;
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
    handoff_probe(
        &f,
        &case_context,
        &mut peers,
        PA,
        &[CA],
        "handoff-private-forward",
    )
    .await;
    handoff_probe(
        &f,
        &case_context,
        &mut peers,
        CA,
        &[PA],
        "handoff-private-reverse",
    )
    .await;
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
            if mcp {
                crate::canonical_mcp::TransitionProbe::submit(&f, &refused, &key.token)
                    .await
                    .failed_and_close(code)
                    .await;
            } else {
                controller.transition(refused.clone()).await.unwrap();
            }
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
        handoff_probe(
            &f,
            &case_context,
            &mut peers,
            PA,
            &[CA],
            "handoff-refused-keeps-private-forward",
        )
        .await;
        handoff_probe(
            &f,
            &case_context,
            &mut peers,
            CA,
            &[PA],
            "handoff-refused-keeps-private-reverse",
        )
        .await;
        handoff_probe(
            &f,
            &case_context,
            &mut peers,
            RA,
            &[],
            "handoff-refused-keeps-remote-held",
        )
        .await;
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
        let mcp_probe = if let Some(settings) = host {
            Some(crate::canonical_agent_rehearsal::submit(&f, &command, settings).await)
        } else if mcp {
            Some(crate::canonical_mcp::TransitionProbe::submit(&f, &command, &key.token).await)
        } else {
            None
        };
        let sdk_probe = if sdk_algorithm.is_some() && !mcp {
            Some(super::sdk_movements::handoff(&f, &command, &key.token, &group).await)
        } else if !mcp {
            controller.transition(command.clone()).await.unwrap();
            None
        } else {
            None
        };
        let record = wait_state(&f, &key.id, &command.operation_id, true).await;
        (sdk_probe, mcp_probe, record)
    };
    let (sdk_probe, mcp_probe, record) = if initial_activation {
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
    if let Some(probe) = &mcp_probe {
        probe.completed(&record).await;
    }
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
    handoff_probe(
        &f,
        &case_context,
        &mut peers,
        RA,
        &remote_receivers,
        "handoff-retained-remote",
    )
    .await;
    handoff_probe(
        &f,
        &case_context,
        &mut peers,
        CA,
        &incoming_receivers,
        "handoff-incoming-speaker",
    )
    .await;
    if disposition == OutgoingDisposition::Observe {
        handoff_probe(
            &f,
            &case_context,
            &mut peers,
            PA,
            &[],
            "handoff-observer-cannot-publish",
        )
        .await;
    }
    // Wait beyond both planning/attempt reader intervals. Continued audio
    // follows current physical lifetime policy, never retained command reads.
    tokio::time::sleep(Duration::from_secs(5)).await;
    handoff_probe(
        &f,
        &case_context,
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
        handoff_probe(
            &f,
            &case_context,
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
    if let Some(probe) = mcp_probe {
        probe.replay_and_close().await;
    } else {
        controller.transition(command).await.unwrap();
    }
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

#[tokio::test]
async fn production_mcp_handoff_keeps_decoded_native_audio_roles_original_recovery_and_one_effect()
{
    for algorithm in ["EdDSA", "ES256"] {
        for (outgoing, scope) in [
            (OutgoingDisposition::Observe, RoleScope::Session),
            (OutgoingDisposition::Leave, RoleScope::Session),
            (OutgoingDisposition::Observe, RoleScope::Conversation),
            (OutgoingDisposition::Leave, RoleScope::Conversation),
        ] {
            handoff_case_with_control(outgoing, scope, false, Some(algorithm), true, None).await;
            eprintln!("MCP handoff {algorithm}:outgoing={outgoing:?},scope={scope:?}, independent decoded native audio, original recovery, one effect, all owners joined");
        }
    }
}

#[tokio::test]
#[ignore = "Explicit single-scene diagnostic; not the full eight-scene qualification"]
async fn diagnostic_mcp_leave_conversation_handoff_scene() {
    handoff_case_with_control(
        OutgoingDisposition::Leave,
        RoleScope::Conversation,
        false,
        Some("EdDSA"),
        true,
        None,
    )
    .await;
}

pub(crate) async fn host_rehearsal(settings: &crate::canonical_agent_rehearsal::Settings) {
    handoff_case_with_control(
        OutgoingDisposition::Observe,
        RoleScope::Session,
        false,
        Some("EdDSA"),
        true,
        Some(settings),
    )
    .await;
}
