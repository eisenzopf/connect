//! Protected regular-credential installation; no per-task issuance or conversion.
use crate::{
    cli::{
        init,
        task_access::{codex, read_bounded, require_private},
    },
    config::ServerConfig,
};
use anyhow::{Context, Result};
use connect_client::ClientConfig;
use connect_protocol::tasks::TASK_PRINCIPAL_PREFIX;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, net::SocketAddr, path::Path};

const PROFILE: &str = "connect-codex-agent-install/1";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    profile: String,
    status: String,
    identity_id: String,
    credential_id: String,
    credential_expires_at: i64,
    client_sha256: String,
    mcp_uri: String,
    skill_files: BTreeMap<String, String>,
}

pub(super) struct Installation {
    pub client: ClientConfig,
    pub credential_expires_at: i64,
    pub digest: String,
    pub configuration: String,
}

/// Export an already-issued regular agent key after read-only local validation.
/// The key cannot be an administrator, task grant or provider credential. This
/// writes only a new private host directory and never rotates existing authority.
pub fn export(
    config_path: &Path,
    server_path: &Path,
    address: SocketAddr,
    output: &Path,
) -> Result<()> {
    validate_address(address)?;
    let client = load_client(config_path)?;
    validate_regular(&client)?;
    let server = ServerConfig::load(server_path)?;
    let mcp = server
        .mcp
        .as_ref()
        .context("Configure the local MCP listener before export")?;
    anyhow::ensure!(
        (mcp.bind == address || (mcp.bind.port() == 0 && mcp.bind.ip() == address.ip()))
            && mcp.binding.is_none(),
        "Select this installation's regular MCP listener"
    );
    let actor = connect_store::Store::authenticate_file(&server.database, &client.credential)?;
    anyhow::ensure!(
        !actor.permits("admin"),
        "Use a separately issued agent communication credential, not an administrator key"
    );
    anyhow::ensure!(
        actor.identity_id == client.identity_id && actor.credential_id == client.credential_id,
        "Agent configuration does not match its current credential"
    );
    anyhow::ensure!(
        actor.permits("send") || actor.permits("history") || actor.permits("inbox"),
        "Agent credential has no communication access"
    );
    let client_bytes = toml::to_string_pretty(&client)?.into_bytes();
    let uri = format!("http://{address}/mcp");
    let manifest = Manifest {
        profile: PROFILE.into(),
        status: "exported-existing-credential".into(),
        identity_id: actor.identity_id,
        credential_id: actor.credential_id,
        credential_expires_at: actor.expires_at,
        client_sha256: hex::encode(Sha256::digest(&client_bytes)),
        mcp_uri: uri.clone(),
        skill_files: BTreeMap::new(),
    };
    init::private_directory(output)?;
    init::private_write(&output.join("agent.toml"), &client_bytes)?;
    let mut manifest = manifest;
    manifest.skill_files = codex::write_skill(output)?;
    init::private_write(
        &output.join("mcp-override.txt"),
        codex::agent_configuration(&uri).as_bytes(),
    )?;
    // Publish the completion marker last. Existing complete/incomplete exports
    // remain untouched; never substitute this authority for a task installation.
    init::private_write(
        &output.join("install.json"),
        &serde_json::to_vec_pretty(&manifest)?,
    )?;
    println!(
        "{}",
        serde_json::json!({"status":"agent-exported", "identity_id":manifest.identity_id,
        "credential_id":manifest.credential_id, "mcp_uri":uri, "directory":output})
    );
    Ok(())
}

fn validate_address(address: SocketAddr) -> Result<()> {
    anyhow::ensure!(
        address.ip().is_loopback() && address.port() != 0,
        "Agent installation requires a concrete loopback MCP address"
    );
    Ok(())
}

fn validate_regular(client: &ClientConfig) -> Result<()> {
    client.validate()?;
    anyhow::ensure!(
        !client.credential_id.starts_with(TASK_PRINCIPAL_PREFIX)
            && client.credential.len() == 64
            && client.credential.is_ascii(),
        "Use a regular agent credential; task-grant exports keep their separate access mode"
    );
    Ok(())
}

fn load_client(path: &Path) -> Result<ClientConfig> {
    require_private(path)?;
    let bytes = read_bounded(path)?;
    let text = std::str::from_utf8(&bytes).context("Agent configuration must be UTF-8")?;
    let mut client: ClientConfig = toml::from_str(text)
        .map_err(|_| anyhow::anyhow!("Invalid private agent client configuration"))?;
    if client.trust_certificate.is_relative() {
        client.trust_certificate = path
            .parent()
            .unwrap_or(Path::new("."))
            .join(&client.trust_certificate);
    }
    client.trust_certificate = client
        .trust_certificate
        .canonicalize()
        .context("Agent trust certificate is missing")?;
    client.validate()?;
    Ok(client)
}

pub(super) fn load(directory: &Path) -> Result<Installation> {
    anyhow::ensure!(
        std::fs::symlink_metadata(directory)?.file_type().is_dir(),
        "Expected an agent installation directory"
    );
    require_private(directory)?;
    let manifest_path = directory.join("install.json");
    require_private(&manifest_path)?;
    let bytes = read_bounded(&manifest_path)?;
    let manifest: Manifest = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("Invalid agent installation manifest"))?;
    anyhow::ensure!(
        manifest.profile == PROFILE && manifest.status == "exported-existing-credential",
        "Use the matching regular-agent installation; task-grant intent cannot be converted"
    );
    let path = directory.join("agent.toml");
    require_private(&path)?;
    anyhow::ensure!(
        manifest.client_sha256 == hex::encode(Sha256::digest(read_bounded(&path)?)),
        "Agent credential configuration changed; preserve the original host intent"
    );
    let client = load_client(&path)?;
    validate_regular(&client)?;
    anyhow::ensure!(
        manifest.identity_id == client.identity_id
            && manifest.credential_id == client.credential_id,
        "Agent installation identity changed"
    );
    let address: SocketAddr = manifest
        .mcp_uri
        .strip_prefix("http://")
        .and_then(|s| s.strip_suffix("/mcp"))
        .context("Invalid local agent MCP URL")?
        .parse()?;
    validate_address(address)?;
    for (name, _) in codex::SKILL {
        let path = directory.join(".agents/skills/connect").join(name);
        require_private(&path)?;
        anyhow::ensure!(
            manifest.skill_files.get(*name)
                == Some(&hex::encode(Sha256::digest(read_bounded(&path)?))),
            "Installed agent skill changed"
        );
    }
    let configuration = codex::agent_configuration(&manifest.mcp_uri);
    let config_path = directory.join("mcp-override.txt");
    require_private(&config_path)?;
    anyhow::ensure!(
        read_bounded(&config_path)? == configuration.as_bytes(),
        "Agent MCP configuration changed"
    );
    anyhow::ensure!(
        !directory.join("task-grant.json").try_exists()?
            && !directory.join("grant-renewal.json").try_exists()?,
        "Regular-agent installations cannot adopt task-grant files"
    );
    Ok(Installation {
        client,
        credential_expires_at: manifest.credential_expires_at,
        digest: hex::encode(Sha256::digest(&bytes)),
        configuration,
    })
}
