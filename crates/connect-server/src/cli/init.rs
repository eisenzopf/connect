use crate::config::ServerConfig;
use anyhow::{bail, Context, Result};
use connect_client::ClientConfig;
use connect_protocol::Limits;
use connect_store::{Scope, Store};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub fn private_directory(path: &Path) -> Result<()> {
    fs::create_dir(path).context("Create new private state directory")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
pub fn private_write(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .context("Create protected file without overwriting existing data")?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
pub fn initialize(directory: &Path, example: bool, bind: std::net::SocketAddr) -> Result<()> {
    if directory.exists() {
        let config = ServerConfig::load(&directory.join("server.toml"))
            .context("Existing directory is not a complete installation; select a new directory")?;
        Store::open(&config.database, config.limits)?.integrity_check()?;
        println!("Existing installation preserved: {}", directory.display());
        return Ok(());
    }
    if !bind.ip().is_loopback() {
        bail!("Initialization requires a loopback address")
    }
    if let Some(parent) = directory.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    private_directory(directory)?;
    let store = Store::initialize(&directory.join("state.sqlite3"), Limits::default())?;
    let generated = crate::tls::generate_local_certificate()?;
    private_write(
        &directory.join("certificate.der"),
        generated.cert.der().as_ref(),
    )?;
    private_write(
        &directory.join("private-key.der"),
        &generated.signing_key.serialize_der(),
    )?;
    let operator = store.create_identity("operator")?;
    let expiry = chrono::Utc::now().timestamp() + 30 * 24 * 60 * 60;
    let key = store.issue(&operator.id, Scope::operator(), expiry)?;
    let config = |key: connect_store::IssuedCredential, identity_id: String| ClientConfig {
        endpoint: bind,
        server_name: "localhost".into(),
        trust_certificate: PathBuf::from("certificate.der"),
        credential: key.token,
        identity_id,
        credential_id: key.id,
        timeout_ms: 10_000,
    };
    private_write(
        &directory.join("operator.toml"),
        toml::to_string_pretty(&config(key, operator.id))?.as_bytes(),
    )?;
    if example {
        let app = store.create_identity("reviews")?;
        let agent = store.create_identity("agent")?;
        let app_key = store.issue(&app.id, Scope::messaging(vec![agent.id.clone()]), expiry)?;
        let agent_key = store.issue(&agent.id, Scope::messaging(vec![app.id.clone()]), expiry)?;
        private_write(
            &directory.join("reviews.toml"),
            toml::to_string_pretty(&config(app_key, app.id))?.as_bytes(),
        )?;
        private_write(
            &directory.join("agent.toml"),
            toml::to_string_pretty(&config(agent_key, agent.id))?.as_bytes(),
        )?;
    }
    let config = ServerConfig {
        bind,
        websocket_bind: None,
        database: "state.sqlite3".into(),
        certificate: "certificate.der".into(),
        private_key: "private-key.der".into(),
        sip: None,
        webrtc: None,
        limits: Limits::default(),
    };
    private_write(
        &directory.join("server.toml"),
        toml::to_string_pretty(&config)?.as_bytes(),
    )?;
    println!("Initialized local installation: {}", directory.display());
    Ok(())
}

/// Explicit maintenance writes preserve file protection and replace one config
/// atomically. Fresh initialization continues to use exclusive private_write.
pub fn private_replace(path: &Path, bytes: &[u8]) -> Result<()> {
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    private_write(&temporary, bytes)?;
    fs::rename(&temporary, path)?;
    if let Some(parent) = path.parent() {
        fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}
