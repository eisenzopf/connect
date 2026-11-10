//! Exact precommit source-set routing metadata; no inline authority refresh.
use super::*;
use crate::canonical_media::weak_identity::WeakIdentity;
use arc_swap::ArcSwap;
use connect_store::{NativeReplacementSceneSnapshot, NativeReplacementSceneSnapshotLease};
use std::{collections::BTreeMap, sync::Mutex};
type Entry = crate::canonical_media::Connection;
// The sole packet caller selects at most the original source, retained peer,
// and this exact publisher. Keep this bounded identity key inline; the map
// allocates its owned Vec only when a new route is actually enrolled.
struct RouteKey {
    parts: [usize; 4],
    len: usize,
}
impl RouteKey {
    fn new(origin: usize, entries: impl Iterator<Item = usize>) -> connect_protocol::Result<Self> {
        let mut key = Self {
            parts: [origin, 0, 0, 0],
            len: 1,
        };
        for owner in entries {
            let next = key.parts.get_mut(key.len).ok_or_else(|| {
                connect_protocol::Error::new(
                    "capacity-exceeded",
                    "Private receive source key exceeds its original source set",
                )
            })?;
            *next = owner;
            key.len += 1;
        }
        Ok(key)
    }
    fn as_slice(&self) -> &[usize] {
        &self.parts[..self.len]
    }
}
struct Route {
    entries: Vec<WeakIdentity>,
    snapshot: NativeReplacementSceneSnapshot,
}
impl Route {
    fn matches(&self, entries: &[Arc<Entry>]) -> bool {
        self.entries.len() == entries.len()
            && self
                .entries
                .iter()
                .zip(entries)
                .all(|(old, current)| old.matches(current))
    }
    fn live(&self) -> bool {
        self.entries.iter().all(WeakIdentity::live)
    }
}
pub(crate) struct Routes {
    published: ArcSwap<BTreeMap<Vec<usize>, Arc<Route>>>,
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
pub(super) struct Epoch(pub connect_store::NativeReplacementSceneSnapshotEpoch);
impl PacketEpoch for Epoch {
    fn current(&self) -> bool {
        self.0.current()
    }
}
impl Routes {
    pub(super) fn admit(
        &self,
        store: &Arc<connect_store::Store>,
        origin: &Arc<NativeReplacementMediaOrigin>,
        entries: &[Arc<Entry>],
        original: Option<(
            &Arc<connect_store::NativeMediaOrigin>,
            &connect_store::NativeMediaRead,
        )>,
    ) -> connect_protocol::Result<NativeReplacementSceneSnapshotLease> {
        // Exact Arc identities include all generation/credential/physical facts.
        // Registered factories hold origins, never their Service entry owners.
        let key = RouteKey::new(
            Arc::as_ptr(origin) as usize,
            entries
                .iter()
                .map(|entry| Arc::as_ptr(&entry.origin) as usize),
        )?;
        let found = self
            .published
            .load()
            .get(key.as_slice())
            .filter(|route| route.matches(entries))
            .cloned();
        let route = match found {
            Some(route) => route,
            None => {
                let _registration = self.registration.try_lock().map_err(|error| match error {
                    std::sync::TryLockError::WouldBlock => connect_protocol::Error::new(
                        "authority-busy",
                        "Original replacement route enrollment is busy",
                    ),
                    std::sync::TryLockError::Poisoned(_) => connect_protocol::Error::forbidden(),
                })?;
                let mut next = (**self.published.load()).clone();
                if let Some(route) = next
                    .get(key.as_slice())
                    .filter(|route| route.matches(entries))
                    .cloned()
                {
                    route
                } else {
                    if next.values().filter(|route| route.live()).count() >= 1024 {
                        return Err(connect_protocol::Error::new(
                            "authority-busy",
                            "Original replacement route capacity is busy",
                        ));
                    }
                    let snapshot = store.try_register_native_replacement_scene_snapshot(
                        origin.clone(),
                        entries.iter().map(|entry| entry.origin.clone()).collect(),
                    )?;
                    let route = Arc::new(Route {
                        entries: entries.iter().map(WeakIdentity::new).collect(),
                        snapshot,
                    });
                    next.retain(|_, route| route.live());
                    next.insert(key.as_slice().to_vec(), route.clone());
                    self.published.store(Arc::new(next));
                    route
                }
            }
        };
        match original {
            Some((source, read)) => route.snapshot.admit_under(source, read),
            None => route.snapshot.admit(),
        }
    }
}

// Original ingress must retain retryable metadata absence; the existing outer
// packet_error maps this to retained-packet NotReady only when appropriate.
pub(super) fn packet_error(error: connect_protocol::Error) -> StreamControlError {
    if error.code == "authority-busy" {
        StreamControlError::MetadataBusy
    } else {
        crate::canonical_media::packet_authority_error(error)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scene_route_miss_is_metadata_busy_and_definitive_refusal_stays_authority() {
        assert!(matches!(
            packet_error(connect_protocol::Error::new("authority-busy", "pending")),
            StreamControlError::MetadataBusy
        ));
        assert!(matches!(
            packet_error(connect_protocol::Error::forbidden()),
            StreamControlError::Authority
        ));
    }
    #[test]
    fn inline_scene_key_uses_exact_ordered_arc_identities_for_borrowed_lookup() {
        let origin = Arc::new(());
        let first = Arc::new(7_u64);
        let second = Arc::new(7_u64);
        let ptr = |owner: &Arc<u64>| Arc::as_ptr(owner) as usize;
        let original = RouteKey::new(
            Arc::as_ptr(&origin) as usize,
            [ptr(&first), ptr(&second)].into_iter(),
        )
        .unwrap();
        let owned = BTreeMap::from([(original.as_slice().to_vec(), 19)]);
        let same = RouteKey::new(
            Arc::as_ptr(&origin) as usize,
            [ptr(&first), ptr(&second)].into_iter(),
        )
        .unwrap();
        assert_eq!(owned.get(same.as_slice()), Some(&19));
        let reordered = RouteKey::new(
            Arc::as_ptr(&origin) as usize,
            [ptr(&second), ptr(&first)].into_iter(),
        )
        .unwrap();
        assert!(owned.get(reordered.as_slice()).is_none());
        let prefix =
            RouteKey::new(Arc::as_ptr(&origin) as usize, [ptr(&first)].into_iter()).unwrap();
        assert!(owned.get(prefix.as_slice()).is_none());
        let equal_metadata = Arc::new(7_u64);
        let replaced = RouteKey::new(
            Arc::as_ptr(&origin) as usize,
            [ptr(&first), ptr(&equal_metadata)].into_iter(),
        )
        .unwrap();
        assert!(owned.get(replaced.as_slice()).is_none());
        assert_eq!(
            RouteKey::new(0, [1, 2, 3, 4].into_iter())
                .err()
                .unwrap()
                .code,
            "capacity-exceeded"
        );
    }
}
