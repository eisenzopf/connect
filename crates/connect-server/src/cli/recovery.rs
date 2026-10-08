//! Local recovery commands. Manifests are completion markers, never secret logs.
use super::init::{private_directory, private_write};
use crate::{config::ServerConfig, installation::InstallationLock};
use anyhow::{Context, Result};
use connect_client::ClientConfig;
use connect_store::{Snapshot, Store};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    path::{Path, PathBuf},
};

const FILES: &[&str] = &[
    "state.sqlite3",
    "certificate.der",
    "private-key.der",
    "server.toml",
    "operator.toml",
    "reviews.toml",
    "agent.toml",
];
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupManifest {
    pub version: u32,
    pub created_at: String,
    pub binary_version: String,
    pub snapshot: Snapshot,
    pub files: BTreeMap<String, String>,
}
fn hash(path: &Path) -> Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 16 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(hex::encode(digest.finalize()))
}
fn bounded_read(path: &Path) -> Result<Vec<u8>> {
    anyhow::ensure!(
        std::fs::metadata(path)?.len() <= 1024 * 1024,
        "Recovery config or key exceeds 1 MiB"
    );
    Ok(std::fs::read(path)?)
}
fn portable_client(path: &Path, endpoint: SocketAddr) -> Result<Vec<u8>> {
    let mut config = ClientConfig::load(path)?;
    config.trust_certificate = PathBuf::from("certificate.der");
    config.endpoint = endpoint;
    Ok(toml::to_string_pretty(&config)?.into_bytes())
}
pub fn verified_manifest(directory: &Path) -> Result<BackupManifest> {
    let manifest: BackupManifest =
        serde_json::from_slice(&bounded_read(&directory.join("manifest.json"))?)
            .map_err(|_| anyhow::anyhow!("Backup manifest is invalid or incomplete"))?;
    anyhow::ensure!(manifest.version == 1, "Unsupported backup manifest version");
    for required in &FILES[..5] {
        anyhow::ensure!(
            manifest.files.contains_key(*required),
            "Backup is missing required recovery material"
        );
    }
    for (name, expected) in &manifest.files {
        anyhow::ensure!(
            FILES.contains(&name.as_str()),
            "Backup manifest contains an unsupported path"
        );
        let path = directory.join(name);
        anyhow::ensure!(
            std::fs::symlink_metadata(&path)?.is_file(),
            "Backup file must be a regular file"
        );
        anyhow::ensure!(
            &hash(&path)? == expected,
            "Backup checksum mismatch: {name}"
        );
    }
    anyhow::ensure!(
        Store::inspect_snapshot(&directory.join("state.sqlite3"))? == manifest.snapshot,
        "Backup database does not match its manifest"
    );
    Ok(manifest)
}

pub fn backup(server_path: &Path, operator_path: &Path, output: &Path) -> Result<()> {
    let server = ServerConfig::load(server_path)?;
    let operator = ClientConfig::load(operator_path)?;
    Store::authorize_recovery(&server.database, &operator.credential)?;
    // Serialize against stopped-state maintenance; online message writers remain
    // safe because SQLite's backup API captures one committed snapshot.
    private_directory(output).context("Backup destination must be a new directory")?;
    let snapshot = Store::backup_database(&server.database, &output.join("state.sqlite3"))?;
    private_write(
        &output.join("certificate.der"),
        &bounded_read(&server.certificate)?,
    )?;
    private_write(
        &output.join("private-key.der"),
        &bounded_read(&server.private_key)?,
    )?;
    let mut portable = server.clone();
    portable.database = "state.sqlite3".into();
    portable.certificate = "certificate.der".into();
    portable.private_key = "private-key.der".into();
    private_write(
        &output.join("server.toml"),
        toml::to_string_pretty(&portable)?.as_bytes(),
    )?;
    private_write(
        &output.join("operator.toml"),
        &portable_client(operator_path, server.bind)?,
    )?;
    let parent = server_path.parent().unwrap_or(Path::new("."));
    for name in ["reviews.toml", "agent.toml"] {
        let source = parent.join(name);
        if source.is_file() {
            private_write(&output.join(name), &portable_client(&source, server.bind)?)?;
        }
    }
    let mut files = BTreeMap::new();
    for name in FILES {
        let path = output.join(name);
        if path.is_file() {
            files.insert((*name).to_owned(), hash(&path)?);
        }
    }
    let manifest = BackupManifest {
        version: 1,
        created_at: chrono::Utc::now().to_rfc3339(),
        binary_version: env!("CARGO_PKG_VERSION").into(),
        snapshot,
        files,
    };
    // A partially written backup lacks this final, durable completion marker.
    private_write(
        &output.join("manifest.json"),
        &serde_json::to_vec_pretty(&manifest)?,
    )?;
    verified_manifest(output)?;
    println!("Verified protected local backup: {}", output.display());
    Ok(())
}

pub fn restore(
    backup: &Path,
    directory: &Path,
    bind: SocketAddr,
    websocket_bind: Option<SocketAddr>,
) -> Result<()> {
    let manifest = verified_manifest(backup)?;
    let mut server = ServerConfig::load(&backup.join("server.toml"))?;
    server.bind = bind;
    server.websocket_bind = websocket_bind;
    server.validate()?;
    server.database = "state.sqlite3".into();
    server.certificate = "certificate.der".into();
    server.private_key = "private-key.der".into();
    private_directory(directory).context("Restore destination must be a fresh directory")?;
    for name in manifest.files.keys().filter(|n| *n != "server.toml") {
        if name.ends_with(".toml") {
            private_write(
                &directory.join(name),
                &portable_client(&backup.join(name), bind)?,
            )?;
        } else if name == "state.sqlite3" {
            let info = Store::backup_database(&backup.join(name), &directory.join(name))?;
            anyhow::ensure!(info == manifest.snapshot, "Restored database mismatch");
        } else {
            private_write(&directory.join(name), &bounded_read(&backup.join(name))?)?;
        }
    }
    private_write(
        &directory.join("server.toml"),
        toml::to_string_pretty(&server)?.as_bytes(),
    )?;
    anyhow::ensure!(
        Store::inspect_snapshot(&directory.join("state.sqlite3"))? == manifest.snapshot,
        "Restored installation differs from backup"
    );
    println!(
        "Restored local installation: {} schema={}",
        directory.display(),
        manifest.snapshot.schema
    );
    Ok(())
}

pub fn migrate(server_path: &Path, operator_path: &Path, backup: &Path) -> Result<()> {
    let server = ServerConfig::load(server_path)?;
    let _lock = InstallationLock::acquire(&server.database)?;
    let operator = ClientConfig::load(operator_path)?;
    Store::authorize_recovery(&server.database, &operator.credential)?;
    let manifest = verified_manifest(backup)?;
    let snapshot = Store::migrate(
        &server.database,
        &backup.join("state.sqlite3"),
        &manifest.snapshot,
    )?;
    println!(
        "Migration complete schema={} installation={}",
        snapshot.schema, snapshot.installation_id
    );
    Ok(())
}
