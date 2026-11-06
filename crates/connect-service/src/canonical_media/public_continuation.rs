//! Short current-route continuation between physical commit and SQL projection.
//! Installed inactive before Core commit. Classification is not authority: every
//! packet additionally needs a fresh Store pair and the actual managed route.
use super::*;
use rvoip_core::orchestrator::participant_control::CurrentReplacementPublication;
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) struct Continuation {
    root: Weak<crate::canonical_replacement::PreparedReplacement>,
    pub(super) metadata: Arc<connect_store::NativeReplacementActivation>,
    pub(super) entry: Arc<Connection>,
    ordinary: AtomicBool,
}
impl Continuation {
    pub(super) fn new(
        root: Weak<crate::canonical_replacement::PreparedReplacement>,
        metadata: Arc<connect_store::NativeReplacementActivation>,
        entry: Arc<Connection>,
    ) -> Arc<Self> {
        Arc::new(Self {
            root,
            metadata,
            entry,
            ordinary: AtomicBool::new(false),
        })
    }
    pub(crate) fn ordinary(&self) -> bool {
        self.ordinary.load(Ordering::Acquire)
    }
    /// Called inside the exact installed-entry publication guard immediately
    /// after that original SQL transaction committed. It only changes lookup.
    pub(super) fn publish_ordinary(&self) {
        self.ordinary.store(true, Ordering::Release);
    }
    pub(super) fn current(
        &self,
        owner: &MediaOwner,
    ) -> ControlResult<CurrentReplacementPublication> {
        // No Core mutex crosses Store acquisition: this method is called only
        // after the fresh reader exists. The Core result retains a route ticket,
        // never a Core/session owner or any mutex guard.
        let root = self.root.upgrade().ok_or(StreamControlError::Authority)?;
        if self.ordinary() {
            return Err(StreamControlError::MetadataBusy);
        }
        if tokio::time::Instant::now() >= root.deadline {
            return Err(StreamControlError::Deadline);
        }
        root.handle.current_publication(owner)
    }
}
impl crate::canonical_replacement::TargetPolicy {
    pub(crate) fn current_continuation(&self) -> ControlResult<Option<Arc<Continuation>>> {
        let Some(continuation) = self.public_continuation.get() else {
            return Ok(None);
        };
        if continuation.ordinary() {
            return Err(StreamControlError::MetadataBusy);
        }
        let root = self.root.upgrade().ok_or(StreamControlError::Authority)?;
        // Routing classification only. The later current() cannot be bypassed
        // with this historical phase observation, even after a second handoff.
        Ok(root.handle.is_committed().then(|| continuation.clone()))
    }
}

impl Continuation {
    /// Fresh continuation authority for receiver metadata only. The actual
    /// Core publication is retained through the same original reliable write.
    pub(crate) fn receiver_epoch_read(
        &self,
        principal: &rvoip_core::AuthenticatedPrincipal,
    ) -> ControlResult<(
        Arc<Connection>,
        Arc<NativeMediaRead>,
        CurrentReplacementPublication,
    )> {
        let domain = self
            .entry
            .domain
            .upgrade()
            .ok_or(StreamControlError::Cancelled)?;
        let read = Arc::new(
            domain
                .store
                .try_native_replacement_continuation_media_read(
                    &self.entry.origin,
                    Some(&self.metadata),
                    None,
                    None,
                )
                .map_err(super::packet_authority_error)?,
        );
        let owner = self.entry.owner(read.target(), principal, false)?;
        let owner = self
            .entry
            .activation()?
            .ok_or(StreamControlError::NotReady)?
            .current(owner)?;
        let publication = self.current(&owner)?;
        Ok((self.entry.clone(), read, publication))
    }
}
