//! Original provider media facts, refreshed only by the existing Store watcher.
use super::*;
use crate::media_authorization::{Epoch, Handle, Lease, Prepared, SnapshotFacts, Stamp};
impl SnapshotFacts for PrivateProviderMediaRead {
    fn current(&self) -> bool {
        PrivateProviderMediaRead::current(self)
    }
}
#[derive(Clone)]
pub struct PrivateProviderMediaSnapshot {
    handle: Handle<PrivateProviderMediaRead>,
}
/// Actual IO authority. Keep through the original send and drop before idle,
/// queued buffering, or historical recording. This pins its original Read fences.
pub struct PrivateProviderMediaLease {
    live: Lease<PrivateProviderMediaRead>,
}
/// Queue metadata only, with no authority/file fence. Revocation is permanent;
/// even a later valid publication for this same context cannot revive this epoch.
#[derive(Clone)]
pub struct PrivateProviderMediaEpoch {
    stamp: Stamp,
    epoch: Epoch,
}
impl PrivateProviderMediaEpoch {
    pub fn current(&self) -> bool {
        self.epoch.current()
    }
    pub fn same_generation(&self, other: &Self) -> bool {
        self.stamp == other.stamp
    }
}
impl PrivateProviderMediaLease {
    pub fn current(&self) -> bool {
        self.live.current()
    }
    pub fn expires_at(&self) -> i64 {
        self.live.facts().expires_at()
    }
    pub fn epoch(&self) -> PrivateProviderMediaEpoch {
        PrivateProviderMediaEpoch {
            stamp: self.live.stamp(),
            epoch: self.live.packet_epoch(),
        }
    }
}
impl PrivateProviderMediaSnapshot {
    pub fn admit(&self) -> Result<PrivateProviderMediaLease> {
        // observe preserves definitive original policy refusal vs refreshing.
        Ok(PrivateProviderMediaLease {
            live: self.handle.observe()?,
        })
    }
    pub fn admit_original(
        &self,
        original: &PrivateProviderMediaEpoch,
    ) -> Result<PrivateProviderMediaLease> {
        if !original.current() {
            return Err(Error::new(
                "stale-media-epoch",
                "Original provider output epoch ended",
            ));
        }
        Ok(PrivateProviderMediaLease {
            live: self.handle.admit_original(original.stamp)?,
        })
    }
    pub fn epoch(&self) -> Result<PrivateProviderMediaEpoch> {
        // Drop the complete lease before returning scheduling metadata.
        let lease = self.admit()?;
        Ok(lease.epoch())
    }
}
impl Store {
    /// Control-only enrollment. No inline SQL and no per-provider worker: the
    /// existing service Runtime owns the sole Store watcher and its natural join.
    /// Its coordinated-writer boundary must already be enabled for admission.
    pub fn register_private_provider_media_snapshot(
        self: &Arc<Self>,
        context: PrivateProviderContext,
        applied: Arc<PrivateProviderApplied>,
    ) -> Result<PrivateProviderMediaSnapshot> {
        let weak = Arc::downgrade(self);
        let hints = [AuthorityScope::conversation(
            &context.selection.origin.request.conversation_id,
            AuthorityMode::Read,
        )?];
        let handle =
            self.media_authorization_registry()
                .register_with_hints(&hints, move || {
                    let store = weak.upgrade().ok_or_else(Error::forbidden)?;
                    let facts =
                        Arc::new(store.try_private_provider_media_read(&context, &applied)?);
                    let remaining = (chrono::DateTime::from_timestamp(facts.expires_at, 0)
                        .ok_or_else(Error::forbidden)?
                        - chrono::Utc::now())
                    .to_std()
                    .map_err(|_| Error::forbidden())?;
                    // Keep the original bounded read expiry. Expiry is a real invalidation
                    // event, never a license to renew an already queued output's epoch.
                    let until = std::time::Instant::now()
                        .checked_add(remaining)
                        .ok_or_else(Error::forbidden)?;
                    Prepared::new(facts.clone(), facts._held.clone(), until)
                })?;
        Ok(PrivateProviderMediaSnapshot { handle })
    }
}
