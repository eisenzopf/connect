//! Intended Native SDK credential export, separate from network issuance.
use super::{private_write, read_bounded, require_private, validate_grant, GrantFile};
use anyhow::{Context, Result};
use connect_protocol::tasks::{TaskAccessCommand, TASK_ACCESS_PROFILE};
use std::path::Path;

/// Export an intended Native grant to a fresh protected SDK config. This is
/// local metadata conversion, not issuance, refresh or current authentication.
/// The parent token is replaced and existing output files are preserved.
pub fn export_native(config: &Path, grant_file: &Path, output: &Path) -> Result<()> {
    require_private(grant_file)?;
    let file: GrantFile = serde_json::from_slice(&read_bounded(grant_file)?)
        .map_err(|_| anyhow::anyhow!("Invalid protected grant file"))?;
    anyhow::ensure!(
        file.profile == TASK_ACCESS_PROFILE,
        "Unsupported grant file profile"
    );
    let TaskAccessCommand::Issue { definition, .. } = &file.request.command else {
        anyhow::bail!("Grant file must contain retained issuance intent");
    };
    let grant = file
        .grant
        .as_ref()
        .context("Issuance is pending; reconcile before export")?;
    validate_grant(grant, definition)?;
    let client = connect_client::ClientConfig::load(config)?.with_task_grant(grant)?;
    private_write(output, toml::to_string_pretty(&client)?.as_bytes())?;
    println!(
        "{}",
        serde_json::json!({"grant_id":grant.grant.id,"task_id":grant.grant.task_id,"status":"exported-not-refreshed"})
    );
    Ok(())
}
