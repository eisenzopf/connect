//! Own the real CLI process; no inherited provider credentials or test API hooks.
#![allow(clippy::unwrap_used, dead_code)]
#[path = "codex_child.rs"]
mod owned_runtime;
use super::native_tasks::StoppedHost;
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, BufReader, Lines},
    process::{ChildStdout, Command},
};

pub struct Process {
    runtime: owned_runtime::OwnedRuntime,
    output: Lines<BufReader<ChildStdout>>,
    pub ready: serde_json::Value,
    stdout: String,
    stderr: std::path::PathBuf,
}
/// Select the installed candidate when qualifying archives; otherwise Cargo's CLI.
pub fn binary() -> std::path::PathBuf {
    std::env::var_os("CONNECT_TEST_BINARY")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_connect").into())
}
fn command(state: &StoppedHost, input: &Path, api: &str, log: &Path, action: &str) -> Command {
    let mut command = Command::new(binary());
    command.env_clear().current_dir(&state.directory);
    for key in ["PATH", "HOME", "TMPDIR", "USER", "LANG"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
        .env("VAPI_PRIVATE_KEY", "fake-private-key")
        .env("CONNECT_VAPI_API_BASE_URL", api)
        .arg("vapi-task")
        .arg(action)
        .arg("--config")
        .arg(state.directory.join("host-owner.toml"))
        .arg("--server-config")
        .arg(state.directory.join("server.toml"))
        .arg("--input")
        .arg(input)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(std::fs::File::create(log).unwrap());
    let operator = state.directory.join("host-operator.toml");
    if operator.is_file() {
        command.arg("--operator-config").arg(operator);
    }
    command
}
impl Process {
    pub fn launch(state: &StoppedHost, input: &Path, api: &str, label: &str) -> Self {
        let stderr = state.directory.join(format!("{label}.log"));
        let mut runtime =
            owned_runtime::OwnedRuntime::spawn(&mut command(state, input, api, &stderr, "serve"))
                .unwrap();
        let output = BufReader::new(runtime.io().unwrap().stdout.take().unwrap()).lines();
        Self {
            runtime,
            output,
            ready: serde_json::Value::Null,
            stdout: String::new(),
            stderr,
        }
    }
    pub async fn start(state: &StoppedHost, input: &Path, api: &str, label: &str) -> Self {
        let mut process = Self::launch(state, input, api, label);
        tokio::time::timeout(Duration::from_secs(45), async {
            loop {
                let line = process
                    .output
                    .next_line()
                    .await
                    .unwrap()
                    .expect("host exited before ready");
                process.stdout.push_str(&line);
                process.stdout.push('\n');
                match serde_json::from_str::<serde_json::Value>(&line) {
                    Ok(value) if value["status"] == "vapi-task-ready" => process.ready = value,
                    _ => {}
                }
                if line.starts_with("ready transport=quic") {
                    break;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(process.ready["status"], "vapi-task-ready");
        process
    }
    pub async fn stop(mut self, secrets: &[&str]) {
        #[cfg(unix)]
        {
            let raw = self.runtime.io().unwrap().id().unwrap();
            assert!(raw > 1);
            let pid = rustix::process::Pid::from_raw(i32::try_from(raw).unwrap()).unwrap();
            rustix::process::kill_process(pid, rustix::process::Signal::TERM).unwrap();
        }
        #[cfg(not(unix))]
        self.runtime.stop().await.unwrap();
        let status = tokio::time::timeout(Duration::from_secs(10), self.runtime.wait())
            .await
            .unwrap()
            .unwrap();
        assert!(status.success(), "host shutdown failed");
        self.output
            .into_inner()
            .read_to_string(&mut self.stdout)
            .await
            .unwrap();
        assert!(self.stdout.contains("stopped"), "host did not drain");
        let error = std::fs::read_to_string(&self.stderr).unwrap();
        for secret in secrets.iter().copied().chain(["fake-private-key"]) {
            assert!(
                !self.stdout.contains(secret) && !error.contains(secret),
                "host exposed a secret"
            );
        }
    }
    pub async fn reconcile(
        state: &StoppedHost,
        input: &Path,
        api: &str,
        resource: &str,
        success: bool,
    ) {
        let log = state.directory.join("reconcile.log");
        let mut cmd = command(state, input, api, &log, "reconcile");
        cmd.arg("--resource").arg(resource);
        let mut runtime = owned_runtime::OwnedRuntime::spawn(&mut cmd).unwrap();
        let mut output = runtime.io().unwrap().stdout.take().unwrap();
        let status = tokio::time::timeout(Duration::from_secs(45), runtime.wait())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(status.success(), success);
        let mut text = String::new();
        output.read_to_string(&mut text).await.unwrap();
        assert_eq!(text.contains("reconciled"), success);
        assert!(!text.contains("fake-private-key"));
        assert!(!std::fs::read_to_string(log)
            .unwrap()
            .contains("fake-private-key"));
    }
    pub async fn fails(state: &StoppedHost, input: &Path, api: &str, label: &str) {
        let log = state.directory.join(format!("{label}.log"));
        let mut runtime =
            owned_runtime::OwnedRuntime::spawn(&mut command(state, input, api, &log, "serve"))
                .unwrap();
        let mut output = runtime.io().unwrap().stdout.take().unwrap();
        let status = tokio::time::timeout(Duration::from_secs(45), runtime.wait())
            .await
            .unwrap()
            .unwrap();
        assert!(!status.success());
        let mut text = String::new();
        output.read_to_string(&mut text).await.unwrap();
        assert!(!text.contains("ready transport=quic"));
        assert!(!std::fs::read_to_string(log)
            .unwrap()
            .contains("fake-private-key"));
    }
}
