//! Protected local grant files retain issuance intent before a possible dispatch.
use super::init::{private_replace, private_write};
use anyhow::{Context, Result};
use connect_protocol::tasks::{
    ProtectedTaskGrant, TaskAccessCommand, TaskAccessOutcome, TaskAccessRequest,
    TASK_ACCESS_PROFILE,
};
use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, path::Path};
mod arguments;
mod transport;
pub use arguments::Command;
use transport::{exchange, validate_address};

const MAX_BYTES: usize = 65_536;

mod export;
pub use export::export_native;
pub mod codex;
pub mod codex_host;

/// Protected local installation state. Pending intent survives a lost response;
/// only this intended credential file contains the issued bearer plaintext.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrantFile {
    pub profile: String,
    pub request: TaskAccessRequest,
    pub grant: Option<ProtectedTaskGrant>,
}

/// Persist a caller-owned issuance ID and inputs before contacting the server.
/// Existing files are never replaced by a fresh issue; resume retains the intent.
pub async fn issue(config: &Path, address: SocketAddr, input: &Path, output: &Path) -> Result<()> {
    let request: TaskAccessRequest = serde_json::from_slice(&read_bounded(input)?)
        .map_err(|_| anyhow::anyhow!("Invalid protected task access request"))?;
    anyhow::ensure!(
        matches!(request.command, TaskAccessCommand::Issue { .. }),
        "Expected an issue command"
    );
    request.validate(chrono::Utc::now().timestamp())?;
    validate_address(address)?;
    let file = GrantFile {
        profile: TASK_ACCESS_PROFILE.into(),
        request,
        grant: None,
    };
    private_write(output, &serde_json::to_vec_pretty(&file)?)?;
    resume(config, address, output).await
}

/// Reconcile the original protected issuance. Does not invent an ID or extend
/// its lifetime. Refresh is an explicit new issue under current backing authority.
pub async fn resume(config: &Path, address: SocketAddr, output: &Path) -> Result<()> {
    require_private(output)?;
    let mut file: GrantFile = serde_json::from_slice(&read_bounded(output)?)
        .map_err(|_| anyhow::anyhow!("Invalid protected grant file"))?;
    anyhow::ensure!(
        file.profile == TASK_ACCESS_PROFILE,
        "Unsupported grant file profile"
    );
    let TaskAccessCommand::Issue { definition, .. } = &file.request.command else {
        anyhow::bail!("Grant file must contain retained issuance intent");
    };
    if let Some(grant) = &file.grant {
        validate_grant(grant, definition)?;
        report(grant, "saved-not-refreshed");
        return Ok(());
    }
    let TaskAccessOutcome::Issued(grant) = exchange(config, address, &file.request)
        .await
        .context("Issuance may be unknown; resume this grant file with its original request ID")?
    else {
        anyhow::bail!("Server did not return an intended grant exchange");
    };
    validate_grant(&grant, definition)?;
    file.grant = Some(grant);
    private_replace(output, &serde_json::to_vec_pretty(&file)?).context(
        "Grant export failed; retained issuance must be reconciled with its original ID",
    )?;
    report(
        file.grant.as_ref().context("Missing issued grant")?,
        "issued",
    );
    Ok(())
}

pub(super) fn require_private(output: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        anyhow::ensure!(
            std::fs::symlink_metadata(output)?.permissions().mode() & 0o077 == 0,
            "Grant file must have private permissions"
        );
    }
    Ok(())
}

/// Explicit developer revocation; does not cancel the task or conversation.
pub async fn revoke(config: &Path, address: SocketAddr, grant_id: &str) -> Result<()> {
    let request = TaskAccessRequest::new(TaskAccessCommand::Revoke {
        grant_id: grant_id.into(),
    });
    let result = exchange(config, address, &request).await?;
    anyhow::ensure!(
        matches!(result, TaskAccessOutcome::Revoked { grant_id: id } if id == grant_id),
        "Server did not confirm revocation"
    );
    println!(
        "{}",
        serde_json::json!({"grant_id":grant_id,"status":"revoked"})
    );
    Ok(())
}

fn report(grant: &ProtectedTaskGrant, status: &str) {
    println!(
        "{}",
        serde_json::json!({"grant_id":grant.grant.id,"task_id":grant.grant.task_id,"expires_at":grant.grant.expires_at,"status":status})
    );
}
pub(super) fn read_bounded(path: &Path) -> Result<Vec<u8>> {
    use std::io::Read;
    let metadata = std::fs::symlink_metadata(path)?;
    anyhow::ensure!(
        metadata.file_type().is_file(),
        "Expected a regular task access file"
    );
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() <= MAX_BYTES, "Task access file exceeds 64 KiB");
    Ok(bytes)
}

pub(super) fn validate_grant(
    grant: &ProtectedTaskGrant,
    definition: &connect_protocol::tasks::GrantDefinition,
) -> Result<()> {
    anyhow::ensure!(
        uuid::Uuid::parse_str(&grant.grant.id).is_ok()
            && grant.grant.task_id == definition.task_id
            && grant.grant.binding == definition.binding
            && grant.grant.operations == definition.operations
            && grant.grant.expires_at == definition.expires_at
            && !grant.grant.revoked
            && !grant.expose_secret().is_empty()
            && grant.expose_secret().len() <= 2048
            && grant.expose_secret().is_ascii(),
        "Server grant does not match retained issuance intent"
    );
    Ok(())
}
