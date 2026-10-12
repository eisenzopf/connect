//! Atomically adopt owner-issued authority without replacing host recovery intent.
use super::super::{read_bounded, require_private, validate_grant, GrantFile};
use anyhow::{Context, Result};
use connect_protocol::tasks::{TaskAccessCommand, TASK_ACCESS_PROFILE};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Renewal {
    profile: String,
    installation: String,
    file: GrantFile,
}

pub(super) fn validate(file: &GrantFile, current: bool) -> Result<()> {
    anyhow::ensure!(
        file.profile == TASK_ACCESS_PROFILE,
        "Unsupported grant profile"
    );
    let TaskAccessCommand::Issue { definition, .. } = &file.request.command else {
        anyhow::bail!("Missing retained issuance intent");
    };
    // Historical exports remain identity evidence after expiry. Validate their
    // shape at an instant inside their lifetime, never use this to authorize a
    // turn. Only the selected current grant may supply its bearer to the host.
    let now = if current {
        chrono::Utc::now().timestamp()
    } else {
        definition
            .expires_at
            .checked_sub(1)
            .context("Invalid grant expiry")?
    };
    file.request.validate(now)?;
    validate_grant(
        file.grant.as_ref().context("Issuance is pending")?,
        definition,
    )
}

fn same_delegation(original: &GrantFile, replacement: &GrantFile) -> Result<()> {
    let old = &original
        .grant
        .as_ref()
        .context("Missing original grant")?
        .grant;
    let new = &replacement
        .grant
        .as_ref()
        .context("Missing replacement grant")?
        .grant;
    anyhow::ensure!(
        old.task_id == new.task_id
            && old.binding == new.binding
            && old.operations == new.operations,
        "Renewal must preserve the exact task, invocation binding and operations"
    );
    Ok(())
}

pub(super) fn select(
    directory: &Path,
    original: GrantFile,
    digest: &str,
    current: bool,
) -> Result<GrantFile> {
    let path = directory.join("grant-renewal.json");
    let exists = match std::fs::symlink_metadata(&path) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.into()),
    };
    let selected = if exists {
        require_private(&path)?;
        let renewal: Renewal = serde_json::from_slice(&read_bounded(&path)?)
            .map_err(|_| anyhow::anyhow!("Invalid protected grant renewal"))?;
        anyhow::ensure!(
            renewal.profile == "connect-codex-renewal/1" && renewal.installation == digest,
            "Grant renewal belongs to a different installation"
        );
        validate(&renewal.file, current)?;
        same_delegation(&original, &renewal.file)?;
        renewal.file
    } else {
        validate(&original, current)?;
        original
    };
    Ok(selected)
}

/// Adopt an already issued replacement grant under the host installation lock.
/// This developer operation mints no authority, changes no model session, and
/// never gives a backing credential to the host. Current MCP authorization still
/// governs every operation; renewing a file cannot revive a cancelled task.
pub fn renew(directory: &Path, grant_file: &Path) -> Result<()> {
    let directory = std::fs::canonicalize(directory)?;
    super::install::load_for_renewal(&directory)?;
    let _lock = super::lock(&directory)?;
    let installation = super::install::load_for_renewal(&directory)?;
    if let Some(state) = super::state::State::load(&directory)? {
        anyhow::ensure!(
            state.installation == installation.digest
                && state.grant_id == installation.original_grant_id
                && state.task_id
                    == installation
                        .file
                        .grant
                        .as_ref()
                        .context("Missing grant")?
                        .grant
                        .task_id,
            "Saved host state belongs to a different installation"
        );
        anyhow::ensure!(
            state.session_id.is_some(),
            "Missing saved session requires repair before renewal"
        );
    }
    require_private(grant_file)?;
    let file: GrantFile = serde_json::from_slice(&read_bounded(grant_file)?)
        .map_err(|_| anyhow::anyhow!("Invalid protected grant file"))?;
    validate(&file, true)?;
    same_delegation(&installation.file, &file)?;
    let grant = file.grant.as_ref().context("Missing replacement grant")?;
    let report = serde_json::json!({"status":"renewed-not-invoked", "task_id":grant.grant.task_id,
        "grant_id":grant.grant.id,"expires_at":grant.grant.expires_at});
    let renewal = Renewal {
        profile: "connect-codex-renewal/1".into(),
        installation: installation.digest,
        file,
    };
    let path = directory.join("grant-renewal.json");
    // A single atomic credential pointer avoids a multi-file manifest/state
    // transaction. The original export and all saved call intent stay intact.
    crate::cli::init::private_replace(&path, &serde_json::to_vec(&renewal)?)?;
    println!("{report}");
    Ok(())
}
