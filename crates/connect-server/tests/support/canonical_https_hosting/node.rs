//! Installed Node finite HTTPS control, with independent signed native evidence.
use super::*;
use connect_client::sdk;
use serde_json::Value;
use std::{path::PathBuf, process::Stdio};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
async fn run(algorithm: &str) {
    let script = PathBuf::from(std::env::var_os("CONNECT_GATE6_NODE_HTTPS_CONSUMER").unwrap())
        .canonicalize()
        .unwrap();
    assert!(script.starts_with(
        std::env::current_dir()
            .unwrap()
            .join(".local")
            .canonicalize()
            .unwrap()
    ));
    let h = Hosted::configured_algorithm(algorithm, vec![], 8, |_| {}).await;
    let options = || sdk::RequestOptions::new(Duration::from_secs(4)).unwrap();
    let mutation = |id: &str| sdk::MutationOptions {
        operation_id: id.parse().unwrap(),
        request: options(),
        extensions: None,
    };
    let native = sdk::Client::connect(h.f.client.clone(), "tnt_default".parse().unwrap())
        .await
        .unwrap();
    let room = native
        .create_conversation(
            vec![h.f.peer.identity_id.parse().unwrap()],
            sdk::ConversationPolicy::Persistent,
            mutation("node-https-room"),
        )
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap()
        .value;
    let snapshot = room.refresh(options()).await.unwrap();
    let recipient = snapshot
        .participants
        .iter()
        .find(|p| p.identity_id.as_str() == h.f.peer.identity_id)
        .unwrap();
    let spec = h.f.temp.path().join("node-https-spec.json");
    connect_server::cli::init::private_write(&spec,serde_json::to_vec(&json!({"project":script.parent().unwrap(),"endpoint":h.endpoint(HttpSurface::Application).origin,"credential":h.f.client.credential,"cid":room.id(),"recipient":recipient.id})).unwrap().as_slice()).unwrap();
    let mut child = tokio::process::Command::new(
        std::env::var_os("CONNECT_GATE6_NODE").unwrap_or_else(|| "node".into()),
    )
    .arg(&script)
    .arg(&spec)
    .env("NODE_EXTRA_CA_CERTS", &h.tls.root)
    .env_remove("NODE_TLS_REJECT_UNAUTHORIZED")
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .kill_on_drop(true)
    .spawn()
    .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    async fn phase(
        lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
        name: &str,
    ) -> Value {
        let until = tokio::time::Instant::now() + Duration::from_secs(15);
        let mut checkpoint = "https-snapshot".to_owned();
        loop {
            let line = match tokio::time::timeout_at(until, lines.next_line()).await {
                Ok(line) => line.unwrap().expect("Node HTTPS phase result"),
                Err(_) => {
                    eprintln!(
                        "installed-typed-node-sdk-failure {}",
                        json!({"phase":"failed","checkpoint":checkpoint,"kind":"deadline","certainty":null,"command":null,"observedState":null,"observedKind":null,"observedCertainty":null,"observedHttpStatus":null,"line":null,"column":null})
                    );
                    panic!("Original Node HTTPS phase deadline elapsed");
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
            assert_eq!(value["phase"], name);
            return value;
        }
    }
    let initial = phase(&mut lines, "initial").await;
    for field in [
        "current_https_journal",
        "original_replay_record",
        "application_receipts_empty",
    ] {
        assert_eq!(initial[field], true);
    }
    let saved = json!({"schema":"connect-operation-reference/1","operation_id":"node-https-original-send","command":"message.send","scope":{"conversation_id":room.id()}});
    let original = native
        .restore(saved, native.operation_kinds().message_send)
        .unwrap()
        .wait(options())
        .await
        .unwrap();
    assert_eq!(
        initial["record_id"],
        original.snapshot.record_id.as_ref().unwrap().as_str()
    );
    assert_eq!(
        original.value.message_id.as_str(),
        "node-https-original-message"
    );
    let actor =
        h.f.server
            .store
            .authenticate(&h.f.client.credential)
            .unwrap();
    let administrator = h.f.server.store.authenticate(&h.f.admin).unwrap();
    let store = h.f.server.store.clone();
    tokio::task::spawn_blocking(move || store.revoke(&administrator, &actor.credential_id))
        .await
        .unwrap()
        .unwrap();
    stdin.write_all(b"revoked\n").await.unwrap();
    let completed = phase(&mut lines, "completed").await;
    for field in [
        "current_https_read_refused",
        "current_https_replay_refused",
        "current_https_new_mutation_refused",
        "current_https_receipts_refused",
    ] {
        assert_eq!(completed[field], true);
    }
    drop(stdin);
    assert!(tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .unwrap()
        .unwrap()
        .success());
    let database = rusqlite::Connection::open_with_flags(
        &h.f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    for (table, key, id, expected) in [
        ("messages", "id", "node-https-original-message", 1),
        ("operations", "operation_id", "node-https-original-send", 1),
        ("messages", "id", "node-https-forbidden-message", 0),
        ("operations", "operation_id", "node-https-forbidden-send", 0),
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
    drop(database);
    native.close(options()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let current = h.f.server.service.canonical_runtime_snapshot().unwrap();
            if current.physical_peers == 0 && current.media_readers == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    h.f.server.shutdown().await.unwrap();
    eprintln!(
        "installed-node-https-witness {}",
        json!({"profile":"node-https/1","algorithm":algorithm,"initial":initial,"completed":completed,"independent_native_original_record":true,"one_original_message_and_operation":true,"zero_forbidden_message_and_operation":true,"node_process_joined":true,"helper_processes":0,"servers_joined":2,"physical_peers_drained":true,"media_readers_drained":true})
    );
}
#[tokio::test]
#[ignore = "Requires independently installed Node HTTPS consumer under .local"]
async fn installed_node_eddsa_https_agrees_with_native_and_enforces_current_authority() {
    run("EdDSA").await;
}
#[tokio::test]
#[ignore = "Requires independently installed Node HTTPS consumer under .local"]
async fn installed_node_es256_https_agrees_with_native_and_enforces_current_authority() {
    run("ES256").await;
}
