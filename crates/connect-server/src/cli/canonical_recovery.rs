//! Stopped-state recovery for the signed canonical installation. A backup is
//! private credential material; only a completely verified directory is published.
use super::init::{private_directory, private_write};
use crate::{canonical::CanonicalServerConfig, installation::InstallationLock};
use anyhow::{ensure, Context, Result};
use connect_client::canonical::CanonicalClientConfig;
use connect_store::{Snapshot, Store};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
};

const FORMAT: &str = "connect/canonical-backup/1";
const MAX_FILES: usize = 512;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format: String,
    pub binary_version: String,
    pub created_at: String,
    pub snapshot: Snapshot,
    pub files: BTreeMap<String, String>,
    pub blobs: BTreeMap<String, String>,
}
struct Stage(PathBuf);
impl Stage {
    fn new(destination: &Path) -> Result<Self> {
        ensure!(
            std::fs::symlink_metadata(destination)
                .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
            "Recovery destination must not exist"
        );
        let parent = destination
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        ensure!(parent.is_dir(), "Recovery destination parent must exist");
        let path = parent.join(format!(
            ".connect-recovery-{}",
            uuid::Uuid::new_v4().simple()
        ));
        private_directory(&path)?;
        Ok(Self(path))
    }
    fn publish(self, destination: &Path) -> Result<()> {
        std::fs::File::open(&self.0)?.sync_all()?;
        #[cfg(unix)]
        rustix::fs::renameat_with(
            rustix::fs::CWD,
            &self.0,
            rustix::fs::CWD,
            destination,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .context("Publish recovery without replacing existing state")?;
        #[cfg(not(unix))]
        anyhow::bail!("Recovery requires the qualified Unix filesystem profile");
        std::fs::File::open(
            destination
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )?
        .sync_all()
        .context("Recovery published; parent sync failed")?;
        Ok(())
    }
}
impl Drop for Stage {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn hash(path: &Path) -> Result<String> {
    let mut input = std::fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 16384];
    loop {
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
    }
    Ok(hex::encode(digest.finalize()))
}
fn regular(path: &Path) -> Result<()> {
    ensure!(
        std::fs::symlink_metadata(path)?.is_file(),
        "Recovery material must be a regular file"
    );
    Ok(())
}
fn read(path: &Path) -> Result<Vec<u8>> {
    regular(path)?;
    ensure!(
        std::fs::metadata(path)?.len() <= 1024 * 1024,
        "Recovery configuration exceeds 1 MiB"
    );
    Ok(std::fs::read(path)?)
}
fn copy(source: &Path, destination: &Path) -> Result<()> {
    regular(source)?;
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut output = options.open(destination)?;
    std::io::copy(&mut std::fs::File::open(source)?, &mut output)?;
    output.flush()?;
    output.sync_all()?;
    Ok(())
}
fn numbered(name: &str, prefix: &str, suffix: &str) -> bool {
    name.strip_prefix(prefix)
        .and_then(|s| s.strip_suffix(suffix))
        .is_some_and(|n| n.len() == 4 && n.bytes().all(|b| b.is_ascii_digit()))
}
fn allowed(name: &str) -> bool {
    matches!(name, "server.toml" | "operator.toml" | "state.sqlite3")
        || numbered(name, "asset-", ".bin")
        || numbered(name, "peer-", ".toml")
}
fn blob_name(name: &str) -> bool {
    name.len() == 41
        && name.ends_with(".blob")
        && name.is_ascii()
        && uuid::Uuid::parse_str(&name[..36]).is_ok()
}
fn portable_reference(path: &Path, files: &BTreeMap<String, String>) -> Result<()> {
    let name = path.to_str().context("Invalid recovery reference")?;
    ensure!(
        allowed(name) && files.contains_key(name),
        "Recovery configuration references material outside its manifest"
    );
    Ok(())
}
fn check_client(raw: &[u8], files: &BTreeMap<String, String>) -> Result<()> {
    let config: CanonicalClientConfig = toml::from_str(std::str::from_utf8(raw)?)
        .map_err(|_| anyhow::anyhow!("Invalid canonical recovery client"))?;
    for path in [
        &config.trust_certificate,
        &config.signing_private_key,
        &config.server_public_key,
    ] {
        portable_reference(path, files)?;
    }
    Ok(())
}
/// Validate every path and byte before any restored installation is published.
pub fn verified_manifest(directory: &Path) -> Result<Manifest> {
    ensure!(
        std::fs::symlink_metadata(directory)?.is_dir(),
        "Backup must be a directory, not a symlink"
    );
    let manifest: Manifest = serde_json::from_slice(&read(&directory.join("manifest.json"))?)
        .map_err(|_| anyhow::anyhow!("Invalid canonical backup manifest"))?;
    ensure!(
        manifest.format == FORMAT,
        "Unsupported canonical backup format"
    );
    ensure!(manifest.snapshot.schema == connect_store::SCHEMA_VERSION, "Unsupported backup schema; preserve it with its matching binary and explicitly initialize new state");
    ensure!(
        manifest.files.len() <= MAX_FILES,
        "Recovery file capacity exceeded"
    );
    for required in ["server.toml", "operator.toml", "state.sqlite3"] {
        ensure!(
            manifest.files.contains_key(required),
            "Incomplete canonical backup"
        );
    }
    for (name, expected) in &manifest.files {
        ensure!(allowed(name), "Unsafe recovery path");
        let path = directory.join(name);
        regular(&path)?;
        ensure!(hash(&path)? == *expected, "Recovery checksum mismatch");
        if name == "operator.toml" || numbered(name, "peer-", ".toml") {
            check_client(&read(&path)?, &manifest.files)?;
        }
    }
    let server: CanonicalServerConfig =
        toml::from_str(std::str::from_utf8(&read(&directory.join("server.toml"))?)?)
            .map_err(|_| anyhow::anyhow!("Invalid canonical recovery server"))?;
    ensure!(
        server.database == Path::new("state.sqlite3"),
        "Invalid recovery database path"
    );
    for path in [
        &server.certificate,
        &server.private_key,
        &server.signing_private_key,
    ] {
        portable_reference(path, &manifest.files)?;
    }
    if let Some(config) = &server.public_vapi {
        portable_reference(&config.secret_references, &manifest.files)?;
        crate::secrets::SecretReferences::load(&directory.join(&config.secret_references))?;
        for target in &config.targets {
            portable_reference(&target.native_peer, &manifest.files)?;
            check_client(
                &read(&directory.join(&target.native_peer))?,
                &manifest.files,
            )?;
        }
    }
    let database = directory.join("state.sqlite3");
    ensure!(
        Store::inspect_snapshot(&database)? == manifest.snapshot,
        "Backup snapshot differs from manifest"
    );
    ensure!(
        Store::artifact_blob_names(&database)?
            == manifest.blobs.keys().cloned().collect::<Vec<_>>(),
        "Backup blob references differ from snapshot"
    );
    if !manifest.blobs.is_empty() {
        ensure!(
            std::fs::symlink_metadata(directory.join("blobs"))?.is_dir(),
            "Invalid recovery blob directory"
        );
    }
    for (name, expected) in &manifest.blobs {
        ensure!(blob_name(name), "Unsafe blob path");
        let path = directory.join("blobs").join(name);
        regular(&path)?;
        ensure!(hash(&path)? == *expected, "Recovery blob checksum mismatch");
    }
    Ok(manifest)
}
struct Assets<'a> {
    root: &'a Path,
    files: BTreeMap<String, String>,
    next: usize,
}
impl Assets<'_> {
    fn write(&mut self, name: &str, bytes: &[u8]) -> Result<PathBuf> {
        ensure!(
            self.files.len() < MAX_FILES,
            "Recovery file capacity exceeded"
        );
        private_write(&self.root.join(name), bytes)?;
        self.files.insert(name.into(), hash(&self.root.join(name))?);
        Ok(name.into())
    }
    fn import(&mut self, path: &Path) -> Result<PathBuf> {
        let name = format!("asset-{:04}.bin", self.next);
        self.next += 1;
        self.write(&name, &read(path)?)
    }
    fn client(&mut self, path: &Path, name: &str) -> Result<PathBuf> {
        regular(path)?;
        let mut config = CanonicalClientConfig::load(path)?;
        config.trust_certificate = self.import(&config.trust_certificate)?;
        config.signing_private_key = self.import(&config.signing_private_key)?;
        config.server_public_key = self.import(&config.server_public_key)?;
        self.write(name, toml::to_string_pretty(&config)?.as_bytes())
    }
}
/// Offline only: exclude all service/maintenance writers before taking the
/// database/blob/configuration set. No process environment or secret value export.
pub fn backup(server_path: &Path, operator_path: &Path, output: &Path) -> Result<()> {
    let mut server = CanonicalServerConfig::load(server_path)?;
    regular(operator_path)?;
    let operator = CanonicalClientConfig::load(operator_path)?;
    let _lock =
        InstallationLock::acquire(&server.database).context("Stop the server before backup")?;
    Store::authorize_recovery(&server.database, &operator.credential)?;
    let stage = Stage::new(output)?;
    let snapshot = Store::backup_database(&server.database, &stage.0.join("state.sqlite3"))?;
    let mut assets = Assets {
        root: &stage.0,
        files: BTreeMap::new(),
        next: 0,
    };
    assets.files.insert(
        "state.sqlite3".into(),
        hash(&stage.0.join("state.sqlite3"))?,
    );
    server.certificate = assets.import(&server.certificate)?;
    server.private_key = assets.import(&server.private_key)?;
    server.signing_private_key = assets.import(&server.signing_private_key)?;
    assets.client(operator_path, "operator.toml")?;
    if let Some(config) = &mut server.public_vapi {
        crate::secrets::SecretReferences::load(&config.secret_references)?;
        config.secret_references = assets.import(&config.secret_references)?;
        for (i, target) in config.targets.iter_mut().enumerate() {
            target.native_peer =
                assets.client(&target.native_peer, &format!("peer-{i:04}.toml"))?;
        }
    }
    let source_blobs = Store::blob_directory_for(&server.database);
    server.database = "state.sqlite3".into();
    assets.write("server.toml", toml::to_string_pretty(&server)?.as_bytes())?;
    let mut blobs = BTreeMap::new();
    for name in Store::artifact_blob_names(&stage.0.join("state.sqlite3"))? {
        ensure!(blob_name(&name), "Invalid snapshot blob name");
        if blobs.is_empty() {
            private_directory(&stage.0.join("blobs"))?;
        }
        let destination = stage.0.join("blobs").join(&name);
        copy(&source_blobs.join(&name), &destination)?;
        blobs.insert(name, hash(&destination)?);
    }
    if !blobs.is_empty() {
        std::fs::File::open(stage.0.join("blobs"))?.sync_all()?;
    }
    let manifest = Manifest {
        format: FORMAT.into(),
        binary_version: env!("CARGO_PKG_VERSION").into(),
        created_at: chrono::Utc::now().to_rfc3339(),
        snapshot,
        files: assets.files,
        blobs,
    };
    private_write(
        &stage.0.join("manifest.json"),
        &serde_json::to_vec_pretty(&manifest)?,
    )?;
    verified_manifest(&stage.0)?;
    stage.publish(output)?;
    println!("Verified protected canonical backup: {}", output.display());
    Ok(())
}
/// Preserve listener/profile settings and enrolled keys. New bindings require an
/// explicit operator configuration edit; no silent loopback/profile conversion.
pub fn restore(backup: &Path, directory: &Path) -> Result<()> {
    let manifest = verified_manifest(backup)?;
    let stage = Stage::new(directory)?;
    for name in manifest.files.keys() {
        let destination = stage.0.join(name);
        if name == "state.sqlite3" {
            ensure!(
                Store::backup_database(&backup.join(name), &destination)? == manifest.snapshot,
                "Restored database mismatch"
            );
        } else {
            copy(&backup.join(name), &destination)?;
        }
    }
    if !manifest.blobs.is_empty() {
        let destination = Store::blob_directory_for(&stage.0.join("state.sqlite3"));
        private_directory(&destination)?;
        for name in manifest.blobs.keys() {
            copy(&backup.join("blobs").join(name), &destination.join(name))?;
        }
    }
    ensure!(
        Store::inspect_snapshot(&stage.0.join("state.sqlite3"))? == manifest.snapshot,
        "Restored snapshot mismatch"
    );
    if !manifest.blobs.is_empty() {
        std::fs::File::open(Store::blob_directory_for(&stage.0.join("state.sqlite3")))?
            .sync_all()?;
    }
    // Rehash copied configuration to catch source replacement during restore.
    for (name, expected) in &manifest.files {
        if name != "state.sqlite3" {
            ensure!(
                hash(&stage.0.join(name))? == *expected,
                "Recovery material changed during restore"
            );
        }
    }
    for (name, expected) in &manifest.blobs {
        ensure!(
            hash(&Store::blob_directory_for(&stage.0.join("state.sqlite3")).join(name))?
                == *expected,
            "Recovery blob changed during restore"
        );
    }
    stage.publish(directory)?;
    println!(
        "Restored protected canonical installation: {} schema={}",
        directory.display(),
        manifest.snapshot.schema
    );
    Ok(())
}
