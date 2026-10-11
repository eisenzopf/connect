//! Export an already-issued local grant and workflow assets, never authority.
use super::{private_write, read_bounded, require_private, validate_grant, GrantFile};
use crate::cli::init::private_directory;
use anyhow::{Context, Result};
use connect_protocol::tasks::{InvocationKind, TaskAccessCommand, TASK_ACCESS_PROFILE};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, net::SocketAddr, path::Path};

const SKILL: &[(&str, &str)] = &[
    (
        "SKILL.md",
        include_str!("../../../../../skills/connect/SKILL.md"),
    ),
    (
        "agents/openai.yaml",
        include_str!("../../../../../skills/connect/agents/openai.yaml"),
    ),
    (
        "references/json-cli.md",
        include_str!("../../../../../skills/connect/references/json-cli.md"),
    ),
];

/// Export to a new private project directory without contacting Connect/Codex.
/// Provider grants cannot be converted into local host authority. The manifest
/// is written last; an incomplete directory must never be treated as installed.
/// Current authority is still checked by MCP, not attested by this conversion.
pub fn export(grant_file: &Path, address: SocketAddr, output: &Path) -> Result<()> {
    super::transport::validate_address(address)?;
    require_private(grant_file)?;
    let file: GrantFile = serde_json::from_slice(&read_bounded(grant_file)?)
        .map_err(|_| anyhow::anyhow!("Invalid protected grant file"))?;
    anyhow::ensure!(
        file.profile == TASK_ACCESS_PROFILE,
        "Unsupported grant file profile"
    );
    file.request.validate(chrono::Utc::now().timestamp())?;
    let TaskAccessCommand::Issue { definition, .. } = &file.request.command else {
        anyhow::bail!("Grant file must retain its issuance intent");
    };
    let grant = file
        .grant
        .as_ref()
        .context("Issuance is pending; reconcile before export")?;
    validate_grant(grant, definition)?;
    anyhow::ensure!(
        matches!(
            grant.grant.binding.kind,
            InvocationKind::Native | InvocationKind::HostSession
        ),
        "Provider grants require their verified provider adapter"
    );

    // Exclusive creation preserves both complete and interrupted installations.
    // Parent directories are chosen by the developer and must already exist.
    private_directory(output)?;
    private_write(
        &output.join("task-grant.json"),
        &serde_json::to_vec_pretty(&file)?,
    )?;
    let skill_root = output.join(".agents/skills/connect");
    std::fs::create_dir_all(&skill_root)?;
    let mut digests = BTreeMap::new();
    for (name, content) in SKILL {
        let path = skill_root.join(name);
        std::fs::create_dir_all(path.parent().context("Skill asset parent is missing")?)?;
        private_write(&path, content.as_bytes())?;
        digests.insert(*name, hex::encode(Sha256::digest(content.as_bytes())));
    }
    let uri = format!("http://{address}/mcp");
    let config = format!(
        "mcp_servers.connect_task={{url=\"{uri}\",bearer_token_env_var=\"CONNECT_TASK_GRANT\",required=true,startup_timeout_sec=10,tool_timeout_sec=10}}"
    );
    private_write(&output.join("mcp-override.txt"), config.as_bytes())?;
    let manifest = serde_json::json!({
        "profile":"connect-codex-install/1", "status":"exported-not-refreshed",
        "task_id":grant.grant.task_id, "grant_id":grant.grant.id,
        "binding":grant.grant.binding, "expires_at":grant.grant.expires_at,
        "mcp_uri":uri, "skill_files":digests,
    });
    private_write(
        &output.join("install.json"),
        &serde_json::to_vec_pretty(&manifest)?,
    )?;
    println!(
        "{}",
        serde_json::json!({
            "task_id":grant.grant.task_id, "grant_id":grant.grant.id,
            "status":"exported-not-refreshed", "directory":output,
        })
    );
    Ok(())
}
