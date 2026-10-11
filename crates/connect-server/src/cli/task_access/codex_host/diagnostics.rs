//! Protected metadata inspection; neither a lock nor a session grants authority.
use super::{install, state};
use fs2::FileExt;
use serde::Serialize;
use std::{fs::File, io, path::Path};

/// Local adapter observation. This never attests model progress or delivery.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum HostState {
    Unconfigured,
    InvalidInstallation,
    BindingMismatch,
    InvalidState,
    LockUnavailable,
    ChangedDuringSample,
    NotStarted,
    TurnActive,
    TurnComplete,
    Interrupted,
    MissingSession,
}

/// Only validated correlation metadata; no prompts, tokens, paths or raw errors.
#[derive(Debug, Serialize)]
pub struct HostObservation {
    pub state: HostState,
    pub session_id: Option<String>,
    pub run_id: Option<String>,
    pub attempts: Option<u32>,
}
impl HostObservation {
    /// Operational projection of the supervised launcher's retained state.
    /// A held lock remains uncertain; a completed turn still needs explicit resume.
    pub fn readiness(&self) -> connect_protocol::ComponentReadiness {
        use connect_client::task_diagnostics::WorkerCondition as W;
        let condition = match self.state {
            HostState::Unconfigured => W::Unconfigured,
            HostState::NotStarted => W::NotStarted,
            HostState::TurnActive => W::OwnershipObserved,
            HostState::TurnComplete => W::ExplicitResumeRequired,
            HostState::Interrupted | HostState::MissingSession => W::RecoveryRequired,
            HostState::ChangedDuringSample => W::ChangedDuringSample,
            HostState::InvalidInstallation
            | HostState::BindingMismatch
            | HostState::InvalidState
            | HostState::LockUnavailable => W::InvalidMetadata,
        };
        condition.component("codex-host")
    }
    fn empty(state: HostState) -> Self {
        Self {
            state,
            session_id: None,
            run_id: None,
            attempts: None,
        }
    }
}

/// Inspect an explicitly selected installation for an already authenticated task.
/// The caller must recheck remote authority after sampling local metadata. Files
/// are read with existing private/bounded checks; no log/session content is read,
/// lock file created, grant refreshed or replacement session started.
pub fn inspect(directory: Option<&Path>, task_id: &str) -> HostObservation {
    inspect_with_lock_probe(directory, task_id, lock)
}

fn inspect_with_lock_probe(
    directory: Option<&Path>,
    task_id: &str,
    probe: impl FnOnce(&Path) -> anyhow::Result<bool>,
) -> HostObservation {
    let Some(directory) = directory else {
        return HostObservation::empty(HostState::Unconfigured);
    };
    let Ok(installation) = install::load(directory) else {
        return HostObservation::empty(HostState::InvalidInstallation);
    };
    let Some(grant) = &installation.file.grant else {
        return HostObservation::empty(HostState::InvalidInstallation);
    };
    if grant.grant.task_id != task_id {
        return HostObservation::empty(HostState::BindingMismatch);
    }
    let initial = match state::State::load(directory) {
        Ok(value) => value,
        Err(_) => return HostObservation::empty(HostState::InvalidState),
    };
    let path = directory.join("host.lock");
    let lock = match std::fs::symlink_metadata(&path) {
        Ok(_) => match probe(&path) {
            Ok(value) => Some(value),
            Err(_) => return HostObservation::empty(HostState::LockUnavailable),
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(_) => return HostObservation::empty(HostState::LockUnavailable),
    };
    let active = lock == Some(true);
    let latest = match state::State::load(directory) {
        Ok(value) => value,
        Err(_) => return HostObservation::empty(HostState::InvalidState),
    };
    if initial != latest {
        return HostObservation::empty(HostState::ChangedDuringSample);
    }
    let saved = match latest {
        Some(value) => value,
        None => {
            return HostObservation::empty(if active {
                HostState::TurnActive
            } else {
                HostState::NotStarted
            })
        }
    };
    if saved.installation != installation.digest
        || saved.task_id != task_id
        || saved.grant_id != installation.original_grant_id
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

// Release the probe immediately; diagnostics must not hold a startup lock while
// reading state. WouldBlock observes ownership at this instant, never a PID.
fn lock(path: &Path) -> anyhow::Result<bool> {
    super::super::require_private(path)?;
    super::super::read_bounded(path)?;
    let file = File::open(path)?;
    match FileExt::try_lock_shared(&file) {
        Ok(()) => {
            FileExt::unlock(&file)?;
            Ok(false)
        }
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(true),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
#[path = "diagnostics_tests.rs"]
mod tests;
