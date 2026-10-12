//! Observe regular host metadata after public task-owner authorization.
use super::{codex, state::State};
use crate::cli::task_access::{
    codex_host::diagnostics::{self, HostObservation, HostState},
    read_bounded,
};
use std::{io, path::Path};

/// Select regular-agent sampling only for its explicit installation format.
/// None leaves the original narrow-task sampler responsible for that directory.
/// The caller must recheck remote task authority after this local observation.
pub fn inspect(directory: Option<&Path>, owner_id: &str) -> Option<HostObservation> {
    let directory = directory?;
    let manifest = read_bounded(&directory.join("install.json")).ok()?;
    let manifest: serde_json::Value = serde_json::from_slice(&manifest).ok()?;
    if manifest["profile"] != "connect-codex-agent-install/1" {
        return None;
    }
    Some(sample(directory, owner_id))
}

fn sample(directory: &Path, owner_id: &str) -> HostObservation {
    let Ok(installation) = codex::load(directory) else {
        return HostObservation::empty(HostState::InvalidInstallation);
    };
    if installation.client.identity_id != owner_id {
        return HostObservation::empty(HostState::BindingMismatch);
    }
    if installation.credential_expires_at <= chrono::Utc::now().timestamp() {
        return HostObservation::empty(HostState::CredentialExpired);
    }
    let initial = match State::load(directory) {
        Ok(value) => value,
        Err(_) => return HostObservation::empty(HostState::InvalidState),
    };
    let lock_path = directory.join("host.lock");
    let lock = match std::fs::symlink_metadata(&lock_path) {
        Ok(_) => match diagnostics::lock(&lock_path) {
            Ok(value) => Some(value),
            Err(_) => return HostObservation::empty(HostState::LockUnavailable),
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(_) => return HostObservation::empty(HostState::LockUnavailable),
    };
    let latest = match State::load(directory) {
        Ok(value) => value,
        Err(_) => return HostObservation::empty(HostState::InvalidState),
    };
    if initial != latest {
        return HostObservation::empty(HostState::ChangedDuringSample);
    }
    let active = lock == Some(true);
    let Some(saved) = latest else {
        return HostObservation::empty(if active {
            HostState::TurnActive
        } else {
            HostState::NotStarted
        });
    };
    if saved.installation != installation.digest
        || saved.identity_id != owner_id
        || saved.credential_id != installation.client.credential_id
    {
        return HostObservation::empty(HostState::BindingMismatch);
    }
    if lock.is_none() {
        return HostObservation::empty(HostState::LockUnavailable);
    }
    HostObservation {
        state: if active {
            HostState::TurnActive
        } else if saved.session_id.is_none() {
            HostState::MissingSession
        } else if saved.completed {
            HostState::TurnComplete
        } else {
            HostState::Interrupted
        },
        session_id: saved.session_id,
        run_id: Some(saved.run_id),
        attempts: Some(saved.attempts),
    }
}
