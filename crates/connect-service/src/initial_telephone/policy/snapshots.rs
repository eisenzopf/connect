//! Bounded routing metadata only. Real IO authority belongs to the Store lease.
use super::*;
use crate::canonical_media::weak_identity::WeakIdentity;
use arc_swap::{ArcSwap, ArcSwapOption};
use connect_store::{
    InitialTelephoneMediaSnapshot, InitialTelephoneMediaSnapshotLease,
    InitialTelephoneMediaSnapshotStamp,
};
use std::{collections::BTreeMap, sync::Mutex};
type Native = crate::canonical_media::Connection;
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    owner: usize,
    generation: u64,
    epoch: u64,
}
fn key(native: Option<&Arc<Native>>) -> Option<Key> {
    native.map(|n| Key {
        owner: Arc::as_ptr(n) as usize,
        generation: n.origin.generation(),
        epoch: n.origin.credential_epoch(),
    })
}
struct Owner {
    stamp: InitialTelephoneMediaSnapshotStamp,
    owner: MediaOwner,
}
pub(super) struct Route {
    native: Option<WeakIdentity>,
    snapshot: InitialTelephoneMediaSnapshot,
    owner: ArcSwapOption<Owner>,
}
impl Route {
    fn matches(&self, native: Option<&Arc<Native>>) -> bool {
        match (&self.native, native) {
            (None, None) => true,
            (Some(weak), Some(current)) => weak.matches(current),
            _ => false,
        }
    }
    fn live(&self) -> bool {
        self.native.as_ref().is_none_or(WeakIdentity::live)
    }
}
pub(crate) struct Routes {
    published: ArcSwap<BTreeMap<Option<Key>, Arc<Route>>>,
    registration: Mutex<()>,
}
impl Default for Routes {
    fn default() -> Self {
        Self {
            published: ArcSwap::from_pointee(BTreeMap::new()),
            registration: Mutex::new(()),
        }
    }
}
pub(super) struct Epoch(pub connect_store::InitialTelephoneMediaSnapshotEpoch);
impl PacketEpoch for Epoch {
    fn current(&self) -> bool {
        self.0.current()
    }
}
fn error(e: connect_protocol::Error) -> StreamControlError {
    crate::canonical_media::packet_authority_error(e)
}
impl Leg {
    pub(super) fn snapshot_read(
        &self,
        store: &Arc<connect_store::Store>,
        native: Option<&Arc<Native>>,
    ) -> ControlResult<(Arc<Route>, InitialTelephoneMediaSnapshotLease)> {
        let routes = &self.snapshots;
        let key = key(native);
        let found = routes
            .published
            .load()
            .get(&key)
            .filter(|route| route.matches(native))
            .cloned();
        let route = match found {
            Some(route) => route,
            None => {
                let _registration = routes.registration.try_lock().map_err(|e| match e {
                    std::sync::TryLockError::WouldBlock => StreamControlError::NotReady,
                    std::sync::TryLockError::Poisoned(_) => StreamControlError::Authority,
                })?;
                let mut next = (**routes.published.load()).clone();
                if let Some(route) = next.get(&key).filter(|r| r.matches(native)).cloned() {
                    route
                } else {
                    // No removal/publication on failed enrollment.
                    if next.values().filter(|r| r.live()).count() >= 1024 {
                        return Err(StreamControlError::NotReady);
                    }
                    let snapshot = store
                        .try_register_initial_telephone_media_snapshot(
                            self.origin()?,
                            native.map(|n| n.origin.clone()),
                        )
                        .map_err(error)?;
                    let route = Arc::new(Route {
                        native: native.map(WeakIdentity::new),
                        snapshot,
                        owner: ArcSwapOption::empty(),
                    });
                    next.retain(|_, route| route.live());
                    next.insert(key, route.clone());
                    routes.published.store(Arc::new(next));
                    route
                }
            }
        };
        let read = route.snapshot.admit().map_err(error)?;
        Ok((route, read))
    }
    pub(super) fn snapshot_owner(
        &self,
        route: &Route,
        read: &InitialTelephoneMediaSnapshotLease,
    ) -> ControlResult<MediaOwner> {
        if !read.current() {
            return Err(StreamControlError::Authority);
        }
        self.validate_owner_read(read.phone())?;
        let stamp = read.stamp();
        let owner = match route.owner.load_full().filter(|owner| owner.stamp == stamp) {
            Some(owner) => owner.owner.clone(),
            None => {
                let owner = self.owner(read.phone())?;
                route.owner.store(Some(Arc::new(Owner {
                    stamp,
                    owner: owner.clone(),
                })));
                owner
            }
        };
        match self.proof.load_full().filter(|proof| proof.is_committed()) {
            Some(proof) => proof.current(owner),
            None => Ok(owner),
        }
    }
    pub(super) fn snapshot_current(
        &self,
        scope: &MediaScope<'_>,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        let runtime = self.runtime()?;
        if let MediaPurpose::Receive { source } = scope.purpose {
            let native = runtime.media.entry(&source.connection)?;
            let (route, read) = self.snapshot_read(&runtime.media.store, Some(&native))?;
            let current_native = read.native().ok_or(StreamControlError::Authority)?;
            if !read.current()
                || !read.phone().is_connected()
                || !read.phone().may_receive()
                || current_native.target().handoff_preparation
                || current_native.target().publication_suspended
            {
                return Err(StreamControlError::Authority);
            }
            crate::canonical_media::source_matches(source, &native, current_native.target())?;
            let mut owner = self.snapshot_owner(&route, &read)?;
            owner.may_route_source = true;
            return Ok(Box::new(PairLease { owner, _read: read }));
        }
        let (route, read) = self.snapshot_read(&runtime.media.store, None)?;
        if !read.phone().is_connected() || read.phone().is_private_admission() {
            return Err(StreamControlError::Authority);
        }
        let owner = self.snapshot_owner(&route, &read)?;
        Ok(Box::new(PairLease { owner, _read: read }))
    }
}
