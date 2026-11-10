//! Established SIP packet facts. SQL/file admission runs only on the shared watcher.
use crate::{
    media_authorization::{Epoch, Handle, Lease, Prepared, SnapshotFacts, Stamp},
    AuthorityMode, AuthorityScope, NativeMediaOrigin, NativeMediaRead, Store, TelephoneMediaOrigin,
    TelephoneMediaRead, TelephoneNativeMediaRead,
};
use connect_protocol::{Error, Result};
use std::sync::Arc;

enum Facts {
    Unary(TelephoneMediaRead),
    Pair(TelephoneNativeMediaRead),
}
impl Facts {
    fn telephone(&self) -> &TelephoneMediaRead {
        match self {
            Self::Unary(read) => read,
            Self::Pair(read) => read.telephone(),
        }
    }
}
impl SnapshotFacts for Facts {
    fn current(&self) -> bool {
        match self {
            Self::Unary(read) => read.current(),
            Self::Pair(read) => read.current(),
        }
    }
}
#[derive(Clone)]
pub struct TelephoneMediaSnapshot {
    handle: Handle<Facts>,
}
/// One actual IO interval. Its epoch alone cannot retain file readers.
pub struct TelephoneMediaSnapshotLease {
    lease: Lease<Facts>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct TelephoneMediaSnapshotStamp(Stamp);
#[derive(Clone)]
pub struct TelephoneMediaSnapshotEpoch(Epoch);
impl TelephoneMediaSnapshotEpoch {
    pub fn current(&self) -> bool {
        self.0.current()
    }
}
impl TelephoneMediaSnapshot {
    pub fn admit(&self) -> Result<TelephoneMediaSnapshotLease> {
        Ok(TelephoneMediaSnapshotLease {
            lease: self.handle.observe()?,
        })
    }
}
impl TelephoneMediaSnapshotLease {
    pub fn current(&self) -> bool {
        self.lease.current()
    }
    pub fn stamp(&self) -> TelephoneMediaSnapshotStamp {
        TelephoneMediaSnapshotStamp(self.lease.stamp())
    }
    pub fn packet_epoch(&self) -> TelephoneMediaSnapshotEpoch {
        TelephoneMediaSnapshotEpoch(self.lease.packet_epoch())
    }
    pub fn telephone(&self) -> &TelephoneMediaRead {
        self.lease.facts().telephone()
    }
    pub fn native(&self) -> Option<&NativeMediaRead> {
        match self.lease.facts() {
            Facts::Unary(_) => None,
            Facts::Pair(read) => Some(read.native()),
        }
    }
    pub fn permits_native_receive(&self) -> bool {
        self.current()
            && matches!(self.lease.facts(), Facts::Pair(read) if read.permits_native_receive())
    }
    pub fn permits_telephone_receive(&self) -> bool {
        self.current()
            && matches!(self.lease.facts(), Facts::Pair(read) if read.permits_telephone_receive())
    }
}
impl Store {
    /// Immediate bounded enrollment; never resolves SQL or waits for a registry
    /// lock on the calling packet task. Missing publication remains a refusal.
    pub fn try_register_telephone_media_snapshot(
        self: &Arc<Self>,
        telephone: Arc<TelephoneMediaOrigin>,
        native: Option<Arc<NativeMediaOrigin>>,
    ) -> Result<TelephoneMediaSnapshot> {
        let s = telephone.snapshot();
        let hints = [
            AuthorityScope::conversation(&s.conversation_id, AuthorityMode::Read)?,
            AuthorityScope::identity(&s.route_owner_id, AuthorityMode::Read)?,
            AuthorityScope::identity(&s.identity_id, AuthorityMode::Read)?,
        ];
        let store = Arc::downgrade(self);
        let manager = Arc::downgrade(&self.authority);
        let handle = self
            .media_authorization_registry()
            .try_register_with_hints(&hints, move || {
                let store = store.upgrade().ok_or_else(Error::forbidden)?;
                // These existing resolvers retain complete current SIP lineage,
                // route credentials, reciprocal private floors and named permission.
                let facts = match &native {
                    Some(native) => Facts::Pair(
                        store.try_telephone_native_media_read(telephone.clone(), native.clone())?,
                    ),
                    None => Facts::Unary(store.try_telephone_media_read(telephone.clone())?),
                };
                let guards = facts.telephone().interval().clone();
                if !manager
                    .upgrade()
                    .is_some_and(|manager| manager.owns(&guards))
                    || !facts.current()
                {
                    return Err(Error::forbidden());
                }
                let expiry = chrono::DateTime::from_timestamp(facts.telephone().expires_at(), 0)
                    .ok_or_else(Error::forbidden)?;
                let remaining = (expiry - chrono::Utc::now())
                    .to_std()
                    .map_err(|_| Error::forbidden())?;
                let until = std::time::Instant::now()
                    .checked_add(remaining)
                    .ok_or_else(Error::forbidden)?;
                Prepared::new(Arc::new(facts), guards, until)
            })?;
        Ok(TelephoneMediaSnapshot { handle })
    }
}
