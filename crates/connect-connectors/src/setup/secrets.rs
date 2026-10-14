//! Migration is read-back confirmed before a legacy secret leaves its checkpoint.
use super::{write_state, SetupState};
use anyhow::{ensure, Context, Result};
use fs2::FileExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

/// Trusted server composition owns storage, current operator authorization and
/// immutable logical secret IDs. Values must never be logged or serialized.
pub trait SetupSecretStore: Send + Sync {
    /// Recheck current setup authority before reads, reuse and publication.
    fn authorize(&self) -> Result<()>;
    /// Metadata reference bound to this exact installation/operation; no secret read.
    fn reference(&self, operation_id: &str) -> Result<String>;
    /// Reuse this exact intent's existing secret; a supplied legacy value must match.
    fn ensure(&self, operation_id: &str, legacy: Option<&str>) -> Result<String>;
    /// Read an admitted reference without substituting or rotating its value.
    fn read(&self, reference: &str) -> Result<String>;
}
pub(super) struct NoSecrets;
impl SetupSecretStore for NoSecrets {
    fn authorize(&self) -> Result<()> {
        Ok(())
    }
    fn reference(&self, _: &str) -> Result<String> {
        anyhow::bail!("SIP vault access is required")
    }
    fn ensure(&self, _: &str, _: Option<&str>) -> Result<String> {
        anyhow::bail!("SIP setup requires the authenticated server vault; use the server CLI or inject SetupSecretStore")
    }
    fn read(&self, _: &str) -> Result<String> {
        anyhow::bail!("SIP vault access is required")
    }
}
/// Metadata-only migration receipt; the password is never included.
#[derive(Debug, Serialize)]
pub struct SecretMigration {
    /// Saved logical setup intent, absent when no SIP secret is needed.
    pub operation_id: Option<String>,
    /// Protected storage reference, containing metadata only.
    pub secret_reference: Option<String>,
    /// Whether the protected checkpoint was atomically rewritten.
    pub changed: bool,
}
pub(super) fn load_private(path: &Path) -> Result<SetupState> {
    let metadata = fs::symlink_metadata(path).context("Read provider checkpoint")?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink() && metadata.len() <= 1024 * 1024,
        "Provider checkpoint must be a bounded regular private file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            metadata.permissions().mode() & 0o077 == 0,
            "Provider checkpoint needs mode 0600 before secret migration"
        );
    }
    serde_json::from_slice(&fs::read(path)?)
        .map_err(|_| anyhow::anyhow!("Invalid provider checkpoint"))
}
pub(super) fn protect(state: &mut SetupState, store: &dyn SetupSecretStore) -> Result<bool> {
    store.authorize()?;
    if !state.plan.create_sip {
        let changed = state.sip_password.take().is_some();
        return Ok(changed);
    }
    let operation_id = state.operation_id.clone().unwrap_or_else(|| {
        let identity = format!(
            "connect-legacy-sip/1:{}:{}",
            state.name,
            state.sip_connection_id.as_deref().unwrap_or("pending")
        );
        format!("{:x}", Sha256::digest(identity.as_bytes()))[..32].to_owned()
    });
    ensure!(
        operation_id.len() == 32 && operation_id.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid setup logical operation ID"
    );
    let previous = state.sip_password.as_deref().filter(|s| !s.is_empty());
    ensure!(previous.is_some() || state.sip_password_ref.is_some() || (state.sip_connection_id.is_none() && state.pending.as_deref()!=Some("sip")),"Existing or uncertain SIP resource needs its original credential; no replacement secret was generated");
    if let Some(reference) = &state.sip_password_ref {
        ensure!(
            reference == &store.reference(&operation_id)?,
            "SIP checkpoint reference does not belong to its logical operation"
        );
    }
    let reference = if state.sip_password_ref.is_some() && previous.is_none() {
        state
            .sip_password_ref
            .clone()
            .context("Missing vault reference")?
    } else {
        store.ensure(&operation_id, previous)?
    };
    let value = store.read(&reference)?;
    ensure!(
        !value.is_empty() && value.len() <= 256,
        "SIP vault secret is empty or oversized"
    );
    if let Some(previous) = previous {
        ensure!(
            value == previous,
            "Vault read-back does not match the legacy SIP secret; original checkpoint preserved"
        );
    }
    let fingerprint = format!("{:x}", Sha256::digest(value.as_bytes()));
    if let Some(expected) = &state.sip_password_fingerprint {
        ensure!(expected==&fingerprint,"SIP vault value differs from its protected checkpoint; do not rotate provider credentials through recovery");
    }
    let changed = state.operation_id.as_ref() != Some(&operation_id)
        || state.sip_password_ref.as_ref() != Some(&reference)
        || state.sip_password.is_some()
        || state.sip_password_fingerprint.is_none();
    state.operation_id = Some(operation_id);
    state.sip_password_ref = Some(reference);
    state.sip_password = None;
    state.sip_password_fingerprint = Some(fingerprint);
    Ok(changed)
}
fn lock(path: &Path) -> Result<fs::File> {
    let lock_path = path.with_extension("lock");
    if let Ok(metadata) = fs::symlink_metadata(&lock_path) {
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "Provider setup lock must be a regular private file"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            ensure!(
                metadata.permissions().mode() & 0o077 == 0,
                "Provider setup lock must be private"
            );
        }
    }
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options.open(lock_path)?;
    lock.try_lock_exclusive()
        .context("Another provider setup owns this checkpoint")?;
    Ok(lock)
}
/// Idempotent local migration; no provider requests, password rotation or deletion.
pub fn migrate_secrets(path: &Path, store: &dyn SetupSecretStore) -> Result<SecretMigration> {
    store.authorize()?;
    let _lock = lock(path)?;
    let mut state = load_private(path)?;
    let changed = protect(&mut state, store)?;
    if changed {
        store.authorize()?;
        write_state(path, &state)?;
    }
    Ok(SecretMigration {
        operation_id: state.operation_id,
        secret_reference: state.sip_password_ref,
        changed,
    })
}
/// Import the original SIP secret into a missing vault entry after restore. Existing
/// entries must match; this cannot rotate a provider password or substitute an ID.
pub fn restore_secret(
    path: &Path,
    store: &dyn SetupSecretStore,
    value: &str,
) -> Result<SecretMigration> {
    store.authorize()?;
    let _lock = lock(path)?;
    let state = load_private(path)?;
    ensure!(
        state.sip_password.is_none() && state.plan.create_sip,
        "Migrate the checkpoint before restoring a vault reference"
    );
    let operation_id = state.operation_id.context("Setup operation ID missing")?;
    let expected = state
        .sip_password_ref
        .context("Setup secret reference missing")?;
    ensure!(
        state.sip_password_fingerprint.as_deref()
            == Some(format!("{:x}", Sha256::digest(value.as_bytes())).as_str()),
        "Imported SIP password does not match the protected checkpoint; vault unchanged"
    );
    ensure!(
        store.reference(&operation_id)? == expected,
        "Restore reference does not belong to this setup operation"
    );
    let reference = store.ensure(&operation_id, Some(value))?;
    ensure!(
        reference == expected && store.read(&reference)? == value,
        "Vault restore differs from the recorded reference; checkpoint unchanged"
    );
    store.authorize()?;
    Ok(SecretMigration {
        operation_id: Some(operation_id),
        secret_reference: Some(reference),
        changed: false,
    })
}
