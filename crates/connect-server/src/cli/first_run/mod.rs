//! Provider-free versioned local setup. Publishing the directory is the last step.
mod arguments;
mod demo;
use super::init;
use anyhow::{Context, Result};
pub use arguments::Command;
use connect_client::ClientConfig;
use connect_store::{Scope, Store};
pub use demo::verify;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    net::{SocketAddr, UdpSocket},
    path::Path,
};

/// Durable paths and concrete ports. No bearer secrets or provider credentials.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SetupState {
    pub profile: String,
    pub schema: u32,
    pub bind: SocketAddr,
    pub server_config: String,
    pub human_config: String,
    pub agent_config: String,
    pub secret_references: String,
    pub provider_mode: String,
    pub tasks_required: bool,
}
fn require_private(path: &Path, directory: bool) -> Result<()> {
    let meta = fs::symlink_metadata(path).context("Read protected setup path")?;
    anyhow::ensure!(
        !meta.file_type().is_symlink()
            && if directory {
                meta.is_dir()
            } else {
                meta.is_file()
            },
        "Setup requires regular files and a private directory"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        anyhow::ensure!(
            meta.permissions().mode() & 0o077 == 0,
            "Setup paths are accessible to other users; restore private permissions explicitly"
        );
    }
    Ok(())
}
/// Validate a complete existing installation without silently adopting other state.
pub fn inspect(directory: &Path) -> Result<SetupState> {
    require_private(directory, true)?;
    for name in [
        "setup.json",
        "server.toml",
        "operator.toml",
        "human.toml",
        "agent.toml",
        "private-key.der",
        "secret-references.toml",
        "state.sqlite3",
    ] {
        require_private(&directory.join(name), false)?;
    }
    let state: SetupState = serde_json::from_slice(&fs::read(directory.join("setup.json"))?)
        .context("Incomplete or unsupported setup state; preserve it and select a new directory")?;
    anyhow::ensure!(
        state.profile == "connect-first-run/1"
            && state.schema == connect_store::SCHEMA_VERSION
            && state.server_config == "server.toml"
            && state.human_config == "human.toml"
            && state.agent_config == "agent.toml"
            && state.secret_references == "secret-references.toml"
            && state.provider_mode == "none"
            && !state.tasks_required,
        "Unsupported setup profile; preserve state and use explicit migration"
    );
    let server = crate::config::ServerConfig::load(&directory.join(&state.server_config))?;
    anyhow::ensure!(
        server.bind == state.bind
            && server.database == directory.join("state.sqlite3")
            && server.certificate == directory.join("certificate.der")
            && server.private_key == directory.join("private-key.der"),
        "Setup configuration no longer matches its local paths/port"
    );
    let store = Store::open_administration(&server.database, server.limits)?;
    store.integrity_check()?;
    for name in [&state.human_config, &state.agent_config] {
        let client = ClientConfig::load(&directory.join(name))?;
        let actor = store.authenticate(&client.credential)?;
        anyhow::ensure!(
            client.endpoint == state.bind
                && actor.identity_id == client.identity_id
                && actor.credential_id == client.credential_id
                && !actor.permits("admin"),
            "Local communication credential no longer matches setup"
        );
    }
    Ok(state)
}
/// Create an installation at an explicit new path. Repeat calls validate/preserve
/// complete state; partial state and listener conflicts require explicit action.
pub fn initialize(directory: &Path, bind: SocketAddr) -> Result<SetupState> {
    let directory = if directory.is_absolute() {
        directory.to_path_buf()
    } else {
        std::env::current_dir()?.join(directory)
    };
    let parent = directory
        .parent()
        .context("Setup requires a parent directory")?;
    fs::create_dir_all(parent)?;
    let name = directory
        .file_name()
        .and_then(|s| s.to_str())
        .context("Invalid setup directory name")?;
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options.open(parent.join(format!(".{name}.connect-init.lock")))?;
    fs2::FileExt::try_lock_exclusive(&lock)
        .context("Another initializer owns this setup path; state is preserved")?;
    if directory.exists() {
        let state = inspect(&directory)?;
        anyhow::ensure!(
            bind.port() == 0 || state.bind == bind,
            "Existing setup uses another port; preserved without edits"
        );
        return Ok(state);
    }
    anyhow::ensure!(
        bind.ip().is_loopback(),
        "First run requires a loopback listener"
    );
    let parent = directory
        .parent()
        .context("Setup requires a parent directory")?;
    fs::create_dir_all(parent)?;
    let name = directory
        .file_name()
        .and_then(|s| s.to_str())
        .context("Invalid setup directory name")?;
    let staging = parent.join(format!(".{name}.connect-initializing"));
    anyhow::ensure!(
        !staging.exists(),
        "Interrupted initialization is preserved at {}; inspect it before selecting a new path",
        staging.display()
    );
    // Hold the selected port through setup; port zero selects a concrete ephemeral port.
    let reservation = UdpSocket::bind(bind)
        .context("Native port is occupied; choose --bind with another loopback port")?;
    let bind = reservation.local_addr()?;
    init::initialize_quiet(&staging, false, bind)?;
    let server = crate::config::ServerConfig::load(&staging.join("server.toml"))?;
    let store = Store::open_administration(&server.database, server.limits)?;
    let human = store.create_identity("human")?;
    let agent = store.create_identity("agent")?;
    for (alias, identity, peer) in [("human", &human, &agent), ("agent", &agent, &human)] {
        let key = store.issue(
            &identity.id,
            Scope::communications(vec![peer.id.clone()]),
            chrono::Utc::now().timestamp() + 30 * 86400,
        )?;
        let client = ClientConfig {
            endpoint: bind,
            server_name: "localhost".into(),
            trust_certificate: "certificate.der".into(),
            credential: key.token,
            identity_id: identity.id.clone(),
            credential_id: key.id,
            timeout_ms: 10000,
        };
        init::private_write(
            &staging.join(format!("{alias}.toml")),
            toml::to_string_pretty(&client)?.as_bytes(),
        )?;
    }
    let refs = crate::secrets::SecretReferences {
        version: 1,
        references: Default::default(),
    };
    init::private_write(
        &staging.join("secret-references.toml"),
        toml::to_string_pretty(&refs)?.as_bytes(),
    )?;
    let state = SetupState {
        profile: "connect-first-run/1".into(),
        schema: connect_store::SCHEMA_VERSION,
        bind,
        server_config: "server.toml".into(),
        human_config: "human.toml".into(),
        agent_config: "agent.toml".into(),
        secret_references: "secret-references.toml".into(),
        provider_mode: "none".into(),
        tasks_required: false,
    };
    init::private_write(
        &staging.join("setup.json"),
        &serde_json::to_vec_pretty(&state)?,
    )?;
    store.integrity_check()?;
    drop(store);
    inspect(&staging)?;
    // rename is same-filesystem; an existing target is never intentionally replaced.
    anyhow::ensure!(
        !directory.exists(),
        "Setup destination appeared during initialization; staged state preserved"
    );
    fs::rename(&staging, &directory)?;
    fs::File::open(parent)?.sync_all()?;
    drop(reservation);
    inspect(&directory)
}
