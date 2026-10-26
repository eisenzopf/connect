//! Selected local host compatibility/authentication checks do not execute a model.
use anyhow::{Context, Result};
use std::{path::Path, process::Stdio};
use tokio::process::Command;
/// Source-qualified host grammar; unknown versions require another qualification.
pub(super) const QUALIFIED_VERSION: &str = "codex-cli 0.164.0-alpha.11.1";
pub(super) async fn check(binary: &Path, deadline: tokio::time::Instant) -> Result<()> {
    let version = probe(binary, &["--version"], deadline).await?;
    anyhow::ensure!(version.status.success() && std::str::from_utf8(&version.stdout).is_ok_and(|text|text.trim()==QUALIFIED_VERSION),"Unknown Codex host version; select the source-qualified Codex runtime before running the agent");
    let auth = probe(binary, &["login", "status"], deadline).await?;
    anyhow::ensure!(auth.status.success(),"Selected model host has no usable authentication; authenticate it explicitly before running the agent");
    Ok(())
}
async fn probe(
    binary: &Path,
    args: &[&str],
    deadline: tokio::time::Instant,
) -> Result<std::process::Output> {
    let mut command = Command::new(binary);
    command
        .args(args)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    for name in [
        "PATH",
        "HOME",
        "CODEX_HOME",
        "CODEX_API_KEY",
        "LANG",
        "TMPDIR",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let budget = deadline.min(tokio::time::Instant::now() + std::time::Duration::from_secs(5));
    let output = crate::cli::task_access::codex_host::process::probe_host(&mut command, budget)
        .await
        .context("Selected Codex runtime preflight failed")?;
    Ok(output)
}
