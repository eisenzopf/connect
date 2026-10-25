//! Fresh signed canonical installation. Trusted offline enrollment precedes use.
use super::init::private_write;
use crate::canonical::CanonicalServerConfig;
use anyhow::{Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use connect_client::canonical::CanonicalClientConfig;
use connect_protocol::{provisioning::*, Limits};
use connect_store::{IssuedCredential, Scope, Store};
use std::{net::SocketAddr, path::Path};

/// Publish only a complete new installation. Never convert an existing directory
/// or infer a server signing pin from an unauthenticated connection.
pub fn initialize(directory: &Path, example: bool, bind: SocketAddr) -> Result<()> {
    initialize_with_http(directory, example, bind, None)
}
pub fn initialize_with_http(
    directory: &Path,
    example: bool,
    bind: SocketAddr,
    http_bind: Option<SocketAddr>,
) -> Result<()> {
    #[cfg(not(unix))]
    anyhow::bail!("Canonical initialization requires the qualified Unix filesystem profile");
    anyhow::ensure!(bind.ip().is_loopback(), "Initialization requires loopback");
    anyhow::ensure!(
        http_bind.is_none_or(|a| a.ip().is_loopback() && a.port() != 0),
        "Saved application configuration requires a concrete loopback HTTP port"
    );
    anyhow::ensure!(
        std::fs::symlink_metadata(directory)
            .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
        "Initialization requires a new destination directory"
    );
    let parent = directory
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let stage = parent.join(format!(".connect-init-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir(&stage)?;
    struct Staging(Option<std::path::PathBuf>);
    impl Drop for Staging {
        fn drop(&mut self) {
            if let Some(path) = self.0.take() {
                let _ = std::fs::remove_dir_all(path);
            }
        }
    }
    let mut staging = Staging(Some(stage.clone()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&stage, std::fs::Permissions::from_mode(0o700))?;
    }
    populate(&stage, example, bind, http_bind)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut directories = vec![stage.clone()];
        while let Some(path) = directories.pop() {
            for entry in std::fs::read_dir(path)? {
                let entry = entry?;
                let kind = entry.file_type()?;
                anyhow::ensure!(
                    kind.is_file() || kind.is_dir(),
                    "Unexpected bootstrap file type"
                );
                let mode = if kind.is_dir() { 0o700 } else { 0o600 };
                std::fs::set_permissions(entry.path(), std::fs::Permissions::from_mode(mode))?;
                if kind.is_dir() {
                    directories.push(entry.path());
                }
            }
        }
    }
    std::fs::File::open(&stage)?.sync_all()?;
    // Unlike ordinary rename, this cannot replace a destination created after
    // preflight. A failure leaves every pre-existing file/directory untouched.
    #[cfg(unix)]
    publish(&stage, directory)?;
    staging.0.take();
    std::fs::File::open(parent)?
        .sync_all()
        .context("Canonical installation published; parent directory sync failed")?;
    drop(staging);
    println!(
        "Initialized signed canonical installation: {}",
        directory.display()
    );
    Ok(())
}
#[cfg(unix)]
fn publish(stage: &Path, directory: &Path) -> Result<()> {
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        stage,
        rustix::fs::CWD,
        directory,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .context("Publish new canonical installation without replacement")?;
    Ok(())
}
fn populate(
    directory: &Path,
    example: bool,
    bind: SocketAddr,
    http_bind: Option<SocketAddr>,
) -> Result<()> {
    let store = Store::initialize(&directory.join("state.sqlite3"), Limits::default())?;
    let tls = crate::tls::generate_local_certificate()?;
    private_write(&directory.join("certificate.der"), tls.cert.der().as_ref())?;
    private_write(
        &directory.join("private-key.der"),
        &tls.signing_key.serialize_der(),
    )?;
    let signing = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519)?;
    private_write(
        &directory.join("server-signing.der"),
        &signing.serialize_der(),
    )?;
    private_write(
        &directory.join("server-signing.pub"),
        signing.public_key_raw(),
    )?;
    let now = chrono::Utc::now();
    let expires = chrono::DateTime::from_timestamp(now.timestamp() + 30 * 86400, 0)
        .context("Canonical bootstrap expiry")?;
    let server = CanonicalServerConfig {
        network_profile: Default::default(),
        webtransport: None,
        websocket: None,
        session_reconnect_grace_seconds: 30,
        bind,
        database: "state.sqlite3".into(),
        certificate: "certificate.der".into(),
        private_key: "private-key.der".into(),
        signing_key_id: "connect-server".into(),
        signing_algorithm: "EdDSA".into(),
        signing_private_key: "server-signing.der".into(),
        signing_key_not_before: now - chrono::Duration::seconds(1),
        signing_key_expires_at: expires,
        http_bind,
        record_https: None,
        record_retention: Default::default(),
        user_assurance: Vec::new(),
        telephone: None,
        public_vapi: None,
        provider_voice: None,
        provider_messages: None,
        limits: Limits::default(),
    };
    server.validate()?;
    let operator = store.create_identity("operator")?;
    let admin = store.issue(&operator.id, Scope::operator(), expires.timestamp())?;
    let administrator = admin.id.clone();
    write_client(
        &store,
        directory,
        "operator",
        admin,
        &administrator,
        &server,
    )?;
    if example {
        let reviews = store.create_identity("reviews")?;
        let agent = store.create_identity("agent")?;
        let reviews_key = store.issue(
            &reviews.id,
            Scope::collaboration(vec![agent.id.clone()]),
            expires.timestamp(),
        )?;
        let agent_key = store.issue(
            &agent.id,
            Scope::collaboration(vec![reviews.id]),
            expires.timestamp(),
        )?;
        write_client(
            &store,
            directory,
            "reviews",
            reviews_key,
            &administrator,
            &server,
        )?;
        write_client(
            &store,
            directory,
            "agent",
            agent_key,
            &administrator,
            &server,
        )?;
    }
    private_write(
        &directory.join("server.toml"),
        toml::to_string_pretty(&server)?.as_bytes(),
    )?;
    store.integrity_check()?;
    Ok(())
}
fn write_client(
    store: &Store,
    directory: &Path,
    name: &str,
    credential: IssuedCredential,
    administrator: &str,
    server: &CanonicalServerConfig,
) -> Result<()> {
    let actor = store.revalidate(&credential.id)?;
    if let Some(endpoint) = server.http_bind {
        let application = connect_client::application::ApplicationClientConfig {
            endpoint,
            credential: credential.token.clone(),
            expires_at: chrono::DateTime::from_timestamp(actor.expires_at, 0)
                .context("Application credential expiry")?,
            timeout_ms: 10_000,
        };
        application.validate()?;
        private_write(
            &directory.join(format!("{name}.application.toml")),
            toml::to_string_pretty(&application)?.as_bytes(),
        )?;
    }
    let pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519)?;
    let private = format!("{name}-signing.der");
    private_write(&directory.join(&private), &pair.serialize_der())?;
    let device_id = format!("device_{}", uuid::Uuid::new_v4().simple());
    let key_id = format!("key_{}", uuid::Uuid::new_v4().simple());
    let platform = std::env::consts::OS;
    store.execute_provisioning(
        administrator,
        ProvisioningCommand::Endpoint {
            operation_id: format!("bootstrap-device-{name}"),
            identity_id: actor.identity_id.clone(),
            device: DeviceSpec {
                id: device_id.clone(),
                kind: "desktop".into(),
                platform: platform.into(),
            },
            transport: EndpointTransport::Quic,
            address: format!("quic://localhost:{}/{device_id}", server.bind.port()),
            capabilities: vec!["data".into()],
            secret_ref: None,
        },
    )?;
    store.execute_provisioning(
        administrator,
        ProvisioningCommand::SigningKey {
            operation_id: format!("bootstrap-key-{name}"),
            identity_id: actor.identity_id.clone(),
            credential_id: credential.id,
            device_id: device_id.clone(),
            key_id: key_id.clone(),
            algorithm: SigningAlgorithm::EdDSA,
            public_key: URL_SAFE_NO_PAD.encode(pair.public_key_raw()),
            not_before: server.signing_key_not_before.timestamp(),
            expires_at: actor.expires_at,
            expected_revision: None,
        },
    )?;
    let client = CanonicalClientConfig {
        network_profile: Default::default(),
        endpoint: server.bind,
        server_name: "localhost".into(),
        trust_certificate: "certificate.der".into(),
        credential: credential.token,
        identity_id: actor.identity_id,
        device_id,
        device_kind: "desktop".into(),
        device_platform: platform.into(),
        signing_key_id: key_id,
        signing_algorithm: "EdDSA".into(),
        signing_private_key: private.into(),
        server_key_id: server.signing_key_id.clone(),
        server_algorithm: server.signing_algorithm.clone(),
        server_public_key: "server-signing.pub".into(),
        server_key_not_before: server.signing_key_not_before,
        server_key_expires_at: server.signing_key_expires_at,
        timeout_ms: 10_000,
    };
    client.validate()?;
    private_write(
        &directory.join(format!("{name}.toml")),
        toml::to_string_pretty(&client)?.as_bytes(),
    )?;
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn destination_created_after_preflight_is_preserved_even_when_empty() {
        use std::os::unix::fs::MetadataExt;
        let temp = tempfile::tempdir().unwrap();
        let stage = temp.path().join("stage");
        let target = temp.path().join("destination");
        std::fs::create_dir(&stage).unwrap();
        std::fs::write(stage.join("ready"), b"new state").unwrap();
        assert!(!target.exists());
        std::fs::create_dir(&target).unwrap();
        let original = std::fs::metadata(&target).unwrap().ino();
        assert!(publish(&stage, &target).is_err());
        assert_eq!(std::fs::metadata(&target).unwrap().ino(), original);
        assert_eq!(std::fs::read_dir(&target).unwrap().count(), 0);
        assert_eq!(std::fs::read(stage.join("ready")).unwrap(), b"new state");
    }
}
