//! First-run metadata follows verified database migration without exporting secrets.
use super::{bounded_read, hash, BackupManifest};
use crate::{cli::first_run::SetupState, config::ServerConfig};
use anyhow::{ensure, Result};
use connect_store::Snapshot;
use std::path::{Path, PathBuf};

pub(super) struct Prepared {
    path: PathBuf,
    original_hash: String,
    installation_id: String,
    setup: SetupState,
}
impl Prepared {
    pub(super) fn needs_sync(&self, schema: u32) -> bool {
        self.setup.schema < schema
    }
    pub(super) fn publish(mut self, snapshot: &Snapshot) -> Result<()> {
        ensure!(
            snapshot.installation_id == self.installation_id
                && snapshot.schema == connect_store::SCHEMA_VERSION,
            "Migrated database differs from the verified setup installation"
        );
        require_private_metadata(&self.path)?;
        ensure!(hash(&self.path)? == self.original_hash,
            "Setup metadata changed during migration; preserve it and create a fresh verified backup");
        self.setup.schema = snapshot.schema;
        crate::cli::init::private_replace(&self.path, &serde_json::to_vec_pretty(&self.setup)?)
    }
}
pub(super) fn prepare(
    server_path: &Path,
    server: &ServerConfig,
    manifest: &BackupManifest,
) -> Result<Option<Prepared>> {
    let path = server_path
        .parent()
        .unwrap_or(Path::new("."))
        .join("setup.json");
    let Some(expected) = manifest.files.get("setup.json") else {
        ensure!(
            matches!(std::fs::symlink_metadata(&path), Err(error) if error.kind() == std::io::ErrorKind::NotFound),
            "Verified backup omits existing setup metadata"
        );
        return Ok(None);
    };
    require_private_metadata(&path)?;
    let original_hash = hash(&path)?;
    ensure!(
        &original_hash == expected,
        "Setup metadata differs from the verified backup; create a fresh backup before migration"
    );
    let setup: SetupState = serde_json::from_slice(&bounded_read(&path)?)?;
    let directory = path.parent().unwrap_or(Path::new(".")).canonicalize()?;
    ensure!(
        server_path.file_name() == Some(std::ffi::OsStr::new("server.toml"))
            && server.database.canonicalize()? == directory.join("state.sqlite3")
            && server.certificate.canonicalize()? == directory.join("certificate.der")
            && server.private_key.canonicalize()? == directory.join("private-key.der"),
        "Setup paths differ from the verified installation"
    );
    ensure!(
        setup.profile == "connect-first-run/1"
            && (17..=manifest.snapshot.schema).contains(&setup.schema)
            && setup.bind == server.bind
            && setup.server_config == "server.toml"
            && setup.human_config == "human.toml"
            && setup.agent_config == "agent.toml"
            && setup.secret_references == "secret-references.toml"
            && setup.provider_mode == "none"
            && !setup.tasks_required,
        "Unsupported or foreign first-run metadata; preserve the original installation"
    );
    Ok(Some(Prepared {
        path,
        original_hash,
        installation_id: manifest.snapshot.installation_id.clone(),
        setup,
    }))
}

fn require_private_metadata(path: &Path) -> Result<()> {
    ensure!(
        std::fs::symlink_metadata(path)?.file_type().is_file(),
        "Setup metadata must be an ordinary protected file"
    );
    let directory = path.parent().unwrap_or(Path::new("."));
    ensure!(
        std::fs::symlink_metadata(directory)?.file_type().is_dir(),
        "Setup metadata directory must be ordinary and protected"
    );
    crate::cli::task_access::require_private(path)?;
    crate::cli::task_access::require_private(directory)
}
