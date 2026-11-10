//! Bounded routing metadata only. Real IO authority belongs to the Store lease.
use super::*;
use crate::canonical_media::weak_identity::WeakIdentity;
use arc_swap::{ArcSwap, ArcSwapOption};
use connect_store::{TelephoneMediaSnapshot, TelephoneMediaSnapshotStamp};
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
    stamp: TelephoneMediaSnapshotStamp,
    owner: MediaOwner,
}
pub(super) struct Route {
    native: Option<WeakIdentity>,
    snapshot: TelephoneMediaSnapshot,
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
pub(super) struct Routes {
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
pub(super) struct Epoch(pub connect_store::TelephoneMediaSnapshotEpoch);
impl PacketEpoch for Epoch {
    fn current(&self) -> bool {
        self.0.current()
    }
}
fn error(e: connect_protocol::Error) -> StreamControlError {
    crate::canonical_media::packet_authority_error(e)
}
impl media::Policy {
    pub(super) fn snapshot_read(
        &self,
        store: &Arc<connect_store::Store>,
        native: Option<&Arc<Native>>,
    ) -> ControlResult<(Arc<Route>, TelephoneMediaSnapshotLease)> {
        let established = self.established.get().ok_or(StreamControlError::NotReady)?;
        let routes = &established.snapshots;
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
                        .try_register_telephone_media_snapshot(
                            established.origin.clone(),
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
        read: &TelephoneMediaSnapshotLease,
    ) -> ControlResult<MediaOwner> {
        if !read.current()
            || !self.physical_current()
            || self.principal()?.is_expired_at(chrono::Utc::now())
        {
            return Err(StreamControlError::Authority);
        }
        let stamp = read.stamp();
        let cached = route.owner.load_full();
        let owner = match cached.filter(|cached| cached.stamp == stamp) {
            Some(cached) => cached.owner.clone(),
            None => {
                // Reconstruct only once per publication, never reuse facts from
                // an older role/permission/credential generation. Concurrent
                // misses may compute the same immutable facts independently.
                let owner = self.established_owner(read.telephone())?;
                route.owner.store(Some(Arc::new(Owner {
                    stamp,
                    owner: owner.clone(),
                })));
                owner
            }
        };
        self.established
            .get()
            .ok_or(StreamControlError::NotReady)?
            .proof
            .current(owner)
    }
}
