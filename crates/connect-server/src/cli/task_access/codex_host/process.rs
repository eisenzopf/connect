//! A bounded JSONL child stream preserves exact session mapping before return.
use super::state::State;
use crate::cli::init::private_write;
use anyhow::{Context, Result};
use serde_json::Value;
use std::{io::Write, path::Path, process::Stdio};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::Command,
};

const MAX_EVENTS: u64 = 4 * 1024 * 1024;
const MAX_EVENT: usize = 512 * 1024;

pub(super) async fn execute(
    directory: &Path,
    binary: &Path,
    config: &str,
    bearer: &str,
    state: &mut State,
) -> Result<()> {
    let event_path = directory.join(format!("host-{}-{}.jsonl", state.run_id, state.attempts));
    private_write(&event_path, b"")?;
    let mut journal = std::fs::OpenOptions::new().append(true).open(event_path)?;
    let mut command = Command::new(binary);
    command.arg("exec");
    if let Some(id) = &state.session_id {
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
        .env("CONNECT_TASK_GRANT", bearer)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
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
    let mut child = command
        .spawn()
        .context("Codex could not start; retained intent needs inspection")?;
    let result = async {
        let mut stdin = child.stdin.take().context("Missing Codex stdin")?;
        stdin.write_all(state.prompt.as_bytes()).await?;
        stdin.shutdown().await?;
        // Closing the pipe is what gives the child EOF; shutdown alone does not
        // close a Unix process stdin while this handle remains in scope.
        drop(stdin);
        let stdout = child.stdout.take().context("Missing Codex JSONL stream")?;
        let mut stream = BufReader::new(stdout.take(MAX_EVENTS + 1));
        let mut total = 0;
        let mut completed = false;
        let mut failed = false;
        loop {
            let mut line = Vec::new();
            let count = stream.read_until(b'\n', &mut line).await?;
            if count == 0 { break; }
            total += count;
            anyhow::ensure!(total <= MAX_EVENTS as usize && count <= MAX_EVENT, "Codex event stream exceeded its bound; retain this session for repair");
            let mut event: Value = serde_json::from_slice(&line).context("Invalid Codex JSONL event")?;
            if event["type"] == "thread.started" {
                let id = event["thread_id"].as_str().context("Missing host session ID")?;
                uuid::Uuid::parse_str(id).context("Invalid host session ID")?;
                anyhow::ensure!(state.session_id.as_deref().is_none_or(|saved| saved == id), "Codex returned a different session; task mapping is preserved");
                state.session_id = Some(id.to_owned());
                state.save(directory)?;
            }
            if event["type"] == "turn.completed" { completed = true; }
            if event["type"] == "turn.failed" || event["type"] == "error" { failed = true; }
            redact(&mut event, bearer);
            serde_json::to_writer(&mut journal, &event)?;
            journal.write_all(b"\n")?;
            journal.sync_all()?;
        }
        let status = child.wait().await?;
        anyhow::ensure!(status.success() && completed && !failed && state.session_id.is_some(), "Codex turn was not confirmed complete; preserve original task/session and reconcile tool results");
        state.completed = true;
        state.save(directory)?;
        Ok(())
    }.await;
    if result.is_err() {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    result
}
fn redact(value: &mut Value, bearer: &str) {
    match value {
        Value::String(s) => *s = s.replace(bearer, "[REDACTED]"),
        Value::Array(values) => values.iter_mut().for_each(|v| redact(v, bearer)),
        Value::Object(values) => values.values_mut().for_each(|v| redact(v, bearer)),
        _ => {}
    }
}
