//! Explicit credential and trust maintenance; normal initialization never rotates.
use super::init::{private_directory, private_replace, private_write};
use crate::{config::ServerConfig, installation::InstallationLock, tls};
use anyhow::Result;
use connect_client::ClientConfig;
use connect_store::{Scope, Store};
use std::path::Path;

/// Add task ownership and delegation to an explicitly issued local credential.
/// Communication rights and recipients are unchanged; this grants no admin scope.
pub fn with_task_management(mut scope: Scope) -> Scope {
    scope.operations.extend(
        [
            "task_create",
            "task_get",
            "task_cancel",
            "task_replies",
            "task_consume_reply",
            "task_invoke",
            "task_inbox",
            "task_wait",
            "task_recall",
            "task_effect_get",
            "task_grant",
        ]
        .map(str::to_string),
    );
    scope
}

pub fn rotate_tls(server_path: &Path, operator_path: &Path) -> Result<()> {
    let mut server = ServerConfig::load(server_path)?;
    let _lock = InstallationLock::acquire(&server.database)?;
    let operator = ClientConfig::load(operator_path)?;
    Store::authorize_recovery(&server.database, &operator.credential)?;
    let parent = server_path.parent().unwrap_or(Path::new("."));
    let name = format!("tls-{}", uuid::Uuid::new_v4());
    let directory = parent.join(&name);
    private_directory(&directory)?;
    let generated = tls::generate_local_certificate()?;
    private_write(
        &directory.join("certificate.der"),
        generated.cert.der().as_ref(),
    )?;
    private_write(
        &directory.join("private-key.der"),
        &generated.signing_key.serialize_der(),
    )?;
    server.certificate = Path::new(&name).join("certificate.der");
    server.private_key = Path::new(&name).join("private-key.der");
    server.database = server.database.canonicalize()?;
    // Switching the server's pair is one atomic config replacement. If client
    // config refresh is interrupted, old trust fails closed until refreshed.
    private_replace(server_path, toml::to_string_pretty(&server)?.as_bytes())?;
    let certificate = directory.join("certificate.der").canonicalize()?;
    let mut clients = vec![
        operator_path.to_path_buf(),
        parent.join("reviews.toml"),
        parent.join("agent.toml"),
        parent.join("outside.toml"),
    ];
    clients.sort();
    clients.dedup();
    for path in clients.into_iter().filter(|p| p.is_file()) {
        let mut config = ClientConfig::load(&path)?;
        config.trust_certificate = certificate.clone();
        private_replace(&path, toml::to_string_pretty(&config)?.as_bytes())?;
    }
    println!(
        "TLS rotated; local demo trust refreshed. Other clients must explicitly trust: {}",
        certificate.display()
    );
    Ok(())
}
