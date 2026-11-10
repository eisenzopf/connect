//! Initial SIP packet facts with explicit original purpose/phase. SQL/file admission runs only on the shared watcher.
use crate::{
    media_authorization::{Epoch, Handle, Lease, Prepared, SnapshotFacts, Stamp},
    AuthorityMode, AuthorityScope, InitialTelephoneMediaOrigin, InitialTelephoneMediaRead,
    InitialTelephoneNativeRead, NativeMediaOrigin, NativeMediaRead, Store,
};
use connect_protocol::{Error, Result};
use std::sync::Arc;

enum Read {
    Unary(InitialTelephoneMediaRead),
    Pair(InitialTelephoneNativeRead),
}
impl Read {
    fn phone(&self) -> &InitialTelephoneMediaRead {
        match self {
            Self::Unary(read) => read,
            Self::Pair(read) => read.phone(),
        }
    }
}
impl Read {
    fn current(&self) -> bool {
        match self {
            Self::Unary(read) => read.current(),
            Self::Pair(read) => read.current(),
        }
    }
}
struct Facts {
    read: Read,
    phase: InitialTelephonePacketPhase,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitialTelephonePacketPhase {
    Ordinary,
    PrivateApplied,
    PrivateEstablished,
}
impl SnapshotFacts for Facts {
    fn current(&self) -> bool {
        self.read.current()
    }
}
#[derive(Clone)]
pub struct InitialTelephoneMediaSnapshot {
    handle: Handle<Facts>,
}
/// One actual IO interval. Its epoch alone cannot retain file readers.
pub struct InitialTelephoneMediaSnapshotLease {
    lease: Lease<Facts>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct InitialTelephoneMediaSnapshotStamp(Stamp);
#[derive(Clone)]
pub struct InitialTelephoneMediaSnapshotEpoch(Epoch);
impl InitialTelephoneMediaSnapshotEpoch {
    pub fn current(&self) -> bool {
        self.0.current()
    }
}
impl InitialTelephoneMediaSnapshot {
    pub fn admit(&self) -> Result<InitialTelephoneMediaSnapshotLease> {
        Ok(InitialTelephoneMediaSnapshotLease {
            lease: self.handle.observe()?,
        })
    }
}
impl InitialTelephoneMediaSnapshotLease {
    pub fn current(&self) -> bool {
        self.lease.current()
    }
    pub fn stamp(&self) -> InitialTelephoneMediaSnapshotStamp {
        InitialTelephoneMediaSnapshotStamp(self.lease.stamp())
    }
    pub fn packet_epoch(&self) -> InitialTelephoneMediaSnapshotEpoch {
        InitialTelephoneMediaSnapshotEpoch(self.lease.packet_epoch())
    }
    pub fn phone(&self) -> &InitialTelephoneMediaRead {
        self.lease.facts().read.phone()
    }
    pub fn native(&self) -> Option<&NativeMediaRead> {
        match &self.lease.facts().read {
            Read::Unary(_) => None,
            Read::Pair(read) => Some(read.native()),
        }
    }
    pub fn phase(&self) -> InitialTelephonePacketPhase {
        self.lease.facts().phase
    }
}
impl Store {
    /// Immediate bounded enrollment; never resolves SQL or waits for a registry
    /// lock on the calling packet task. Missing publication remains a refusal.
    pub fn try_register_initial_telephone_media_snapshot(
        self: &Arc<Self>,
        telephone: Arc<InitialTelephoneMediaOrigin>,
        native: Option<Arc<NativeMediaOrigin>>,
    ) -> Result<InitialTelephoneMediaSnapshot> {
        let s = telephone.source();
        let hints = [
            AuthorityScope::conversation(&s.conversation_id, AuthorityMode::Read)?,
            AuthorityScope::identity(&s.route_owner_id, AuthorityMode::Read)?,
            AuthorityScope::identity(&s.target.identity_id, AuthorityMode::Read)?,
        ];
        let store = Arc::downgrade(self);
        let manager = Arc::downgrade(&self.authority);
        let handle = self
            .media_authorization_registry()
            .try_register_with_hints(&hints, move || {
                let store = store.upgrade().ok_or_else(Error::forbidden)?;
                // These existing resolvers retain complete current SIP lineage,
                // route credentials, reciprocal private floors and named permission.
                let read = match &native {
                    Some(native) => Read::Pair(
                        store
                            .try_initial_telephone_native_read(telephone.clone(), native.clone())?,
                    ),
                    None if !telephone.source().is_private_candidate() => {
                        Read::Unary(store.try_initial_telephone_media_read(telephone.clone())?)
                    }
                    None => Read::Unary(
                        store.try_private_initial_telephone_media_read(telephone.clone())?,
                    ),
                };
                if !read.phone().is_connected() || read.phone().is_private_admission() {
                    return Err(Error::forbidden());
                }
                let phase = if telephone.source().is_private_candidate() {
                    use crate::PrivateInitialTelephonePhase::*;
                    match store.private_initial_telephone_phase(&telephone)? {
                        Applied => InitialTelephonePacketPhase::PrivateApplied,
                        Established => InitialTelephonePacketPhase::PrivateEstablished,
                        Quarantined | Prepared => {
                            return Err(Error::new(
                                "authority-busy",
                                "Private telephone packet phase is not applied",
                            ))
                        }
                        Closed => return Err(Error::forbidden()),
                    }
                } else {
                    InitialTelephonePacketPhase::Ordinary
                };
                let facts = Facts { read, phase };
                let guards = facts.read.phone().interval().clone();
                if !manager
                    .upgrade()
                    .is_some_and(|manager| manager.owns(&guards))
                    || !facts.current()
                {
                    return Err(Error::forbidden());
                }
                let expiry = chrono::DateTime::from_timestamp(facts.read.phone().expires_at(), 0)
                    .ok_or_else(Error::forbidden)?;
                let remaining = (expiry - chrono::Utc::now())
                    .to_std()
                    .map_err(|_| Error::forbidden())?;
                let until = std::time::Instant::now()
                    .checked_add(remaining)
                    .ok_or_else(Error::forbidden)?;
                Prepared::new(Arc::new(facts), guards, until)
            })?;
        Ok(InitialTelephoneMediaSnapshot { handle })
    }
}
