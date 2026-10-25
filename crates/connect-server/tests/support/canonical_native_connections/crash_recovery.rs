//! Real non-unwinding child death; retained rows cannot resurrect native media.
use super::*;
use serde::{Deserialize, Serialize};
use std::{fs::File, os::unix::process::ExitStatusExt, process::Stdio};

const CHILD: &str = "canonical_native_connections::crash_recovery::native_ready_child";
const ROOT_ENV: &str = "CONNECT_G6_CRASH_ROOT";
const ALGORITHM_ENV: &str = "CONNECT_G6_CRASH_ALGORITHM";

#[derive(Serialize, Deserialize)]
struct Ready {
    config: CanonicalServerConfig,
    client: CanonicalClientConfig,
    conversation: String,
    epoch: String,
    installation: String,
}

// This is a subprocess entry point, never a skipped qualification case. Parent
// cases explicitly invoke it with --ignored and require a ready marker +SIGKILL.
#[ignore = "subprocess entry point; invoked by the crash recovery parent tests"]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_ready_child() {
    let root = std::path::PathBuf::from(std::env::var_os(ROOT_ENV).unwrap());
    let algorithm = std::env::var(ALGORITHM_ENV).unwrap();
    let mut f = Fixture::with_native_dtmf_scope(&algorithm, 12).await;
    assert!(f.config.database.starts_with(&root));
    f.server.store.configure_session_reconnect_grace(1).unwrap();
    f.config.session_reconnect_grace_seconds = 1;
    let mut scene = codecs::setup_started_dtmf(f, true).await;
    {
        let (source, target) = scene.clients.split_at_mut(1);
        codecs::decoded_audio(&source[0], &mut target[0], scene.sh, 0, scene.th, 0, 100).await;
        codecs::decoded_audio(&target[0], &mut source[0], scene.th, 0, scene.sh, 0, 200).await;
    }
    let db = rusqlite::Connection::open_with_flags(
        &scene.f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let epoch = scene.f.server.store.runtime_epoch().to_owned();
    assert_eq!(db.query_row("SELECT count(*) FROM connections WHERE id IN ('codec-source','codec-target') AND state IN ('ready','active') AND observed_runtime_id=?1 AND readiness_json IS NOT NULL",[&epoch],|r|r.get::<_,u64>(0)).unwrap(),2);
    assert_eq!(
        db.query_row(
            "SELECT state FROM sessions WHERE id='native-session'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "active"
    );
    let mut config = scene.f.config.clone();
    config.bind = scene.f.server.address;
    let ready = Ready {
        config,
        client: scene.f.client.clone(),
        conversation: scene.cid.clone(),
        epoch,
        installation: scene.f.server.store.installation_id().unwrap(),
    };
    let pending = root.join("ready.pending");
    std::fs::write(&pending, serde_json::to_vec(&ready).unwrap()).unwrap();
    std::fs::rename(pending, root.join("ready.json")).unwrap();
    // Keep the actual peers, installation lock and native engine alive. The
    // parent must kill this process; no destructor/shutdown can qualify death.
    std::future::pending::<()>().await;
    drop(scene);
}

struct Child(std::process::Child);
impl Drop for Child {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}
struct Scratch(Option<tempfile::TempDir>);
impl Scratch {
    fn path(&self) -> &Path {
        self.0.as_ref().unwrap().path()
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        if std::thread::panicking() {
            let path = self.0.take().unwrap().keep();
            eprintln!("Retained failed crash fixture: {}", path.display());
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_sigkill_native_ready_bridge_recovers_without_resurrection_eddsa() {
    native_crash("EdDSA").await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_sigkill_native_ready_bridge_recovers_without_resurrection_es256() {
    native_crash("ES256").await;
}

async fn native_crash(algorithm: &str) {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let scratch = workspace.join(".local/gate-6/recovery-process");
    std::fs::create_dir_all(&scratch).unwrap();
    let temp = Scratch(Some(tempfile::tempdir_in(scratch).unwrap()));
    let log_path = temp.path().join("child.log");
    let output = File::create(&log_path).unwrap();
    let mut child = Child(
        std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                CHILD,
                "--exact",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(ROOT_ENV, temp.path())
            .env(ALGORITHM_ENV, algorithm)
            .env("TMPDIR", temp.path())
            .stdout(Stdio::from(output.try_clone().unwrap()))
            .stderr(Stdio::from(output))
            .spawn()
            .unwrap(),
    );
    let until = tokio::time::Instant::now() + Duration::from_secs(20);
    let ready = temp.path().join("ready.json");
    while !ready.exists() {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "crash child exited before ready; log: {}",
            log_path.display()
        );
        assert!(
            tokio::time::Instant::now() < until,
            "crash child did not attest decoded readiness; log: {}",
            log_path.display()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let ready: Ready = serde_json::from_slice(&std::fs::read(ready).unwrap()).unwrap();
    child.0.kill().unwrap();
    let death = child.0.wait().unwrap();
    assert_eq!(death.signal(), Some(9), "test requires actual SIGKILL");
    let server = RunningCanonicalServer::start(ready.config.clone())
        .await
        .unwrap();
    assert_ne!(server.store.runtime_epoch(), ready.epoch);
    assert_eq!(server.store.installation_id().unwrap(), ready.installation);
    let db = rusqlite::Connection::open_with_flags(
        &ready.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let physical: Vec<(String,String,u64,Option<String>,Option<String>)> = db.prepare("SELECT id,state,generation,observed_runtime_id,readiness_json FROM connections WHERE id IN ('codec-source','codec-target') ORDER BY id").unwrap().query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    assert_eq!(physical.len(), 2);
    assert!(
        physical
            .iter()
            .all(|(_, state, generation, epoch, readiness)| matches!(
                state.as_str(),
                "retired" | "interrupted"
            ) && *generation > 1
                && epoch.is_none()
                && readiness.is_none()),
        "successor resurrected old physical readiness: {physical:?}"
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM native_connection_lifetimes WHERE retired=0",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    let until = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let state: String = db
            .query_row(
                "SELECT state FROM sessions WHERE id='native-session'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        if state == "ended" {
            break;
        }
        assert!(
            tokio::time::Instant::now() < until,
            "dead bridge remained active beyond configured reconnect grace"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let mut client_config = ready.client;
    client_config.endpoint = server.address;
    let mut client = CanonicalClient::open(client_config).await.unwrap();
    let snapshot = client
        .conversation_snapshot(
            &ready.conversation,
            serde_json::from_value(json!({})).unwrap(),
        )
        .await
        .unwrap();
    assert!(!snapshot.has_more);
    let ended = snapshot
        .items
        .iter()
        .find(|item| item["kind"] == "session" && item["data"]["id"] == "native-session")
        .unwrap();
    assert_eq!(ended["data"]["state"], "ended");
    assert_eq!(ended["data"]["ended_reason"], "reconnect-grace-expired");
    let stale = client
        .send_native_dtmf(
            CommandScope {
                conversation_id: Some(ready.conversation.clone()),
                session_id: Some("native-session".into()),
                connection_id: Some("codec-target".into()),
                operation_id: Some("after-crash-stale-native-signal".into()),
                ..Default::default()
            },
            "5",
            100,
            "info",
        )
        .await;
    assert!(
        matches!(stale, Err(ClientError::Application(_))),
        "a fresh control peer must not execute an old media Connection"
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM native_dtmf_sources", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
    let mut input = connect_client::conversations::CreateRequest::persistent(vec![]);
    input.tenant_id = Some("tnt_default".into());
    let fresh = client
        .create_conversation("after-native-crash-room", input)
        .await
        .unwrap();
    assert_ne!(fresh.id, ready.conversation);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM connections WHERE observed_runtime_id=?1",
            [server.store.runtime_epoch()],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0,
        "metadata recovery created a new physical peer"
    );
    if let Some(path) = std::env::var_os("CONNECT_G6_CRASH_CAPTURE") {
        let path = std::path::PathBuf::from(path);
        assert!(path.is_absolute() && path.starts_with(workspace.join(".local/gate-6")));
        std::fs::write(path.with_extension(format!("{algorithm}.json")),serde_json::to_vec_pretty(&json!({"algorithm":algorithm,"killed_signal":death.signal(),"decoded_duplex_before_kill":true,"new_runtime_epoch":true,"installation_preserved":true,"physical_after_restart":physical,"unretired_old_lifetimes":0,"session_ended_after_grace":true,"public_session_after_crash":ended,"stale_native_signal_refused_without_source_or_io":true,"fresh_conversation_without_physical_resurrection":true})).unwrap()).unwrap();
    }
    client.close();
    drop(db);
    server.shutdown().await.unwrap();
}
