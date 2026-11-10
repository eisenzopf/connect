//! Presence-only lifetime observation. No media grant can be extracted from it.
use super::*;
use crate::media_authorization::{Handle, Prepared, SnapshotFacts};
fn same_physical_lineage(
    current: [&str; 8],
    original: [&str; 8],
    current_generation: u64,
    original_generation: u64,
    current_membership: u64,
    original_membership: u64,
    current_epoch: u64,
    original_epoch: u64,
) -> bool {
    current == original
        && current_generation == original_generation
        && current_membership == original_membership
        && current_epoch >= original_epoch
}
fn identity(origin: &NativeMediaOrigin) -> [&str; 8] {
    [
        origin.offer_record_id(),
        origin.connection_id(),
        origin.runtime_connection_id(),
        origin.conversation_id(),
        origin.session_id(),
        origin.participant_id(),
        &origin.endpoint_id,
        &origin.binding.device.id,
    ]
}
/// Scheduling metadata only. Cannot admit media or revive an old IO epoch.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct NativeConnectionLifetimeStamp(crate::media_authorization::Stamp);
#[derive(Clone)]
pub struct NativeConnectionLifetimeSnapshot {
    origin: Arc<NativeMediaOrigin>,
    // Cached scheduling metadata owns no installation custody after retirement.
    // The revocable LifetimeFacts publication keeps its actual Read/owner.
    owner: Weak<InstallationOwnership>,
    handle: Handle<LifetimeFacts>,
}
struct LifetimeFacts {
    owner: Arc<InstallationOwnership>,
    expires_at: i64,
}
impl SnapshotFacts for LifetimeFacts {
    fn current(&self) -> bool {
        self.owner.current() && crate::now() < self.expires_at
    }
}
impl NativeConnectionLifetimeSnapshot {
    /// Same original physical lineage may use a verified credential successor
    /// for lifetime only. This lends no queued frame, media or signal authority.
    pub fn observe_original(&self, original: &NativeMediaOrigin) -> Result<bool> {
        self.observe_original_generation(original).map(|_| true)
    }
    /// Atomically observe the same validated lifetime publication and its state
    /// generation. The stamp schedules reconciliation, never grants authority.
    pub fn observe_original_generation(
        &self,
        original: &NativeMediaOrigin,
    ) -> Result<NativeConnectionLifetimeStamp> {
        let current = &self.origin;
        let owner = self.owner.upgrade().ok_or_else(Error::forbidden)?;
        if !owner.current()
            || original
                .owner
                .upgrade()
                .is_none_or(|original_owner| !Arc::ptr_eq(&original_owner, &owner))
            || !same_physical_lineage(
                identity(current),
                identity(original),
                current.generation(),
                original.generation(),
                current.membership_epoch(),
                original.membership_epoch(),
                current.credential_epoch,
                original.credential_epoch,
            )
            || current.binding.device != original.binding.device
            || (current.credential_epoch == original.credential_epoch
                && (current.binding.credentials != original.binding.credentials
                    || current.binding.key_id != original.binding.key_id
                    || current.public_key != original.public_key))
            || crate::now() >= original.expires_at
        {
            return Err(Error::forbidden());
        }
        let observation = self.handle.observe()?;
        if !observation.current() {
            return Err(Error::new(
                "authority-busy",
                "Original lifetime observation is refreshing",
            ));
        }
        Ok(NativeConnectionLifetimeStamp(observation.stamp()))
    }
}
impl Store {
    /// Registration is control-only and never performs the factory SQL inline.
    /// Existing Runtime owns the sole Registry watcher and its shutdown/join.
    pub fn try_register_native_connection_lifetime_snapshot(
        self: &Arc<Self>,
        origin: Arc<NativeMediaOrigin>,
    ) -> Result<NativeConnectionLifetimeSnapshot> {
        let owner = current_owner(self, &origin)?;
        let weak = Arc::downgrade(self);
        let captured = origin.clone();
        let handle = self
            .media_authorization_registry()
            .try_register_with_hints(&origin.snapshot_hints, move || {
                let store = weak.upgrade().ok_or_else(Error::forbidden)?;
                let owner = current_owner(&store, &captured)?;
                // Reuse the original Presence purpose and complete fenced
                // lifetime resolver. A media permission is never substituted.
                let (expires_at, held) = snapshot::cached(
                    &store,
                    &snapshot::metadata(&captured)?.lifetime,
                    None,
                    |c| {
                        let (_, scopes, expiry) =
                            facts_for(&store, c, &captured, ReadPurpose::Presence)?;
                        Ok((expiry, scopes))
                    },
                    |_| Ok(()),
                )?;
                let expires_at = expires_at.min(captured.expires_at);
                let remaining = (chrono::DateTime::from_timestamp(expires_at, 0)
                    .ok_or_else(Error::forbidden)?
                    - chrono::Utc::now())
                .to_std()
                .map_err(|_| Error::forbidden())?;
                let until = std::time::Instant::now()
                    .checked_add(remaining)
                    .ok_or_else(Error::forbidden)?;
                Prepared::new(Arc::new(LifetimeFacts { owner, expires_at }), held, until)
            })?;
        Ok(NativeConnectionLifetimeSnapshot {
            origin,
            owner: Arc::downgrade(&owner),
            handle,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lifetime_successor_is_same_original_physical_lineage_not_new_generation_or_foreign_owner() {
        let original = [
            "offer",
            "connection",
            "runtime",
            "conversation",
            "session",
            "participant",
            "endpoint",
            "device",
        ];
        assert!(same_physical_lineage(original, original, 1, 1, 2, 2, 4, 3));
        assert!(!same_physical_lineage(original, original, 2, 1, 2, 2, 4, 3));
        assert!(!same_physical_lineage(original, original, 1, 1, 3, 2, 4, 3));
        assert!(!same_physical_lineage(original, original, 1, 1, 2, 2, 2, 3));
        for changed in 0..original.len() {
            let mut foreign = original;
            foreign[changed] = "foreign-original";
            assert!(!same_physical_lineage(foreign, original, 1, 1, 2, 2, 4, 3));
        }
    }
}
