//! Real SIGKILL after durable capture, before any public replacement activation.
use super::*;
use crate::canonical_native_connections::crash_recovery::{Child, Scratch};
use serde::{Deserialize, Serialize};
use std::{fs::File, os::unix::process::ExitStatusExt, process::Stdio};

const ROOT: &str = "CONNECT_G6_REPLACEMENT_CAPTURE_CRASH_ROOT";
const CHILD: &str = "canonical_native_connections::consultation::groups::replacement::activation::crash::captured_replacement_child";

#[derive(Serialize, Deserialize)]
struct Captured {
    config: CanonicalServerConfig,
    controller: connect_client::application::ApplicationClientConfig,
    command: TransitionCommand,
    candidate: String,
    record: String,
    effects: Value,
    epoch: String,
    installation: String,
}

#[ignore = "subprocess entry point; parent requires captured movement and SIGKILL"]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn captured_replacement_child() {
    assert!(std::env::var_os(ROOT).is_some());
    super::exercise().await;
    panic!("captured child must be killed before final activation");
}

pub(crate) async fn pause_after_capture(
    f: &Fixture,
    candidate: &str,
    command: &TransitionCommand,
    credential: &str,
) {
    let Some(root) = std::env::var_os(ROOT) else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    assert!(f.config.database.starts_with(&root));
    let mut controller = canonical_http_config(f);
    controller.credential = credential.into();
    let actor = f.server.store.authenticate(credential).unwrap();
    let record = status(f, &actor.credential_id, &command.operation_id).await;
    assert_eq!(record.state, OperationState::Preparing);
    assert!(record.result.is_none());
    assert_eq!(record.effects.len(), 7);
    assert!(record
        .effects
        .iter()
        .all(|e| e.state == EffectState::Confirmed && e.attempt == 1));
    super::unpublished(f, candidate, &record.record_id);
    let mut config = f.config.clone();
    config.bind = f.server.address;
    let marker = Captured {
        config,
        controller,
        command: command.clone(),
        candidate: candidate.into(),
        record: record.record_id,
        effects: serde_json::to_value(record.effects).unwrap(),
        epoch: f.server.store.runtime_epoch().into(),
        installation: f.server.store.installation_id().unwrap(),
    };
    let pending = root.join("captured.pending");
    std::fs::write(&pending, serde_json::to_vec(&marker).unwrap()).unwrap();
    std::fs::rename(pending, root.join("captured.json")).unwrap();
    // Retain the real runtime and peers. Only the parent kills this process;
    // destructors and graceful shutdown cannot satisfy this crash case.
    std::future::pending::<()>().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_sigkill_captured_replacement_recovers_stopped_without_ready_or_replay() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let scratch = workspace.join(".local/gate-6/recovery-process");
    std::fs::create_dir_all(&scratch).unwrap();
    let temp = Scratch(Some(tempfile::tempdir_in(scratch).unwrap()));
    let log = temp.path().join("child.log");
    let output = File::create(&log).unwrap();
    let mut child = Child(
        std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                CHILD,
                "--exact",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(ROOT, temp.path())
            .env("TMPDIR", temp.path())
            .env("CONNECT_GATE6_KEEP_LISTENER_STATE", "1")
            .stdout(Stdio::from(output.try_clone().unwrap()))
            .stderr(Stdio::from(output))
            .spawn()
            .unwrap(),
    );
    let until = tokio::time::Instant::now() + Duration::from_secs(30);
    let marker = temp.path().join("captured.json");
    while !marker.exists() {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "child failed: {}",
            log.display()
        );
        assert!(
            tokio::time::Instant::now() < until,
            "capture timed out: {}",
            log.display()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let captured: Captured = serde_json::from_slice(&std::fs::read(marker).unwrap()).unwrap();
    child.0.kill().unwrap();
    assert_eq!(child.0.wait().unwrap().signal(), Some(9));
    let db = rusqlite::Connection::open(&captured.config.database).unwrap();
    let before: (String, Option<String>, u64) = db.query_row(
        "SELECT o.state,o.result_json,(SELECT count(*) FROM native_replacement_lifetimes WHERE operation_record_id=o.record_id) FROM operations o WHERE o.record_id=?1",
        [&captured.record], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    ).unwrap();
    assert_eq!(before, ("preparing".into(), None, 0));
    // Remove only the isolated fixture fault after actual process death. No
    // production operation, capture, lifetime or authority row is edited.
    db.execute_batch("DROP TRIGGER fixture_projection_block")
        .unwrap();
    drop(db);
    let server = RunningCanonicalServer::start(captured.config.clone())
        .await
        .unwrap();
    assert_ne!(server.store.runtime_epoch(), captured.epoch);
    assert_eq!(
        server.store.installation_id().unwrap(),
        captured.installation
    );
    let mut config = captured.controller;
    config.endpoint = server.http_address.unwrap();
    let actor = server.store.authenticate(&config.credential).unwrap();
    let mut client = CanonicalHttpClient::open(config).await.unwrap();
    let read = || {
        server
            .store
            .execute_operation_action(
                &actor.credential_id,
                OperationAction::Get {
                    operation_id: captured.command.operation_id.clone(),
                },
            )
            .unwrap()
    };
    let record = read();
    assert_eq!(record.state, OperationState::Completed);
    assert_eq!(
        serde_json::to_value(&record.effects).unwrap(),
        captured.effects
    );
    let result = record.result.as_ref().unwrap();
    assert_eq!(result["activation_state"], "stopped");
    assert!(result["readiness"].is_null());
    assert!(result["physical_readiness"].is_object());
    let db = rusqlite::Connection::open(&captured.config.database).unwrap();
    let retired: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM connections c JOIN native_connection_lifetimes l ON l.connection_id=c.id WHERE c.id=?1 AND l.operation_record_id=?2 AND l.generation=2 AND l.retired=1 AND c.generation>2 AND c.state='retired' AND c.observed_runtime_id IS NULL AND c.readiness_json IS NULL)",
        rusqlite::params![captured.candidate,captured.record], |r| r.get(0),
    ).unwrap();
    assert!(retired);
    for sql in [
        "SELECT count(*) FROM journal WHERE operation_record_id=?1 AND kind='connection_ready'",
        "SELECT count(*) FROM native_connection_lifetimes WHERE retired=0 AND ?1 IS NOT NULL",
    ] {
        assert_eq!(
            db.query_row(sql, [&captured.record], |r| r.get::<_, u64>(0))
                .unwrap(),
            0
        );
    }
    // Public recovery/replay must return the original result and cannot allocate
    // another physical peer or repeat any once-only replacement effect.
    let replay = client.transition(captured.command.clone()).await.unwrap();
    assert_eq!(serde_json::to_value(replay).unwrap()["result"], *result);
    assert_eq!(read(), record);
    let runtime = server.service.canonical_runtime_snapshot().unwrap();
    assert_eq!(runtime.physical_peers, 0);
    assert_eq!(runtime.connections, 0);
    assert_eq!(runtime.replacement_preparations, 0);
    drop(db);
    drop(client);
    server.shutdown().await.unwrap();
}
