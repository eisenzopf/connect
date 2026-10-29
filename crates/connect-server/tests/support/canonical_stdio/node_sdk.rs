//! Installed typed Node package -> public Rust SDK + current canonical HTTP authority.
use super::*;
use sha2::Digest;
use std::{os::unix::fs::PermissionsExt, path::PathBuf, process::Stdio};

#[tokio::test]
#[ignore = "Requires installed Node consumer via CONNECT_GATE6_NODE_CONSUMER under .local"]
async fn installed_typed_node_sdk_recovers_typed_operations_and_current_authority() {
    let script = PathBuf::from(std::env::var_os("CONNECT_GATE6_NODE_CONSUMER").unwrap())
        .canonicalize()
        .unwrap();
    let root = std::env::current_dir().unwrap();
    assert!(script.starts_with(root.join(".local").canonicalize().unwrap()));
    let project = script.parent().unwrap();
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_extra_scopes(
            algorithm,
            8,
            true,
            true,
            true,
            &["operation.cancel", "operation.reconcile"],
        )
        .await;
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
        let sdk_config = connect_client::sdk::ClientConfig {
            control: f.client.clone(),
            tenant_id: "tnt_default".parse().unwrap(),
            application: Some(connect_client::application::ApplicationClientConfig {
                endpoint: f.server.http_address.unwrap(),
                credential: f.client.credential.clone(),
                expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
                timeout_ms: 4000,
            }),
        };
        let peer_control = crate::canonical_native_connections::party_with_scope(
            &f,
            &f.peer.identity_id,
            72,
            vec!["audio".into()],
            peer_scope,
        );
        let peer_sdk_config = connect_client::sdk::ClientConfig {
            control: peer_control.clone(),
            tenant_id: "tnt_default".parse().unwrap(),
            application: None,
        };
        let peer_native = f.temp.path().join("peer-sdk-client.toml");
        init::private_write(
            &peer_native,
            toml::to_string(&peer_sdk_config).unwrap().as_bytes(),
        )
        .unwrap();
        let native = f.temp.path().join("node-native.toml");
        let pid_file = f.temp.path().join("node-helper-pids");
        init::private_write(&native, toml::to_string(&sdk_config).unwrap().as_bytes()).unwrap();
        let wrapper = f.temp.path().join("owned-connect-wrapper");
        init::private_write(&wrapper, br#"#!/usr/bin/env python3
import os,sys,time
path=os.path.join(os.environ['CONNECT_G6_NODE_HELPER_DIR'],'helper-'+os.environ['CONNECT_G6_NODE_ALGORITHM']+'-'+str(os.getpid())+'.stderr')
fd=os.open(path,os.O_CREAT|os.O_EXCL|os.O_WRONLY,0o600)
os.dup2(fd,2)
os.close(fd)
os.write(2,('owned-exec monotonic_ns='+str(time.monotonic_ns())+' pid='+str(os.getpid())+'\n').encode())
pid_file=os.environ['CONNECT_G6_NODE_PID_FILE']+('.validators' if sys.argv[1:]==['sdk-validate-envelope'] else '')
with open(pid_file,'a') as stream: stream.write(str(os.getpid())+'\n')
os.execv(os.environ['CONNECT_G6_NODE_CONNECT'],[os.environ['CONNECT_G6_NODE_CONNECT'],*sys.argv[1:]])
"#).unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
        let input = f.temp.path().join("node-spec.json");
        init::private_write(
            &input,
            &serde_json::to_vec(&json!({
                "project":project,"config":native,"executable":wrapper,"pid_file":pid_file,
                "peer_identity":f.peer.identity_id,"peer_config":peer_native,"http":format!("http://{}",f.server.http_address.unwrap()),
                "credential":f.client.credential,"algorithm":algorithm
            }))
            .unwrap(),
        )
        .unwrap();
        let stderr_path = project.join(format!("native-server-{algorithm}.stderr"));
        let mut child = tokio::process::Command::new("node")
            .arg(&script)
            .arg(&input)
            .env("CONNECT_G6_NODE_PID_FILE", &pid_file)
            .env("CONNECT_G6_NODE_HELPER_DIR", project)
            .env("CONNECT_G6_NODE_ALGORITHM", algorithm)
            .env("CONNECT_G6_NODE_CONNECT", env!("CARGO_BIN_EXE_connect"))
            .current_dir(project)
            .kill_on_drop(true)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(std::fs::File::create(&stderr_path).unwrap()))
            .spawn()
            .unwrap();
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        let mut stdin = child.stdin.take().unwrap();
        async fn phase(
            lines: &mut Lines<BufReader<ChildStdout>>,
            expected: &str,
            algorithm: &str,
        ) -> Value {
            let started = tokio::time::Instant::now();
            let until = tokio::time::Instant::now() + Duration::from_secs(15);
            let mut checkpoint = "create".to_owned();
            loop {
                let line = match tokio::time::timeout_at(until, lines.next_line()).await {
                    Ok(line) => line.unwrap().expect("Node phase result"),
                    Err(_) => {
                        eprintln!(
                            "installed-typed-node-sdk-failure {}",
                            json!({"phase":"failed","checkpoint":checkpoint,"kind":"deadline","certainty":null,"command":null,"observedState":null,"observedKind":null,"observedCertainty":null,"observedHttpStatus":null,"line":null,"column":null})
                        );
                        panic!("Original Node phase deadline elapsed");
                    }
                };
                assert!(line.len() < 4096);
                let value: Value = serde_json::from_str(&line).unwrap();
                if value["phase"] == "progress" {
                    checkpoint = value["checkpoint"].as_str().unwrap().to_owned();
                    continue;
                }
                if value["phase"] == "failed" {
                    eprintln!("installed-typed-node-sdk-failure {}", value);
                }
                assert_eq!(value["phase"], expected);
                eprintln!(
                    "installed-typed-node-sdk-phase {}",
                    json!({"algorithm":algorithm,"phase":expected,"passed":true,"seconds":started.elapsed().as_secs_f64(),"deadline_seconds":15})
                );
                return value;
            }
        }
        // These are separate bounded functional scenarios. The installed
        // journey grew beyond the original single Session-only phase; preserve
        // every 4s SDK request and 15s absolute scenario deadline. The source
        // runner independently retains its 90s whole-function limit.
        let controls = phase(&mut lines, "controls-ready", algorithm).await;
        assert_eq!(controls["session_participants_accepted"], true);
        for field in [
            "incoming_original_hint",
            "incoming_session_decision",
            "incoming_close_control_live",
        ] {
            assert_eq!(controls[field], true);
        }
        let database = rusqlite::Connection::open_with_flags(
            &f.config.database,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let source: String = database.query_row(
            "SELECT id FROM session_invitation_sources WHERE session_id='typed-node-solo' AND operation_id='typed-session-extend'", [], |row|row.get(0)
        ).unwrap();
        let count: u64 = database.query_row(
            "SELECT COUNT(*) FROM session_invitation_sources WHERE session_id='typed-node-solo' AND operation_id='typed-session-extend'", [], |row|row.get(0)
        ).unwrap();
        assert_eq!(count, 1);
        assert_ne!(source, "typed-session-extend");
        assert_eq!(
            controls["incoming_source_sha256"],
            hex::encode(sha2::Sha256::digest(source.as_bytes()))
        );
        drop(database);
        stdin.write_all(b"native\n").await.unwrap();
        let native_phase = phase(&mut lines, "native-complete", algorithm).await;
        assert_eq!(native_phase["native_controls_completed"], true);
        stdin.write_all(b"finish\n").await.unwrap();
        let initial = phase(&mut lines, "initial", algorithm).await;
        assert_eq!(
            std::fs::metadata(&native).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::write(&native, toml::to_string(&sdk_config).unwrap()).unwrap();
        stdin.write_all(b"restart\n").await.unwrap();
        let recovered = phase(&mut lines, "recovered", algorithm).await;
        assert_eq!(initial["record_id"], recovered["record_id"]);
        // Full causal metadata is an independent Store witness, not an
        // excluded legacy application operation or an invented canonical field.
        let actor = f.server.store.authenticate(&f.client.credential).unwrap();
        let durable = f
            .server
            .store
            .execute_operation_action(
                &actor.credential_id,
                connect_protocol::transitions::OperationAction::Get {
                    operation_id: "typed-node-original-send".into(),
                },
            )
            .unwrap();
        assert_eq!(initial["record_id"], durable.record_id);
        assert_eq!(durable.operation_id, "typed-node-original-send");
        assert_eq!(durable.command_type, "send");
        assert_eq!(
            durable.state,
            connect_protocol::transitions::OperationState::Completed
        );
        assert_eq!(durable.conversation_id.as_deref(), initial["cid"].as_str());
        assert!(durable.causal_event_ids.iter().any(
            |id| initial["event_id_sha256"] == hex::encode(sha2::Sha256::digest(id.as_bytes()))
        ));
        for field in ["events_original_record", "events_cursor_persisted"] {
            assert_eq!(initial[field], true);
        }
        for field in [
            "events_resume_original_record",
            "events_resume_http_record",
            "events_resume_new_fact",
        ] {
            assert_eq!(recovered[field], true);
        }
        assert_eq!(
            std::fs::metadata(format!("{}.event-cursor", pid_file.display()))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        // Report through the independent recipient's public application client.
        // Message acceptance alone must not synthesize delivery evidence.
        let mut recipient = connect_client::application::ApplicationClient::open(
            connect_client::application::ApplicationClientConfig {
                endpoint: f.server.http_address.unwrap(),
                credential: peer_control.credential.clone(),
                expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
                timeout_ms: 4000,
            },
        )
        .await
        .unwrap();
        recipient
            .execute(
                connect_client::application::ApplicationOperation::try_from(
                    connect_protocol::Operation::MessageReceipt {
                        request_id: "typed-node-recipient-delivered".into(),
                        conversation_id: initial["cid"].as_str().unwrap().into(),
                        message_id: initial["message_id"].as_str().unwrap().into(),
                        state: connect_protocol::receipts::ClientReceiptState::Delivered,
                    },
                )
                .unwrap(),
            )
            .await
            .unwrap();
        drop(recipient);
        stdin.write_all(b"receipts\n").await.unwrap();
        let receipts = phase(&mut lines, "receipts-observed", algorithm).await;
        assert_eq!(receipts["current_delivery_fact"], true);
        assert_eq!(receipts["http_same_delivery_fact"], true);
        for field in [
            "raw_signed_original_record",
            "raw_same_id_one_message",
            "raw_local_configuration_independent",
        ] {
            assert_eq!(receipts[field], true);
        }
        let principal = f.server.store.authenticate(&f.client.credential).unwrap();
        let admin = f.server.store.authenticate(&f.admin).unwrap();
        let store = f.server.store.clone();
        tokio::task::spawn_blocking(move || store.revoke(&admin, &principal.credential_id))
            .await
            .unwrap()
            .unwrap();
        stdin.write_all(b"revoked\n").await.unwrap();
        let completed = phase(&mut lines, "completed", algorithm).await;
        assert_eq!(completed["raw_current_authority"], true);
        assert_eq!(completed["http_receipts_current_authority"], true);
        assert!(matches!(
            completed["http_receipts_status"].as_u64(),
            Some(401 | 403)
        ));
        let database = rusqlite::Connection::open_with_flags(
            &f.config.database,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        assert_eq!(
            database
                .query_row(
                    "SELECT COUNT(*) FROM messages WHERE id='typed-node-original-message'",
                    [],
                    |row| row.get::<_, u64>(0)
                )
                .unwrap(),
            1
        );
        assert_eq!(
            database
                .query_row(
                    "SELECT COUNT(*) FROM operations WHERE operation_id='typed-node-original-send'",
                    [],
                    |row| row.get::<_, u64>(0)
                )
                .unwrap(),
            1
        );
        // Ordinary message acceptance journals a Message fact. Its original
        // logical ID and sender bind it to the same completed control receipt;
        // no operation_updated event is invented for this atomic mutation.
        let (count, event_id): (u64, String) = database.query_row("SELECT COUNT(*),MIN(j.id) FROM visible_journal j JOIN operations o ON o.record_id=?1 WHERE j.recipient=o.owner_id AND j.conversation_id=o.conversation_id AND j.kind='message' AND j.message_id='typed-node-original-message' AND json_extract(j.data_json,'$.operation_id')=o.operation_id AND o.operation_id='typed-node-original-send' AND o.command_type='send' AND o.state='completed'", [initial["record_id"].as_str().unwrap()], |row|Ok((row.get(0)?,row.get(1)?))).unwrap();
        assert_eq!(count, 1);
        assert_eq!(
            initial["event_id_sha256"],
            hex::encode(sha2::Sha256::digest(event_id.as_bytes()))
        );
        for (table, field, id) in [
            ("messages", "id", "typed-node-events-resume-message"),
            (
                "operations",
                "operation_id",
                "typed-node-events-resume-send",
            ),
        ] {
            let count: u64 = database
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE {field}=?1"),
                    [id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1);
        }
        assert_eq!(completed["events_current_authority_no_data"], true);
        assert_eq!(completed["events_close_control_live"], true);
        drop(database);
        assert_eq!(completed["local_validators_joined"], 3);
        assert_eq!(completed["helpers_joined"], 4);
        drop(stdin);
        assert!(
            tokio::time::timeout(Duration::from_secs(5), lines.next_line())
                .await
                .unwrap()
                .unwrap()
                .is_none()
        );
        let status = tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .unwrap()
            .unwrap();
        assert!(
            status.success(),
            "Node worker failed; inspect private stderr"
        );
        assert!(!std::fs::read_to_string(&stderr_path)
            .unwrap()
            .contains(&f.client.credential));
        assert!(!std::fs::read_to_string(&stderr_path)
            .unwrap()
            .contains(&peer_control.credential));
        for entry in std::fs::read_dir(project).unwrap() {
            let entry = entry.unwrap();
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with(&format!("helper-{algorithm}-"))
            {
                assert_eq!(
                    entry.metadata().unwrap().permissions().mode() & 0o777,
                    0o600
                );
                assert!(!std::fs::read_to_string(entry.path())
                    .unwrap()
                    .contains(&f.client.credential));
                assert!(!std::fs::read_to_string(entry.path())
                    .unwrap()
                    .contains(&peer_control.credential));
            }
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            while {
                let observed = f.server.service.canonical_runtime_snapshot().unwrap();
                observed.physical_peers != 0 || observed.media_readers != 0
            } {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        f.server.shutdown().await.unwrap();
        eprintln!(
            "installed-typed-node-sdk-witness {}",
            json!({"functional_phase_profile":"node-controls/7","controls":controls,"native":native_phase,"initial":initial,"recovered":recovered,"receipts":receipts,"completed":completed,"server_shutdown_joined":true,"physical_peers_drained":true,"media_readers_drained":true})
        );
    }
}
