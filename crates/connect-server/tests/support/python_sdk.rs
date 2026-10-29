//! Installed Python wheel uses the original Rust helper and signed listener.
use super::*;
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
    let mut f = Fixture::with_native_end_scope(algorithm, 8).await;
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
            application: None,
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
    init::private_write(&spec,&serde_json::to_vec(&json!({"project":project,"config":config,
        "cid":cid,"recipient":recipient,"saved":f.temp.path().join("python-native-reference.json")})).unwrap()).unwrap();
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
            std::fs::File::create(f.temp.path().join("python-native.stderr")).unwrap(),
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
    let stopped = phase(&mut lines, "stopped").await;
    for field in [
        "same_id_replay",
        "changed_intent_refused",
        "signed_finite_journal",
        "typed_capability_refusal",
    ] {
        assert_eq!(stopped[field], true);
    }
    original(&f, &stopped, &cid);
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
    for field in ["original_reference_recovered", "same_cause_after_restart"] {
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
    ] {
        assert_eq!(completed[field], true);
    }
    drop(input);
    assert!(tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .unwrap()
        .unwrap()
        .success());
    for (path, expected) in [(&pids, 5), (&pids.with_extension("validators"), 5)] {
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
        json!({"profile":"python-native/1","algorithm":algorithm,
        "original_reference_recovered":true,"same_id_replay":true,"changed_intent_refused":true,"typed_capability_refusal":true,
        "signed_finite_journal":true,"same_cause_after_restart":true,"current_read_refused":true,
        "current_replay_refused":true,"current_new_mutation_refused":true,"independent_store_original_cause":true,
        "one_original_message_and_operation":true,"zero_forbidden_message_and_operation":true,
        "default_helper_on_path":true,"helpers_joined":5,"validators_joined":5,"servers_joined":2,"python_process_joined":true,
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
