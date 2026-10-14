//! Reuse the qualified bounded Codex process lifecycle with regular agent access.
use super::{codex, state::State};
use crate::cli::task_access::{codex_host, read_bounded, require_private};
use anyhow::{Context, Result};
use std::{path::Path, time::Duration};

/// Run or explicitly resume an existing regular-agent installation. No grant is
/// minted or refreshed, and task IDs remain optional work-tracking references.
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
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    let directory = std::fs::canonicalize(directory)?;
    codex::load(&directory)?;
    let _lock = codex_host::lock(&directory)?;
    let installation = codex::load(&directory)?;
    super::preflight::check(binary, deadline).await?;
    anyhow::ensure!(
        installation.credential_expires_at > chrono::Utc::now().timestamp(),
        "Installed agent credential expired; retain its original session for owner repair"
    );
    let supplied = prompt
        .map(|path| {
            require_private(path)?;
            let text = String::from_utf8(read_bounded(path)?).context("Prompt must be UTF-8")?;
            anyhow::ensure!(
                !text.trim().is_empty() && text.len() <= 32 * 1024,
                "Prompt must contain 1–32 KiB of text"
            );
            Ok::<_, anyhow::Error>(text)
        })
        .transpose()?;
    let mut state = if let Some(mut saved) = State::load(&directory)? {
        anyhow::ensure!(
            saved.installation == installation.digest
                && saved.identity_id == installation.client.identity_id
                && saved.credential_id == installation.client.credential_id,
            "Saved host state belongs to another agent installation"
        );
        anyhow::ensure!(saved.session_id.is_some(), "Session ID was not saved after launch; inspect retained intent and Codex session records, never start a replacement automatically");
        if saved.completed {
            saved.run_id = uuid::Uuid::new_v4().to_string();
            saved.prompt=supplied.unwrap_or_else(||"Use $connect to inspect and continue existing communication and tracked work. Recall the authorized conversation context and journal; reconcile original request IDs and uncertain results before further action. Preserve the original conversations and task references.".into());
            saved.attempts = 0;
        } else if let Some(text) = supplied {
            anyhow::ensure!(
                text == saved.prompt,
                "Interrupted agent intent cannot be replaced with another prompt"
            );
        }
        saved.completed = false;
        saved.attempts = saved
            .attempts
            .checked_add(1)
            .context("Host attempt counter exhausted")?;
        saved.save(&directory)?;
        saved
    } else {
        let created = State {
            profile: "connect-codex-agent-host/1".into(),
            installation: installation.digest,
            identity_id: installation.client.identity_id.clone(),
            credential_id: installation.client.credential_id.clone(),
            session_id: None,
            run_id: uuid::Uuid::new_v4().to_string(),
            prompt: supplied.context("A first host turn requires a protected prompt file")?,
            completed: false,
            attempts: 1,
        };
        created.create(&directory)?;
        created
    };
    codex_host::process::execute(
        &directory,
        binary,
        &installation.configuration,
        "CONNECT_AGENT_CREDENTIAL",
        &installation.client.credential,
        &mut state,
        deadline,
    )
    .await?;
    println!(
        "{}",
        serde_json::json!({"status":"host-turn-completed","access_mode":"agent-credential",
        "identity_id":state.identity_id,"credential_id":state.credential_id,"session_id":state.session_id,"run_id":state.run_id})
    );
    Ok(())
}
