//! Validate the completed export, retained grant and exact reviewed skill assets.
use super::super::{read_bounded, require_private, validate_grant, GrantFile};
use anyhow::{Context, Result};
use connect_protocol::tasks::{InvocationKind, TaskAccessCommand, TASK_ACCESS_PROFILE};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{net::SocketAddr, path::Path};

pub(super) struct Installation {
    pub file: GrantFile,
    pub digest: String,
    pub config: String,
}
pub(super) fn load(directory: &Path) -> Result<Installation> {
    let meta = std::fs::symlink_metadata(directory)?;
    anyhow::ensure!(
        meta.file_type().is_dir(),
        "Expected an installation directory"
    );
    require_private(directory)?;
    let manifest_path = directory.join("install.json");
    require_private(&manifest_path)?;
    let bytes = read_bounded(&manifest_path)?;
    let manifest: Value =
        serde_json::from_slice(&bytes).context("Invalid installation manifest")?;
    anyhow::ensure!(
        manifest["profile"] == "connect-codex-install/1"
            && manifest["status"] == "exported-not-refreshed",
        "Incomplete or unsupported installation"
    );
    let grant_path = directory.join("task-grant.json");
    require_private(&grant_path)?;
    let file: GrantFile = serde_json::from_slice(&read_bounded(&grant_path)?)
        .map_err(|_| anyhow::anyhow!("Invalid protected grant file"))?;
    anyhow::ensure!(
        file.profile == TASK_ACCESS_PROFILE,
        "Unsupported grant profile"
    );
    file.request.validate(chrono::Utc::now().timestamp())?;
    let TaskAccessCommand::Issue { definition, .. } = &file.request.command else {
        anyhow::bail!("Missing retained issuance intent");
    };
    let grant = file.grant.as_ref().context("Issuance is pending")?;
    validate_grant(grant, definition)?;
    anyhow::ensure!(
        matches!(
            grant.grant.binding.kind,
            InvocationKind::Native | InvocationKind::HostSession
        ),
        "Provider authority cannot be used by a local host"
    );
    anyhow::ensure!(
        manifest["task_id"] == grant.grant.task_id
            && manifest["grant_id"] == grant.grant.id
            && manifest["binding"] == serde_json::to_value(&grant.grant.binding)?
            && manifest["expires_at"] == grant.grant.expires_at,
        "Installation does not match its retained grant"
    );
    let uri = manifest["mcp_uri"].as_str().context("Missing MCP URI")?;
    let address: SocketAddr = uri
        .strip_prefix("http://")
        .and_then(|s| s.strip_suffix("/mcp"))
        .context("Invalid local MCP URI")?
        .parse()?;
    super::super::transport::validate_address(address)?;
    for name in ["SKILL.md", "agents/openai.yaml", "references/json-cli.md"] {
        let path = directory.join(".agents/skills/connect").join(name);
        require_private(&path)?;
        anyhow::ensure!(
            manifest["skill_files"][name] == hex::encode(Sha256::digest(read_bounded(&path)?)),
            "Installed skill asset has changed"
        );
    }
    let config = super::super::codex::configuration(uri);
    let saved = read_bounded(&directory.join("mcp-override.txt"))?;
    anyhow::ensure!(
        saved == config.as_bytes()
            || saved == super::super::codex::legacy_configuration(uri).as_bytes(),
        "MCP configuration has changed"
    );
    Ok(Installation {
        file,
        digest: hex::encode(Sha256::digest(&bytes)),
        config,
    })
}
