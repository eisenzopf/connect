//! Private precommit scene facts on the Runtime's existing authorization watcher.
use crate::{
    media_authorization::{Epoch, Handle, Lease, Prepared, SnapshotFacts},
    AuthorityMode, AuthorityScope, NativeMediaOrigin, NativeMediaRead, NativeMediaSnapshotEpoch,
    NativeReplacementMediaOrigin, NativeReplacementSceneRead, Store,
};
use connect_protocol::{Error, Result};
use std::sync::Arc;
impl SnapshotFacts for NativeReplacementSceneRead {
    fn current(&self) -> bool {
        NativeReplacementSceneRead::current(self)
    }
}
#[derive(Clone)]
pub struct NativeReplacementSceneSnapshot {
    handle: Handle<NativeReplacementSceneRead>,
    sources: Arc<[Arc<NativeMediaOrigin>]>,
}
/// Actual IO custody: complete current scene plus the exact original ingress
/// lease, when supplied. Neither is retained by queued metadata.
pub struct NativeReplacementSceneSnapshotLease {
    live: Lease<NativeReplacementSceneRead>,
    original: Option<NativeMediaRead>,
    original_epoch: Option<NativeMediaSnapshotEpoch>,
}
#[derive(Clone)]
pub struct NativeReplacementSceneSnapshotEpoch {
    scene: Epoch,
    source: Option<NativeMediaSnapshotEpoch>,
}
impl NativeReplacementSceneSnapshotEpoch {
    pub fn current(&self) -> bool {
        self.scene.current() && self.source.as_ref().is_none_or(|source| source.current())
    }
}
impl NativeReplacementSceneSnapshot {
    pub fn admit(&self) -> Result<NativeReplacementSceneSnapshotLease> {
        Ok(NativeReplacementSceneSnapshotLease {
            live: self.handle.observe()?,
            original: None,
            original_epoch: None,
        })
    }
    /// No SQL, file admission, or new source epoch. Only the same original
    /// packet can lend its bounded ingress custody to this scene delivery.
    pub fn admit_under(
        &self,
        source: &NativeMediaOrigin,
        original: &NativeMediaRead,
    ) -> Result<NativeReplacementSceneSnapshotLease> {
        if !self
            .sources
            .iter()
            .any(|candidate| std::ptr::eq(candidate.as_ref(), source))
        {
            return Err(Error::forbidden());
        }
        let original_epoch = original.packet_epoch().ok_or_else(Error::forbidden)?;
        if !original.current() || !original_epoch.current() {
            return Err(Error::forbidden());
        }
        let mut io = self.admit()?;
        let fresh_source = io
            .scene()
            .existing(source.runtime_connection_id(), None)
            .ok_or_else(Error::forbidden)?;
        // Existing pure exact owner/manager/scope/physical-lineage comparison.
        // The validated temporary is not a newly granted source admission.
        drop(
            fresh_source
                .clone()
                .bound_to_original_source(original, source)?,
        );
        io.original = Some(original.clone());
        io.original_epoch = Some(original_epoch);
        if !io.current() {
            return Err(Error::forbidden());
        }
        Ok(io)
    }
}
impl NativeReplacementSceneSnapshotLease {
    pub fn scene(&self) -> &NativeReplacementSceneRead {
        self.live.facts()
    }
    pub fn current(&self) -> bool {
        self.live.current() && self.original.as_ref().is_none_or(NativeMediaRead::current)
    }
    pub fn expires_at(&self) -> i64 {
        self.original
            .as_ref()
            .map_or(self.scene().expires_at(), |source| {
                self.scene().expires_at().min(source.expires_at())
            })
    }
    pub fn packet_epoch(&self) -> NativeReplacementSceneSnapshotEpoch {
        NativeReplacementSceneSnapshotEpoch {
            scene: self.live.packet_epoch(),
            source: self.original_epoch.clone(),
        }
    }
}
impl Store {
    /// Bounded route-miss enrollment only. Refresh uses the existing complete
    /// scene resolver on the one owned Registry watcher, never the packet task.
    pub fn try_register_native_replacement_scene_snapshot(
        self: &Arc<Self>,
        origin: Arc<NativeReplacementMediaOrigin>,
        sources: Vec<Arc<NativeMediaOrigin>>,
    ) -> Result<NativeReplacementSceneSnapshot> {
        if sources.is_empty() || sources.len() > 1024 {
            return Err(Error::forbidden());
        }
        let hints = sources
            .iter()
            .map(|source| {
                AuthorityScope::conversation(source.conversation_id(), AuthorityMode::Read)
            })
            .collect::<Result<Vec<_>>>()?;
        let sources: Arc<[Arc<NativeMediaOrigin>]> = sources.into();
        let captured_sources = sources.clone();
        let weak = Arc::downgrade(self);
        let handle = self
            .media_authorization_registry()
            .try_register_with_hints(&hints, move || {
                let store = weak.upgrade().ok_or_else(Error::forbidden)?;
                let facts = store.try_native_replacement_scene_read(&origin, &captured_sources)?;
                let guards = facts.snapshot_interval().clone();
                if !store.authority.owns(&guards) || !facts.current() {
                    return Err(Error::forbidden());
                }
                let remaining = (chrono::DateTime::from_timestamp(facts.expires_at(), 0)
                    .ok_or_else(Error::forbidden)?
                    - chrono::Utc::now())
                .to_std()
                .map_err(|_| Error::forbidden())?;
                let until = std::time::Instant::now()
                    .checked_add(remaining)
                    .ok_or_else(Error::forbidden)?;
                Prepared::new(Arc::new(facts), guards, until)
            })?;
        Ok(NativeReplacementSceneSnapshot { handle, sources })
    }
}
