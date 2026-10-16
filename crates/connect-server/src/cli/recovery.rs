//! Local recovery commands. Manifests are completion markers, never secret logs.
mod setup_metadata;
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
    "human.toml",
    "setup.json",
    "secret-references.toml",
];
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupManifest {
    pub version: u32,
    pub created_at: String,
    pub binary_version: String,
    pub snapshot: Snapshot,
    pub files: BTreeMap<String, String>,
    #[serde(default)]
    pub blobs: BTreeMap<String, String>,
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
    anyhow::ensure!(
        matches!(manifest.version, 1 | 2),
        "Unsupported backup manifest version"
    );
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
    let referenced = Store::artifact_blob_names(&directory.join("state.sqlite3"))?;
    anyhow::ensure!(
        referenced == manifest.blobs.keys().cloned().collect::<Vec<_>>(),
        "Backup blob set does not match database references"
    );
    for (name, expected) in &manifest.blobs {
        anyhow::ensure!(valid_blob_name(name), "Backup has an invalid blob name");
        let path = directory.join("blobs").join(name);
        anyhow::ensure!(
            std::fs::symlink_metadata(&path)?.file_type().is_file(),
            "Backup blob must be a regular file"
        );
        anyhow::ensure!(&hash(&path)? == expected, "Backup blob checksum mismatch");
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
    for name in ["reviews.toml", "agent.toml", "human.toml"] {
        let source = parent.join(name);
        if source.is_file() {
            private_write(&output.join(name), &portable_client(&source, server.bind)?)?;
        }
    }
    for name in ["setup.json", "secret-references.toml"] {
        let source = parent.join(name);
        if source.is_file() {
            if name == "setup.json" {
                let _: super::first_run::SetupState =
                    serde_json::from_slice(&bounded_read(&source)?)?;
            } else {
                crate::secrets::SecretReferences::load(&source)?;
            }
            private_write(&output.join(name), &bounded_read(&source)?)?;
        }
    }
    let mut files = BTreeMap::new();
    for name in FILES {
        let path = output.join(name);
        if path.is_file() {
            files.insert((*name).to_owned(), hash(&path)?);
        }
    }
    let mut blobs = BTreeMap::new();
    let blob_names = Store::artifact_blob_names(&output.join("state.sqlite3"))?;
    if !blob_names.is_empty() {
        private_directory(&output.join("blobs"))?;
    }
    let source_blobs = Store::blob_directory_for(&server.database);
    for name in blob_names {
        let source = source_blobs.join(&name);
        let destination = output.join("blobs").join(&name);
        copy_private_file(&source, &destination)?;
        blobs.insert(name, hash(&destination)?);
    }
    let manifest = BackupManifest {
        version: 2,
        created_at: chrono::Utc::now().to_rfc3339(),
        binary_version: env!("CARGO_PKG_VERSION").into(),
        snapshot,
        files,
        blobs,
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
    anyhow::ensure!(
        manifest.snapshot.schema == connect_store::SCHEMA_VERSION,
        "Incompatible backup schema. Preserve it with its matching binary; Gate 6 requires explicit initialization of a new state path"
    );
    let mut server = ServerConfig::load(&backup.join("server.toml"))?;
    server.bind = bind;
    server.websocket_bind = websocket_bind;
    server.validate()?;
    server.database = "state.sqlite3".into();
    server.certificate = "certificate.der".into();
    server.private_key = "private-key.der".into();
    private_directory(directory).context("Restore destination must be a fresh directory")?;
    for name in manifest.files.keys().filter(|n| *n != "server.toml") {
        if name == "setup.json" {
            let mut setup: super::first_run::SetupState =
                serde_json::from_slice(&bounded_read(&backup.join(name))?)?;
            setup.bind = bind;
            private_write(&directory.join(name), &serde_json::to_vec_pretty(&setup)?)?;
        } else if name == "secret-references.toml" {
            crate::secrets::SecretReferences::load(&backup.join(name))?;
            private_write(&directory.join(name), &bounded_read(&backup.join(name))?)?;
        } else if name.ends_with(".toml") {
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
    if !manifest.blobs.is_empty() {
        private_directory(&Store::blob_directory_for(&directory.join("state.sqlite3")))?;
        for name in manifest.blobs.keys() {
            copy_private_file(
                &backup.join("blobs").join(name),
                &Store::blob_directory_for(&directory.join("state.sqlite3")).join(name),
            )?;
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

fn valid_blob_name(name: &str) -> bool {
    name.len() == 41
        && name.ends_with(".blob")
        && uuid::Uuid::parse_str(&name[..36]).is_ok()
        && !name.contains(['/', '\\', '\0'])
}

fn copy_private_file(source: &Path, destination: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(source)?;
    anyhow::ensure!(
        metadata.file_type().is_file(),
        "Source must be a regular file"
    );
    let mut input = std::fs::File::open(source)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut output = options.open(destination)?;
    std::io::copy(&mut input, &mut output)?;
    use std::io::Write as _;
    output.flush()?;
    output.sync_all()?;
    Ok(())
}

pub fn migrate(server_path: &Path, operator_path: &Path, backup: &Path) -> Result<()> {
    let manifest = verified_manifest(backup)?;
    anyhow::ensure!(
        manifest.snapshot.schema == connect_store::SCHEMA_VERSION,
        "Gate 6 does not convert historical state. Preserve a verified backup with its matching binary, then explicitly initialize a new state path"
    );
    let server = ServerConfig::load(server_path)?;
    let _lock = InstallationLock::acquire(&server.database)?;
    let operator = ClientConfig::load(operator_path)?;
    Store::authorize_recovery(&server.database, &operator.credential)?;
    let setup = setup_metadata::prepare(server_path, &server, &manifest)?;
    let snapshot = if manifest.snapshot.schema == connect_store::SCHEMA_VERSION
        && setup
            .as_ref()
            .is_some_and(|setup| setup.needs_sync(connect_store::SCHEMA_VERSION))
    {
        // Explicit recovery from DB commit followed by interrupted metadata
        // publication requires a NEW exact-current verified backup, never the
        // pre-upgrade snapshot or an implicitly adopted installation.
        let current = Store::inspect_snapshot(&server.database)?;
        anyhow::ensure!(
            current == manifest.snapshot,
            "Current verified backup does not match this installation"
        );
        current
    } else {
        Store::migrate(
            &server.database,
            &backup.join("state.sqlite3"),
            &manifest.snapshot,
        )?
    };
    if let Some(setup) = setup {
        Store::authorize_recovery(&server.database, &operator.credential)?;
        setup.publish(&snapshot).context(
            "Database migration committed but setup publication was not confirmed; preserve state and reconcile using a fresh exact-current verified backup"
        )?;
    }
    println!(
        "Migration complete schema={} installation={}",
        snapshot.schema, snapshot.installation_id
    );
    Ok(())
}
