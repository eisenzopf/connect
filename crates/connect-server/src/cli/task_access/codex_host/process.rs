//! A bounded JSONL child stream preserves exact session mapping before return.
use super::child::OwnedChild;
use crate::cli::init::private_write;
use anyhow::{Context, Result};
use serde_json::Value;
use std::{fs::File, io::Write, path::Path, process::Stdio};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::Command,
};

const MAX_EVENTS: u64 = 4 * 1024 * 1024;
const MAX_EVENT: usize = 512 * 1024;

/// Bounded metadata/authentication preflight with the same child ownership rules.
pub(crate) async fn probe_host(
    command: &mut Command,
    deadline: tokio::time::Instant,
) -> Result<std::process::Output> {
    let mut owned = OwnedChild::spawn(command)?;
    let result = tokio::time::timeout_at(deadline, async {
        let mut stdout = owned
            .child()?
            .stdout
            .take()
            .context("Missing host preflight output")?
            .take(4097);
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).await?;
        anyhow::ensure!(bytes.len() <= 4096, "Codex preflight output exceeds limit");
        let status = owned.wait().await?;
        Ok::<_, anyhow::Error>(std::process::Output {
            status,
            stdout: bytes,
            stderr: Vec::new(),
        })
    })
    .await
    .context("Codex preflight exceeded its bounded deadline")
    .and_then(|result| result);
    if result.is_err() {
        owned
            .stop_and_reap()
            .await
            .context("Codex preflight cleanup failed")?;
    }
    result
}

/// Shared process ownership and session persistence, independent of access mode.
pub(crate) trait TurnState {
    fn run_id(&self) -> &str;
    fn attempts(&self) -> u32;
    fn session_id(&self) -> Option<&str>;
    fn prompt(&self) -> &str;
    fn remember_session(&mut self, directory: &Path, id: &str) -> Result<()>;
    fn mark_completed(&mut self, directory: &Path) -> Result<()>;
}

pub(crate) async fn execute(
    directory: &Path,
    binary: &Path,
    config: &str,
    bearer_environment: &str,
    bearer: &str,
    state: &mut impl TurnState,
    deadline: tokio::time::Instant,
) -> Result<()> {
    let event_path = directory.join(format!(
        "host-{}-{}.jsonl",
        state.run_id(),
        state.attempts()
    ));
    private_write(&event_path, b"")?;
    let mut journal = std::fs::OpenOptions::new().append(true).open(event_path)?;
    let mut command = Command::new(binary);
    command.arg("exec");
    if let Some(id) = state.session_id() {
        command.args(["resume", id]);
    }
    command
        .args([
            "--json",
            "--ignore-user-config",
            "--skip-git-repo-check",
            "-c",
            config,
            "-",
        ])
        .current_dir(directory)
        .env_clear()
        .env(bearer_environment, bearer)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // Preserve the developer's model-account location, without copying its files
    // or inheriting unrelated provider/Connect operator credentials.
    for key in [
        "PATH",
        "HOME",
        "CODEX_HOME",
        "CODEX_API_KEY",
        "LANG",
        "TMPDIR",
    ] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    anyhow::ensure!(
        tokio::time::Instant::now() < deadline,
        "Host turn deadline exceeded before spawn; original intent retained"
    );
    let mut owned = OwnedChild::spawn(&mut command)?;
    let result = tokio::time::timeout_at(
        deadline,
        read_turn(&mut owned, directory, bearer, state, &mut journal),
    )
    .await
    .context("Host turn deadline exceeded; original task/session retained for explicit recovery")
    .and_then(|result| result);
    if result.is_err() {
        owned
            .stop_and_reap()
            .await
            .context("Host cleanup failed; inspect retained session before recovery")?;
    }
    result
}

async fn read_turn(
    owned: &mut OwnedChild,
    directory: &Path,
    bearer: &str,
    state: &mut impl TurnState,
    journal: &mut File,
) -> Result<()> {
    let child = owned.child()?;
    let mut stdin = child.stdin.take().context("Missing Codex stdin")?;
    stdin.write_all(state.prompt().as_bytes()).await?;
    stdin.shutdown().await?;
    // EOF requires closing the pipe, not just shutting down its writer.
    drop(stdin);
    let stdout = child.stdout.take().context("Missing Codex JSONL stream")?;
    let mut stream = BufReader::new(stdout.take(MAX_EVENTS + 1));
    let mut total = 0;
    let mut completed = false;
    let mut failed = false;
    loop {
        let mut line = Vec::new();
        let count = stream.read_until(b'\n', &mut line).await?;
        if count == 0 {
            break;
        }
        total += count;
        anyhow::ensure!(
            total <= MAX_EVENTS as usize && count <= MAX_EVENT,
            "Codex event stream exceeded its bound; retain this session for repair"
        );
        let mut event: Value =
            serde_json::from_slice(&line).context("Invalid Codex JSONL event")?;
        if event["type"] == "thread.started" {
            let id = event["thread_id"]
                .as_str()
                .context("Missing host session ID")?;
            uuid::Uuid::parse_str(id).context("Invalid host session ID")?;
            anyhow::ensure!(
                state.session_id().is_none_or(|saved| saved == id),
                "Codex returned a different session; task mapping is preserved"
            );
            state.remember_session(directory, id)?;
        }
        completed |= event["type"] == "turn.completed";
        failed |= event["type"] == "turn.failed" || event["type"] == "error";
        redact(&mut event, bearer);
        serde_json::to_writer(&mut *journal, &event)?;
        journal.write_all(b"\n")?;
        journal.sync_all()?;
    }
    let status = owned.wait().await?;
    anyhow::ensure!(
        status.success() && completed && !failed && state.session_id().is_some(),
        "Codex turn was not confirmed complete; preserve original task/session and reconcile tool results"
    );
    state.mark_completed(directory)?;
    Ok(())
}

fn redact(value: &mut Value, bearer: &str) {
    match value {
        Value::String(s) => *s = s.replace(bearer, "[REDACTED]"),
        Value::Array(values) => values.iter_mut().for_each(|v| redact(v, bearer)),
        Value::Object(values) => values.values_mut().for_each(|v| redact(v, bearer)),
        _ => {}
    }
}
