//! Explicit Codex continuation: one protected installation maps to one session.
//! Host completion is not task completion. All communication authority remains
//! checked by Connect; saved session metadata grants no permissions.
mod install;
mod process;
mod state;
use anyhow::{Context, Result};
use fs2::FileExt;
use std::{path::Path, time::Duration};

/// Start or explicitly resume the saved exact Codex session. A missing session
/// after an interrupted launch requires repair, never `--last` or a fresh task.
/// Process fixtures qualify this adapter separately from actual model support.
pub async fn run(
    directory: &Path,
    binary: &Path,
    prompt: Option<&Path>,
    seconds: u64,
) -> Result<()> {
    anyhow::ensure!(
        (1..=300).contains(&seconds),
        "Host turn budget must be 1–300 seconds"
    );
    let directory = std::fs::canonicalize(directory)?;
    let installation = install::load(&directory)?;
    let lock_path = directory.join("host.lock");
    if !lock_path.try_exists()? {
        super::private_write(&lock_path, b"")?;
    }
    super::require_private(&lock_path)?;
    super::read_bounded(&lock_path)?;
    let lock = std::fs::OpenOptions::new().write(true).open(lock_path)?;
    lock.try_lock_exclusive()
        .context("This installation already has an active host turn")?;
    let grant = installation
        .file
        .grant
        .as_ref()
        .context("Missing installed grant")?;
    let supplied = prompt
        .map(|path| {
            super::require_private(path)?;
            let text =
                String::from_utf8(super::read_bounded(path)?).context("Prompt must be UTF-8")?;
            anyhow::ensure!(
                !text.trim().is_empty() && text.len() <= 32 * 1024,
                "Prompt must contain 1–32 KiB of text"
            );
            Ok::<_, anyhow::Error>(text)
        })
        .transpose()?;
    let mut state = if let Some(mut state) = state::State::load(&directory)? {
        anyhow::ensure!(
            state.installation == installation.digest
                && state.task_id == grant.grant.task_id
                && state.grant_id == grant.grant.id,
            "Saved host state belongs to a different installation"
        );
        anyhow::ensure!(state.session_id.is_some(), "Session ID was not saved after launch; inspect retained intent and Codex session records, never start a replacement automatically");
        if state.completed {
            state.run_id = uuid::Uuid::new_v4().to_string();
            state.prompt = supplied.unwrap_or_else(|| "Use $connect to inspect and continue the original bound task. Reconcile saved mutation IDs and pending/uncertain results before further work; read durable replies. Do not create a replacement task or conversation.".into());
            state.attempts = 0;
        } else if let Some(text) = supplied {
            anyhow::ensure!(
                text == state.prompt,
                "Interrupted intent cannot be replaced with another prompt"
            );
        }
        state.completed = false;
        state.attempts = state
            .attempts
            .checked_add(1)
            .context("Host attempt counter exhausted")?;
        state.save(&directory)?;
        state
    } else {
        let state = state::State {
            profile: "connect-codex-host/1".into(),
            installation: installation.digest,
            task_id: grant.grant.task_id.clone(),
            grant_id: grant.grant.id.clone(),
            session_id: None,
            run_id: uuid::Uuid::new_v4().to_string(),
            prompt: supplied.context("A first host turn requires a protected prompt file")?,
            completed: false,
            attempts: 1,
        };
        state.create(&directory)?;
        state
    };
    tokio::time::timeout(
        Duration::from_secs(seconds),
        process::execute(
            &directory,
            binary,
            &installation.config,
            grant.expose_secret(),
            &mut state,
        ),
    )
    .await
    .context(
        "Host turn deadline exceeded; original task/session retained for explicit recovery",
    )??;
    println!(
        "{}",
        serde_json::json!({"status":"host-turn-completed", "task_id":state.task_id, "session_id":state.session_id, "run_id":state.run_id})
    );
    Ok(())
}
