//! Installed Python wheel uses the original Rust helper and signed listener.
use super::*;
use base64::Engine;
use connect_client::sdk;
use sha2::Digest;
use std::{os::unix::fs::PermissionsExt, path::PathBuf, process::Stdio};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

async fn run(algorithm: &str) {
    let script = PathBuf::from(std::env::var_os("CONNECT_GATE6_PYTHON_CONSUMER").unwrap())
        .canonicalize()
        .unwrap();
    assert!(script.starts_with(
        std::env::current_dir()
            .unwrap()
            .join(".local")
            .canonicalize()
            .unwrap()
    ));
    let python = PathBuf::from(std::env::var_os("CONNECT_GATE6_PYTHON").unwrap());
    assert!(python.is_absolute() && python.is_file());
    let project = script.parent().unwrap();
    let mut f = Fixture::with_extra_scopes(
        algorithm,
        8,
        true,
        true,
        true,
        &["operation.cancel", "operation.reconcile"],
    )
    .await;
    f.server.shutdown().await.unwrap();
    f.config.limits.message_bytes = 65536;
    f.server = RunningCanonicalServer::start(f.config.clone())
        .await
        .unwrap();
    f.client.endpoint = f.server.address;
    crate::canonical_native_connections::ready_endpoint_with_capabilities(
        &f,
        vec!["audio".into(), "video".into(), "data".into()],
    )
    .await;
    let mut peer_scope = connect_store::Scope::collaboration(vec!["*".into()]);
    peer_scope.operations.extend([
        "stream.subscribe".into(),
        "stream.unsubscribe".into(),
        "connection.end".into(),
    ]);
    let peer_control = crate::canonical_native_connections::party_with_scope(
        &f,
        &f.peer.identity_id,
        72,
        vec!["audio".into(), "video".into(), "data".into()],
        peer_scope,
    );
    let peer_config = f.temp.path().join("python-peer.toml");
    init::private_write(
        &peer_config,
        toml::to_string(&sdk::ClientConfig {
            control: peer_control.clone(),
            tenant_id: "tnt_default".parse().unwrap(),
            application: None,
        })
        .unwrap()
        .as_bytes(),
    )
    .unwrap();
    let options = || sdk::RequestOptions::new(Duration::from_secs(4)).unwrap();
    let native = sdk::Client::connect(f.client.clone(), "tnt_default".parse().unwrap())
        .await
        .unwrap();
    let room = native
        .create_conversation(
            vec![f.peer.identity_id.parse().unwrap()],
            sdk::ConversationPolicy::Persistent,
            sdk::MutationOptions {
                operation_id: "python-native-create".parse().unwrap(),
                request: options(),
                extensions: None,
            },
        )
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap()
        .value;
    let recipient = room
        .refresh(options())
        .await
        .unwrap()
        .participants
        .into_iter()
        .find(|p| p.identity_id.as_str() == f.peer.identity_id)
        .unwrap()
        .id;
    let cid = room.id().as_str().to_owned();
    native.close(options()).await.unwrap();
    let config = f.temp.path().join("python-native.toml");
    let write_config = |f: &Fixture| {
        let config_value = sdk::ClientConfig {
            control: f.client.clone(),
            tenant_id: "tnt_default".parse().unwrap(),
            application: Some(connect_client::application::ApplicationClientConfig {
                endpoint: f.server.http_address.unwrap(),
                credential: f.client.credential.clone(),
                expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
                timeout_ms: 4000,
            }),
        };
        let next = config.with_extension("toml.next");
        init::private_write(&next, toml::to_string(&config_value).unwrap().as_bytes()).unwrap();
        std::fs::rename(next, &config).unwrap();
    };
    write_config(&f);
    let pids = f.temp.path().join("python-native-pids");
    let bin = f.temp.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let wrapper = bin.join("connect");
    init::private_write(&wrapper,br#"#!/usr/bin/env python3
import os,sys
path=os.environ['CONNECT_G6_PYTHON_PIDS']+('.validators' if sys.argv[1:2]==['sdk-validate-envelope'] else '')
with open(path,'a') as stream:stream.write(str(os.getpid())+'\n')
os.execv(os.environ['CONNECT_G6_PYTHON_CONNECT'],[os.environ['CONNECT_G6_PYTHON_CONNECT'],*sys.argv[1:]])
"#).unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let spec = f.temp.path().join("python-native-spec.json");
    init::private_write(&spec,&serde_json::to_vec(&json!({"project":project,"av_fixture_dir":crate::language_media::fixtures(),"config":config,
        "cid":cid,"recipient":recipient,"peer_identity":f.peer.identity_id,"peer_config":peer_config,
        "http":format!("http://{}",f.server.http_address.unwrap()),"credential":f.client.credential,
        "saved":f.temp.path().join("python-native-reference.json")})).unwrap()).unwrap();
    let mut path = vec![bin];
    path.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    let mut child = tokio::process::Command::new(&python)
        .arg("-I")
        .arg(&script)
        .arg(&spec)
        .current_dir(project)
        .env("PATH", std::env::join_paths(path).unwrap())
        .env_remove("PYTHONPATH")
        .env_remove("PYTHONHOME")
        .env_remove("PYTHONUSERBASE")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("CONNECT_G6_PYTHON_PIDS", &pids)
        .env("CONNECT_G6_PYTHON_CONNECT", env!("CARGO_BIN_EXE_connect"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(
            std::fs::File::create(project.join(format!("python-native-{algorithm}.stderr")))
                .unwrap(),
        ))
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    async fn phase(
        lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
        expected: &str,
    ) -> Value {
        let line = tokio::time::timeout(Duration::from_secs(15), lines.next_line())
            .await
            .unwrap()
            .unwrap()
            .expect("Python native phase result");
        assert!(line.len() < 4096);
        let value: Value = serde_json::from_str(&line).unwrap();
        if value["phase"] == "failed" {
            eprintln!("installed-python-native-failure {value}");
        }
        assert_eq!(value["phase"], expected);
        value
    }
    fn original(f: &Fixture, value: &Value, cid: &str) {
        let actor = f.server.store.authenticate(&f.client.credential).unwrap();
        let durable = f
            .server
            .store
            .execute_operation_action(
                &actor.credential_id,
                connect_protocol::transitions::OperationAction::Get {
                    operation_id: "python-native-send".into(),
                },
            )
            .unwrap();
        assert_eq!(durable.record_id, value["record_id"]);
        assert_eq!(durable.operation_id, "python-native-send");
        assert_eq!(durable.command_type, "send");
        assert_eq!(durable.conversation_id.as_deref(), Some(cid));
        assert_eq!(
            durable.state,
            connect_protocol::transitions::OperationState::Completed
        );
        assert!(
            durable
                .causal_event_ids
                .iter()
                .any(|id| value["event_id_sha256"]
                    == hex::encode(sha2::Sha256::digest(id.as_bytes())))
        );
    }
    let participants = phase(&mut lines, "participants-completed").await;
    assert_eq!(participants["participant_epoch_advanced"], true);
    assert_eq!(participants["stale_revision_refused"], true);
    let sessions = phase(&mut lines, "sessions-ready").await;
    assert_eq!(sessions["incoming_decision_completed"], true);
    let av = phase(&mut lines, "av-completed").await;
    assert_eq!(av["native_av_utility_capture"], true);
    let av_source = crate::language_media::verify(
        &f.temp.path().join("python-av-source"),
        1100.0,
        [false, true, true],
    )
    .await;
    let av_target = crate::language_media::verify(
        &f.temp.path().join("python-av-target"),
        880.0,
        [true, true, false],
    )
    .await;
    let data = phase(&mut lines, "data-completed").await;
    for name in [
        "native_data_typed_round_trip",
        "native_data_full_64k",
        "native_data_original_recovery",
        "native_data_one_effect_replay",
        "native_data_changed_intent_refused",
        "native_data_foreign_custody_refused",
    ] {
        assert_eq!(data[name], true);
    }
    let native = phase(&mut lines, "native-completed").await;
    assert_eq!(native["native_controls_completed"], true);
    assert_eq!(native["stream_selection_completed"], true);
    let controls = phase(&mut lines, "controls-completed").await;
    for field in [
        "participant_epoch_advanced",
        "stale_revisions_refused",
        "incoming_original_hint",
        "session_decisions_completed",
        "native_controls_completed",
        "stream_selection_completed",
        "native_media_negotiation",
        "native_rtp_local_queue",
    ] {
        assert_eq!(controls[field], true);
    }
    assert!(
        controls["native_clock_local_queue"].is_boolean()
            && controls["native_clock_unnegotiated_refused"].is_boolean()
    );
    assert_ne!(
        controls["native_clock_local_queue"],
        controls["native_clock_unnegotiated_refused"]
    );
    let database = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let source: String = database.query_row(
        "SELECT id FROM session_invitation_sources WHERE session_id='python-control-solo' AND operation_id='python-control-extend'", [], |row| row.get(0),
    ).unwrap();
    assert_ne!(source, "python-control-extend");
    assert_eq!(
        controls["incoming_source_sha256"],
        hex::encode(sha2::Sha256::digest(source.as_bytes()))
    );
    for (id, command) in [
        ("python-control-create", "create"),
        (
            "python-control-member-return",
            "conversation.participant.add",
        ),
        ("python-control-accept", "session.accept"),
        ("python-control-offer", "connection.offer"),
        ("python-control-quality", "connection.update"),
        ("python-control-codecs", "connection.update"),
        ("python-control-subscribe", "stream.subscribe"),
        ("python-control-unsubscribe", "stream.unsubscribe"),
        ("python-control-native-end", "connection.end"),
        ("python-control-session-end", "session.end"),
        ("python-control-reject", "session.reject"),
        ("python-control-cancel", "session.cancel"),
    ] {
        let (count, stored_command, state): (u64, String, String) = database.query_row(
            "SELECT COUNT(*), MIN(command_type), MIN(state) FROM operations WHERE operation_id=?1 AND conversation_id=?2",
            rusqlite::params![id, controls["conversation_id"].as_str().unwrap()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap();
        assert_eq!(
            (count, stored_command.as_str(), state.as_str()),
            (1, command, "completed")
        );
    }
    for format in ["text", "json", "binary"] {
        let expected = match format {
            "text" => format!("🦀{}", "x".repeat(65532)).into_bytes(),
            "json" => serde_json::to_vec(&json!({"data":"x".repeat(65536-11)})).unwrap(),
            _ => (0..65536).map(|i| i as u8).collect::<Vec<_>>(),
        };
        assert_eq!(expected.len(), 65536);
        for (suffix, side) in [("message", "source"), ("return-message", "target")] {
            let message = format!("python-data-{format}-{suffix}");
            let (body, encoding): (String, String) = database
                .query_row(
                    "SELECT body,body_encoding FROM messages WHERE id=?1 AND conversation_id=?2",
                    rusqlite::params![message, controls["conversation_id"].as_str().unwrap()],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            let bytes = if format == "binary" {
                assert_eq!(encoding, "base64");
                base64::engine::general_purpose::STANDARD
                    .decode(body)
                    .unwrap()
            } else {
                assert_eq!(encoding, "utf8");
                body.into_bytes()
            };
            assert_eq!(bytes, expected);
            let operation = format!(
                "python-data-{format}-{}",
                if side == "source" { "send" } else { "return" }
            );
            let connection = format!("python-data-{format}-{side}");
            let (count,state,record):(u64,String,String)=database.query_row("SELECT COUNT(*),MIN(o.state),MIN(o.record_id) FROM operations o WHERE o.operation_id=?1 AND o.connection_id=?2 AND o.command_type='message.send'",rusqlite::params![operation,connection],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
            let effects: u64 = database
                .query_row(
                    "SELECT COUNT(*) FROM operation_effects WHERE operation_record_id=?1",
                    [record],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!((count, state.as_str(), effects), (1, "completed", 1));
        }
        assert_eq!(
            database
                .query_row(
                    "SELECT COUNT(*) FROM messages WHERE id=?1",
                    [format!("python-data-{format}-changed")],
                    |r| r.get::<_, u64>(0)
                )
                .unwrap(),
            0
        );
    }
    drop(database);
    let stopped = phase(&mut lines, "stopped").await;
    for field in [
        "same_id_replay",
        "changed_intent_refused",
        "signed_finite_journal",
        "typed_capability_refusal",
        "typed_message_event",
        "local_wait_cancelled_read_only",
        "durable_cancel_original_preserved",
        "unattributed_reconciliation_refused",
        "canonical_http_same_original_record",
        "receipt_queued_without_delivery",
    ] {
        assert_eq!(stopped[field], true);
    }
    original(&f, &stopped, &cid);
    let application = |credential| connect_client::application::ApplicationClientConfig {
        endpoint: f.server.http_address.unwrap(),
        credential,
        expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
        timeout_ms: 4000,
    };
    let mut sender = connect_client::application::ApplicationClient::open(application(
        f.client.credential.clone(),
    ))
    .await
    .unwrap();
    async fn receipts(
        client: &mut connect_client::application::ApplicationClient,
        cid: &str,
        pid: &str,
        expected: connect_protocol::receipts::DeliveryState,
    ) -> connect_protocol::receipts::RecipientReceipt {
        let result = client
            .execute(
                connect_client::application::ApplicationOperation::try_from(
                    connect_protocol::Operation::MessageReceipts {
                        conversation_id: cid.into(),
                        message_id: "python-native-message".into(),
                        after: None,
                        limit: 63,
                    },
                )
                .unwrap(),
            )
            .await
            .unwrap();
        let connect_protocol::Outcome::MessageReceipts(page) = result else {
            panic!("Expected attributed receipt page")
        };
        assert_eq!(page.receipts.len(), 1);
        assert_eq!(page.receipts[0].state, expected);
        assert!(page.receipts[0].membership_epoch > 0);
        assert_eq!(page.receipts[0].conversation_id, cid);
        assert_eq!(page.receipts[0].message_id, "python-native-message");
        assert_eq!(page.receipts[0].participant_id, pid);
        assert_eq!(page.receipts[0].channel, "native");
        page.receipts[0].clone()
    }
    fn same_receipt(value: &Value, fact: &connect_protocol::receipts::RecipientReceipt) {
        assert_eq!(value["membership_epoch"], fact.membership_epoch);
        assert_eq!(value["observed_at"], fact.observed_at);
        assert_eq!(
            value["delivery_id_sha256"],
            hex::encode(sha2::Sha256::digest(fact.delivery_id.as_bytes()))
        );
        assert_eq!(fact.provenance, "client_reported");
    }
    receipts(
        &mut sender,
        &cid,
        recipient.as_str(),
        connect_protocol::receipts::DeliveryState::Queued,
    )
    .await;
    let mut recipient_app = connect_client::application::ApplicationClient::open(application(
        peer_control.credential.clone(),
    ))
    .await
    .unwrap();
    recipient_app
        .execute(
            connect_client::application::ApplicationOperation::try_from(
                connect_protocol::Operation::MessageReceipt {
                    request_id: "python-native-recipient-delivered".into(),
                    conversation_id: cid.clone(),
                    message_id: "python-native-message".into(),
                    state: connect_protocol::receipts::ClientReceiptState::Delivered,
                },
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let delivered = receipts(
        &mut sender,
        &cid,
        recipient.as_str(),
        connect_protocol::receipts::DeliveryState::Delivered,
    )
    .await;
    input.write_all(b"receipts\n").await.unwrap();
    let observed = phase(&mut lines, "receipts-observed").await;
    assert_eq!(observed["attributed_delivery"], true);
    same_receipt(&observed, &delivered);
    let mut read_reporter = CanonicalClient::open(peer_control).await.unwrap();
    let suppressed = read_reporter
        .report_read(
            "python-native-read-suppressed",
            &cid,
            "python-native-message",
            chrono::Utc::now(),
        )
        .await
        .unwrap();
    assert_eq!(
        suppressed.recorded,
        Some(connect_protocol::receipts::DeliveryState::Delivered)
    );
    recipient_app
        .execute(
            connect_client::application::ApplicationOperation::try_from(
                connect_protocol::Operation::ReceiptPreferencesSet {
                    request_id: "python-native-read-opt-in".into(),
                    conversation_id: cid.clone(),
                    delivery: true,
                    read: true,
                },
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let read = read_reporter
        .report_read(
            "python-native-read",
            &cid,
            "python-native-message",
            chrono::Utc::now(),
        )
        .await
        .unwrap();
    assert_eq!(
        read.recorded,
        Some(connect_protocol::receipts::DeliveryState::Read)
    );
    let read = receipts(
        &mut sender,
        &cid,
        recipient.as_str(),
        connect_protocol::receipts::DeliveryState::Read,
    )
    .await;
    read_reporter.shutdown().await.unwrap();
    drop((sender, recipient_app));
    input.write_all(b"read-receipt\n").await.unwrap();
    let observed = phase(&mut lines, "read-observed").await;
    assert_eq!(observed["attributed_read"], true);
    same_receipt(&observed, &read);
    f.server.shutdown().await.unwrap();
    f.server = RunningCanonicalServer::start(f.config.clone())
        .await
        .unwrap();
    f.client.endpoint = f.server.address;
    write_config(&f);
    input.write_all(b"restart\n").await.unwrap();
    let resumed = phase(&mut lines, "resumed").await;
    assert_eq!(resumed["record_id"], stopped["record_id"]);
    assert_eq!(resumed["event_id_sha256"], stopped["event_id_sha256"]);
    for field in [
        "original_reference_recovered",
        "same_cause_after_restart",
        "synchronous_original_recovered",
    ] {
        assert_eq!(resumed[field], true);
    }
    original(&f, &resumed, &cid);
    let actor = f.server.store.authenticate(&f.client.credential).unwrap();
    let administrator = f.server.store.authenticate(&f.admin).unwrap();
    let store = f.server.store.clone();
    tokio::task::spawn_blocking(move || store.revoke(&administrator, &actor.credential_id))
        .await
        .unwrap()
        .unwrap();
    input.write_all(b"revoked\n").await.unwrap();
    let completed = phase(&mut lines, "completed").await;
    for field in [
        "current_read_refused",
        "current_replay_refused",
        "current_new_mutation_refused",
        "current_receipts_refused",
    ] {
        assert_eq!(completed[field], true);
    }
    assert_eq!(completed["http_workers_joined"], 2);
    drop(input);
    assert!(tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .unwrap()
        .unwrap()
        .success());
    for (path, expected) in [(&pids, 10), (&pids.with_extension("validators"), 5)] {
        let recorded = std::fs::read_to_string(path).unwrap();
        let values: Vec<u32> = recorded.lines().map(|line| line.parse().unwrap()).collect();
        assert_eq!(values.len(), expected);
        let unique: std::collections::BTreeSet<_> = values.iter().collect();
        assert_eq!(unique.len(), expected);
        for pid in values {
            assert!(
                !std::process::Command::new("kill")
                    .args(["-0", &pid.to_string()])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .unwrap()
                    .success(),
                "Original Python helper/validator still exists"
            );
        }
    }
    let database = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    for (table, key, id, expected) in [
        ("messages", "id", "python-native-message", 1),
        ("operations", "operation_id", "python-native-send", 1),
        ("messages", "id", "python-native-forbidden-message", 0),
        (
            "operations",
            "operation_id",
            "python-native-forbidden-send",
            0,
        ),
        ("operations", "operation_id", "python-native-cancel", 1),
        (
            "operations",
            "operation_id",
            "python-native-unattributed-reconcile",
            0,
        ),
    ] {
        let count: u64 = database
            .query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE {key}=?1"),
                [id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, expected);
    }
    let (count,event):(u64,String)=database.query_row("SELECT COUNT(*),MIN(j.id) FROM visible_journal j JOIN operations o ON o.record_id=?1 WHERE j.recipient=o.owner_id AND j.conversation_id=o.conversation_id AND j.kind='message' AND j.message_id='python-native-message' AND json_extract(j.data_json,'$.operation_id')=o.operation_id AND o.operation_id='python-native-send' AND o.command_type='send' AND o.state='completed'",[stopped["record_id"].as_str().unwrap()],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
    assert_eq!(count, 1);
    assert_eq!(
        stopped["event_id_sha256"],
        hex::encode(sha2::Sha256::digest(event.as_bytes()))
    );
    drop(database);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let current = f.server.service.canonical_runtime_snapshot().unwrap();
            if current.physical_peers == 0 && current.media_readers == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    f.server.shutdown().await.unwrap();
    eprintln!(
        "installed-python-native-witness {}",
        json!({"profile":"python-native/6","decoded_language_av":[av_source,av_target],"algorithm":algorithm,
        "original_reference_recovered":true,"same_id_replay":true,"changed_intent_refused":true,"typed_capability_refusal":true,
        "signed_finite_journal":true,"same_cause_after_restart":true,"current_read_refused":true,
        "current_replay_refused":true,"current_new_mutation_refused":true,"independent_store_original_cause":true,
        "one_original_message_and_operation":true,"zero_forbidden_message_and_operation":true,
        "participant_epoch_advanced":true,"stale_revisions_refused":true,"incoming_original_hint":true,
        "session_decisions_completed":true,"native_controls_completed":true,"stream_selection_completed":true,
        "independent_control_records":true,"typed_message_event":true,
        "native_media_negotiation":true,"native_rtp_local_queue":true,
        "native_data_typed_round_trip":true,"native_data_full_64k":true,"native_data_original_recovery":true,
        "native_data_one_effect_replay":true,"native_data_changed_intent_refused":true,"native_data_foreign_custody_refused":true,
        "native_data_independent_store":true,
        "native_clock_local_queue":controls["native_clock_local_queue"],"native_clock_unnegotiated_refused":controls["native_clock_unnegotiated_refused"],
        "local_wait_cancelled_read_only":true,"durable_cancel_original_preserved":true,"unattributed_reconciliation_refused":true,
        "canonical_http_same_original_record":true,"independent_http_receipt_agreement":true,
        "receipt_queued_without_delivery":true,"attributed_delivery":true,"read_default_withheld_then_opted_in":true,
        "attributed_read":true,"current_receipts_refused":true,"synchronous_original_recovered":true,
        "default_helper_on_path":true,"helpers_joined":10,"validators_joined":5,"http_workers_joined":2,"servers_joined":3,"python_process_joined":true,
        "physical_peers_drained":true,"media_readers_drained":true})
    );
}

#[tokio::test]
#[ignore = "Requires clean installed Python wheel consumer under .local"]
async fn installed_python_eddsa_native_recovers_after_restart_and_enforces_current_authority() {
    run("EdDSA").await;
}
#[tokio::test]
#[ignore = "Requires clean installed Python wheel consumer under .local"]
async fn installed_python_es256_native_recovers_after_restart_and_enforces_current_authority() {
    run("ES256").await;
}
