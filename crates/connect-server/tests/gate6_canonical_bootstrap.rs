#![recursion_limit = "256"]
//! Installed default init/serve and public signed client, with no test enrollment.
#![cfg(unix)]
#![allow(clippy::unwrap_used)]
use connect_client::{
    canonical::{CanonicalClient, CanonicalClientConfig},
    conversations::CreateRequest,
    MessageDraft,
};
use connect_server::{
    canonical::{CanonicalServerConfig, RunningCanonicalServer},
    cli::canonical_init,
};
use std::{path::Path, time::Duration};
use tokio::io::{AsyncBufReadExt, BufReader};

async fn command(args: &[&str]) -> std::process::Output {
    tokio::process::Command::new(env!("CARGO_BIN_EXE_connect"))
        .args(args)
        .output()
        .await
        .unwrap()
}
async fn serve(path: &Path) -> (tokio::process::Child, std::net::SocketAddr) {
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_connect"))
        .args(["serve", "--config"])
        .arg(path)
        .kill_on_drop(true)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let ready = tokio::time::timeout(Duration::from_secs(10), async {
        for _ in 0..8 {
            let line = lines
                .next_line()
                .await
                .unwrap()
                .expect("default serve ready");
            assert!(line.len() < 4096);
            if line.starts_with("ready ") {
                return line;
            }
        }
        panic!("no ready line")
    })
    .await
    .unwrap();
    assert!(ready.contains("alpn=uctp/2") && ready.contains("profile=connect/2"));
    let address = ready
        .split_whitespace()
        .find_map(|v| v.strip_prefix("address="))
        .unwrap()
        .parse()
        .unwrap();
    child.stdout = Some(lines.into_inner().into_inner());
    (child, address)
}
async fn stop(child: &mut tokio::process::Child) {
    rustix::process::kill_process(
        rustix::process::Pid::from_raw(child.id().unwrap() as i32).unwrap(),
        rustix::process::Signal::TERM,
    )
    .unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(10), child.wait())
        .await
        .unwrap()
        .unwrap()
        .success());
}
fn config(path: &Path, name: &str, address: std::net::SocketAddr) -> CanonicalClientConfig {
    let mut c = CanonicalClientConfig::load(&path.join(format!("{name}.toml"))).unwrap();
    c.endpoint = address;
    c
}
#[tokio::test]
async fn default_installed_init_serve_enrolls_signed_clients_and_preserves_receipts_after_restart()
{
    // Exercise the workspace's external volume as well as the system temp volume.
    let temp = tempfile::tempdir_in(".local/gate-6").unwrap();
    let state = temp.path().join("fresh");
    let out = command(&[
        "init",
        "--directory",
        state.to_str().unwrap(),
        "--example",
        "review-loop",
        "--bind",
        "127.0.0.1:0",
    ])
    .await;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let operator = CanonicalClientConfig::load(&state.join("operator.toml")).unwrap();
    let agent = CanonicalClientConfig::load(&state.join("agent.toml")).unwrap();
    let app = CanonicalClientConfig::load(&state.join("reviews.toml")).unwrap();
    for secret in [&operator.credential, &agent.credential, &app.credential] {
        assert!(!String::from_utf8_lossy(&out.stdout).contains(secret.as_str()));
        assert!(!String::from_utf8_lossy(&out.stderr).contains(secret.as_str()));
    }
    assert_ne!(
        std::fs::read(&state.join("private-key.der")).unwrap(),
        std::fs::read(&state.join("server-signing.der")).unwrap()
    );
    assert_ne!(
        std::fs::read(&app.signing_private_key).unwrap(),
        std::fs::read(&agent.signing_private_key).unwrap()
    );
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&state).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let mut directories = vec![state.clone()];
    while let Some(path) = directories.pop() {
        for entry in std::fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let is_dir = entry.file_type().unwrap().is_dir();
            assert_eq!(
                entry.metadata().unwrap().permissions().mode() & 0o777,
                if is_dir { 0o700 } else { 0o600 }
            );
            if is_dir {
                directories.push(entry.path());
            }
        }
    }
    let (mut child, address) = serve(&state.join("server.toml")).await;
    let mut client = CanonicalClient::open(config(&state, "reviews", address))
        .await
        .unwrap();
    let mut admin = CanonicalClient::open(config(&state, "operator", address))
        .await
        .unwrap();
    admin.list_conversations(Default::default()).await.unwrap();
    admin.close();
    let mut create = CreateRequest::persistent(vec![agent.identity_id]);
    create.tenant_id = Some("tnt_default".into());
    let room = client
        .create_conversation("default-create", create.clone())
        .await
        .unwrap();
    let sent = client
        .send_message(
            "default-send",
            &room.id,
            "default_message",
            MessageDraft::text_all("one signed default message"),
        )
        .await
        .unwrap();
    client.close();
    stop(&mut child).await;
    let (mut child, address) = serve(&state.join("server.toml")).await;
    let mut client = CanonicalClient::open(config(&state, "reviews", address))
        .await
        .unwrap();
    assert_eq!(
        client
            .create_conversation("default-create", create)
            .await
            .unwrap()
            .id,
        room.id
    );
    assert_eq!(
        client
            .send_message(
                "default-send",
                &room.id,
                "default_message",
                MessageDraft::text_all("one signed default message")
            )
            .await
            .unwrap(),
        sent
    );
    client.close();
    stop(&mut child).await;
    let cfg = CanonicalServerConfig::load(&state.join("server.toml")).unwrap();
    connect_server::installation::InstallationLock::acquire(&cfg.database).unwrap();
}
#[tokio::test]
async fn generated_keys_devices_and_server_pin_are_enforced_without_implicit_enrollment() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("fresh");
    canonical_init::initialize(&state, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let server = RunningCanonicalServer::start(
        CanonicalServerConfig::load(&state.join("server.toml")).unwrap(),
    )
    .await
    .unwrap();
    let good = config(&state, "reviews", server.address);
    let other = config(&state, "agent", server.address);
    let mut wrong = good.clone();
    wrong.signing_private_key = other.signing_private_key.clone();
    assert!(CanonicalClient::open(wrong).await.is_err());
    let mut wrong = good.clone();
    wrong.device_id = other.device_id;
    assert!(CanonicalClient::open(wrong).await.is_err());
    let mut wrong = good.clone();
    wrong.server_public_key = state.join("wrong-server.pub");
    std::fs::write(
        &wrong.server_public_key,
        rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519)
            .unwrap()
            .public_key_raw(),
    )
    .unwrap();
    assert!(CanonicalClient::open(wrong).await.is_err());
    let mut client = CanonicalClient::open(good).await.unwrap();
    client.list_conversations(Default::default()).await.unwrap();
    client.close();
    server.shutdown().await.unwrap();
}
#[tokio::test]
async fn default_cli_refuses_old_configuration_old_alias_and_unmigrated_preset_before_effects() {
    let temp = tempfile::tempdir().unwrap();
    let old = temp.path().join("old");
    connect_server::cli::init::initialize(&old, false, "127.0.0.1:0".parse().unwrap()).unwrap();
    let bytes = std::fs::read(old.join("server.toml")).unwrap();
    let out = command(&[
        "serve",
        "--config",
        old.join("server.toml").to_str().unwrap(),
    ])
    .await;
    assert!(!out.status.success());
    assert_eq!(std::fs::read(old.join("server.toml")).unwrap(), bytes);
    let out = command(&[
        "serve-canonical",
        "--config",
        old.join("server.toml").to_str().unwrap(),
    ])
    .await;
    assert!(!out.status.success());
    let rejected = temp.path().join("connected");
    let out = command(&[
        "init",
        "--directory",
        rejected.to_str().unwrap(),
        "--example",
        "connected-app",
    ])
    .await;
    assert!(!out.status.success());
    assert!(!rejected.exists());
    let canonical = temp.path().join("canonical");
    canonical_init::initialize(&canonical, false, "127.0.0.1:0".parse().unwrap()).unwrap();
    let database = std::fs::read(canonical.join("state.sqlite3")).unwrap();
    let out = command(&[
        "serve",
        "--config",
        canonical.join("server.toml").to_str().unwrap(),
        "--http-bind",
        "0.0.0.0:4433",
    ])
    .await;
    assert!(!out.status.success());
    assert_eq!(
        std::fs::read(canonical.join("state.sqlite3")).unwrap(),
        database
    );
}
#[test]
fn fresh_initialization_never_replaces_existing_files_directories_or_symlinks() {
    let temp = tempfile::tempdir().unwrap();
    let address = "127.0.0.1:0".parse().unwrap();
    let file = temp.path().join("file");
    std::fs::write(&file, b"keep").unwrap();
    assert!(canonical_init::initialize(&file, false, address).is_err());
    assert_eq!(std::fs::read(&file).unwrap(), b"keep");
    let dir = temp.path().join("dir");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("keep"), b"keep").unwrap();
    assert!(canonical_init::initialize(&dir, false, address).is_err());
    assert_eq!(std::fs::read(dir.join("keep")).unwrap(), b"keep");
    let link = temp.path().join("link");
    std::os::unix::fs::symlink(&file, &link).unwrap();
    assert!(canonical_init::initialize(&link, false, address).is_err());
    assert_eq!(std::fs::read(&file).unwrap(), b"keep");
    assert!(canonical_init::initialize(
        &temp.path().join("nonlocal"),
        false,
        "0.0.0.0:4433".parse().unwrap()
    )
    .is_err());
    assert!(!temp.path().join("nonlocal").exists());
}
#[test]
fn competing_initializers_publish_exactly_one_complete_installation() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("new");
    let gate = std::sync::Arc::new(std::sync::Barrier::new(2));
    let jobs: Vec<_> = (0..2)
        .map(|_| {
            let state = state.clone();
            let gate = gate.clone();
            std::thread::spawn(move || {
                gate.wait();
                canonical_init::initialize(&state, false, "127.0.0.1:0".parse().unwrap())
            })
        })
        .collect();
    let results: Vec<_> = jobs.into_iter().map(|j| j.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    let cfg = CanonicalServerConfig::load(&state.join("server.toml")).unwrap();
    CanonicalClientConfig::load(&state.join("operator.toml")).unwrap();
    connect_store::Store::open_administration(&cfg.database, cfg.limits)
        .unwrap()
        .integrity_check()
        .unwrap();
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
}
