//! Selected external server; fixture setup and participation use public interfaces.
#![allow(clippy::unwrap_used)]
use connect_client::{ClientConfig, ConnectClient};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::Read,
    net::UdpSocket,
    ops::{Deref, DerefMut},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

fn digest(path: &Path) -> String {
    // The macOS system hasher avoids the unoptimized test profile's software
    // compression loop for repeated 240 MiB binary checks. This is a fixture
    // tool, never a server/client runtime dependency or weaker integrity check.
    let mut command = Command::new("/usr/bin/shasum");
    environment(&mut command);
    let output = command.args(["-a", "256"]).arg(path).output().unwrap();
    assert!(output.status.success() && output.stdout.len() <= 8192);
    let text = std::str::from_utf8(&output.stdout).unwrap();
    let value = text.split_ascii_whitespace().next().unwrap();
    assert!(value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()));
    value.to_owned()
}

fn environment(command: &mut Command) {
    command.env_clear();
    for key in ["PATH", "HOME", "USER", "LANG", "TMPDIR"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command.env("RUST_LOG", "warn");
}

fn private_file(path: &Path) -> File {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .unwrap()
}

/// A child remains owned even when a readiness assertion or test panics.
pub struct Server {
    child: Option<Child>,
    binary: PathBuf,
    binary_sha256: String,
    directory: PathBuf,
    starts: usize,
    startup_seconds: Vec<f64>,
    shutdown_seconds: Vec<f64>,
}
impl Server {
    pub async fn start(&mut self) {
        assert!(self.child.is_none());
        assert_eq!(digest(&self.binary), self.binary_sha256);
        let log_path = self.directory.join(format!("server-{}.log", self.starts));
        self.starts += 1;
        let log = private_file(&log_path);
        let mut command = Command::new(&self.binary);
        environment(&mut command);
        let began = Instant::now();
        self.child = Some(
            command
                .args(["serve", "--config"])
                .arg(self.directory.join("state/server.toml"))
                .stdin(Stdio::null())
                .stdout(log.try_clone().unwrap())
                .stderr(log)
                .spawn()
                .unwrap(),
        );
        loop {
            assert!(
                self.child.as_mut().unwrap().try_wait().unwrap().is_none(),
                "Selected server exited before readiness; private log retained"
            );
            let mut output = String::new();
            File::open(&log_path)
                .unwrap()
                .take(65536)
                .read_to_string(&mut output)
                .unwrap();
            if output.contains("ready transport=quic") {
                break;
            }
            assert!(
                began.elapsed() < Duration::from_secs(5),
                "Installed server startup exceeded five seconds"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        self.startup_seconds.push(began.elapsed().as_secs_f64());
        assert_eq!(digest(&self.binary), self.binary_sha256);
    }

    pub async fn stop(&mut self) {
        let Some(child) = self.child.as_mut() else {
            return;
        };
        let began = Instant::now();
        if child.try_wait().unwrap().is_none() {
            let pid = rustix::process::Pid::from_raw(i32::try_from(child.id()).unwrap()).unwrap();
            rustix::process::kill_process(pid, rustix::process::Signal::TERM).unwrap();
        }
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(
                    status.success(),
                    "Selected installed server failed shutdown; private log retained"
                );
                break;
            }
            assert!(
                began.elapsed() < Duration::from_secs(5),
                "Installed server shutdown exceeded five seconds"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        self.child = None;
        self.shutdown_seconds.push(began.elapsed().as_secs_f64());
        assert_eq!(digest(&self.binary), self.binary_sha256);
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Own optional client connections so explicit physical loss does not partially
/// move the fixture or prevent its remaining server/peer cleanup.
pub struct ClientSlot(Option<ConnectClient>);
impl ClientSlot {
    pub async fn close(&mut self) {
        if let Some(client) = self.0.take() {
            client.close().await;
        }
    }
}
impl Deref for ClientSlot {
    type Target = ConnectClient;
    fn deref(&self) -> &Self::Target {
        self.0.as_ref().expect("Client connection already closed")
    }
}
impl DerefMut for ClientSlot {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.as_mut().expect("Client connection already closed")
    }
}

pub struct Fixture {
    pub server: Server,
    pub left: ClientSlot,
    pub right: ClientSlot,
    pub conversation: String,
    pub session: String,
    pub left_id: String,
    pub right_id: String,
    left_credential_id: String,
    source_revision: String,
    cli_calls: usize,
}
impl Fixture {
    pub async fn new() -> Self {
        // No CARGO_BIN_EXE, embedded server or ambient target fallback.
        let binary = PathBuf::from(
            std::env::var_os("CONNECT_TEST_BINARY")
                .expect("Select CONNECT_TEST_BINARY explicitly for this external-runtime proof"),
        )
        .canonicalize()
        .unwrap();
        let source_revision = std::env::var("CONNECT_SOURCE_REVISION")
            .expect("Bind the qualification SDK/source with CONNECT_SOURCE_REVISION");
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let work = root.join(".local/gate5/installed-video-failures");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::set_permissions(&work, std::fs::Permissions::from_mode(0o700)).unwrap();
        // Preserve successful and failed private inputs/logs as distinct attempts.
        // Process cleanup stays RAII-owned independently of retained disk evidence.
        let directory = tempfile::tempdir_in(work).unwrap().keep();
        let mut server = Server {
            binary_sha256: digest(&binary),
            binary,
            directory,
            child: None,
            starts: 0,
            startup_seconds: vec![],
            shutdown_seconds: vec![],
        };
        let address = UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let state = server.directory.join("state");
        let mut cli_calls = 0;
        cli(
            &server,
            &mut cli_calls,
            &[
                "init".into(),
                "--directory".into(),
                state.clone().into_os_string(),
                "--example".into(),
                "review-loop".into(),
                "--bind".into(),
                address.to_string().into(),
            ],
        )
        .await;
        let left = ClientConfig::load(&state.join("reviews.toml")).unwrap();
        let right = ClientConfig::load(&state.join("agent.toml")).unwrap();
        let left_id = left.identity_id;
        let right_id = right.identity_id;
        for (name, identity, peer) in [
            ("reviews", &left_id, &right_id),
            ("agent", &right_id, &left_id),
        ] {
            cli(
                &server,
                &mut cli_calls,
                &[
                    "issue".into(),
                    "--config".into(),
                    state.join("operator.toml").into_os_string(),
                    "--server-config".into(),
                    state.join("server.toml").into_os_string(),
                    "--identity-id".into(),
                    identity.as_str().into(),
                    "--recipients".into(),
                    peer.as_str().into(),
                    "--voice".into(),
                    "--output".into(),
                    state.join(format!("{name}-av.toml")).into_os_string(),
                ],
            )
            .await;
        }
        let left_config = ClientConfig::load(&state.join("reviews-av.toml")).unwrap();
        let right_config = ClientConfig::load(&state.join("agent-av.toml")).unwrap();
        let left_credential_id = left_config.credential_id.clone();
        server.start().await;
        let mut left = open(left_config).await;
        let mut right = open(right_config).await;
        let conversation = left
            .create("installed-video-create", vec![right_id.clone()])
            .await
            .unwrap()
            .id;
        let session = left
            .session_invite(
                "installed-video-invite",
                &conversation,
                &right_id,
                vec!["audio".into(), "video".into()],
            )
            .await
            .unwrap()
            .id;
        right
            .session_accept("installed-video-accept", &conversation, &session)
            .await
            .unwrap();
        Self {
            server,
            left: ClientSlot(Some(left)),
            right: ClientSlot(Some(right)),
            conversation,
            session,
            left_id,
            right_id,
            left_credential_id,
            source_revision,
            cli_calls,
        }
    }

    pub async fn revoke_left(&mut self) {
        let state = self.server.directory.join("state");
        cli(
            &self.server,
            &mut self.cli_calls,
            &[
                "revoke".into(),
                "--config".into(),
                state.join("operator.toml").into_os_string(),
                "--server-config".into(),
                state.join("server.toml").into_os_string(),
                self.left_credential_id.clone().into(),
            ],
        )
        .await;
    }

    pub async fn open_right(&self) -> ConnectClient {
        open(ClientConfig::load(&self.server.directory.join("state/agent-av.toml")).unwrap()).await
    }

    pub async fn finish(mut self, case: &str) {
        self.left.close().await;
        self.right.close().await;
        self.server.stop().await;
        println!(
            "INSTALLED_VIDEO_FAILURE_RESULT {}",
            serde_json::json!({
                "case":case,"status":"passed","selected_server_binary":self.server.binary,
                "selected_server_sha256":self.server.binary_sha256,"source_revision":self.source_revision,
                "scope":"public SDK compiled from qualification source plus explicitly selected external server/CLI; no independent decoded-media or installed-SDK-binary claim",
                "hash_tool":"/usr/bin/shasum -a256 (macOS system fixture tool)",
            "fixture_source_sha256":hex::encode(Sha256::digest(include_bytes!("installed_video.rs"))),
                "test_source_sha256":hex::encode(Sha256::digest(include_bytes!("../gate5_installed_video_failures.rs"))),
                "public_cli_calls":self.cli_calls,"startup_seconds":self.server.startup_seconds,
                "shutdown_seconds":self.server.shutdown_seconds,"lifecycle_limit_seconds":5,
                "binary_rechecked":true
            })
        );
    }
}

async fn open(config: ClientConfig) -> ConnectClient {
    tokio::time::timeout(Duration::from_secs(5), ConnectClient::open(config))
        .await
        .unwrap()
        .unwrap()
}

async fn cli(server: &Server, calls: &mut usize, args: &[std::ffi::OsString]) {
    assert_eq!(digest(&server.binary), server.binary_sha256);
    let log = private_file(&server.directory.join(format!("cli-{calls}.log")));
    let mut command = Command::new(&server.binary);
    environment(&mut command);
    let mut command = tokio::process::Command::from(command);
    let child = command
        .args(args)
        .stdin(Stdio::null())
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let status = tokio::time::timeout(Duration::from_secs(5), child.wait_with_output())
        .await
        .unwrap()
        .unwrap()
        .status;
    assert!(
        status.success(),
        "Selected public CLI failed; private log retained"
    );
    *calls += 1;
    assert_eq!(digest(&server.binary), server.binary_sha256);
}
