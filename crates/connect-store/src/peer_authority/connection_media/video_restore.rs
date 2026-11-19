//! Cold current OLD + exact selected publishers under one complete interval.
//! This issues ordinary current media facts only, never a closed-candidate
//! receipt, cancelled-target grant or permission to thaw a frozen Core route.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

pub struct NativeVideoRestoreRead {
    old: NativeMediaRead,
    pairs: BTreeMap<String, NativeMediaRead>,
}
impl NativeVideoRestoreRead {
    pub fn current(&self) -> bool {
        self.old.current() && self.pairs.values().all(NativeMediaRead::current)
    }
    pub fn old(&self) -> &NativeMediaRead {
        &self.old
    }
    pub fn received_from(&self, source_runtime: &str) -> Option<&NativeMediaRead> {
        self.pairs.get(source_runtime)
    }
}
impl Store {
    /// This synchronous control read owns no SQL after return. All selected
    /// pair scopes are admitted TOGETHER, before resealing current facts; no
    /// nested fresh Reader can be queued behind a Writer while OLD is retained.
    /// The caller must independently own actual original native Closed proof
    /// and compare the current Core selected source descriptors before restore.
    pub fn try_native_video_restore_read(
        &self,
        old: &NativeMediaOrigin,
        publishers: &[Arc<NativeMediaOrigin>],
    ) -> Result<NativeVideoRestoreRead> {
        if publishers.len() > self.limits().connections {
            return Err(Error::forbidden());
        }
        current_owner(self, old)?;
        let mut unique = BTreeSet::new();
        for source in publishers {
            current_owner(self, source)?;
            if !unique.insert(source.runtime_connection_id()) {
                return Err(Error::forbidden());
            }
        }
        let ((old_facts, pair_facts, cutoff), held) = snapshot::acquire(self, |c| {
            let (target, source, mut required, mut cutoff) = views(self, c, old, None)?;
            let ordinary = (target, source, subscription_permission(c, old)?);
            let mut pairs = BTreeMap::new();
            for publisher in publishers {
                let (target, source, scopes, expiry) = views(self, c, old, Some(publisher))?;
                cutoff = cutoff.min(expiry);
                required.extend(scopes);
                pairs.insert(
                    publisher.runtime_connection_id().to_owned(),
                    (target, source, subscription_permission(c, old)?),
                );
            }
            Ok((
                (ordinary, pairs, cutoff),
                AuthorityScope::merged(&required)?,
            ))
        })?;
        let owner = current_owner(self, old)?;
        for source in publishers {
            current_owner(self, source)?;
        }
        let make = |(target, source, may_subscribe)| NativeMediaRead {
            _live: None,
            _source_live: None,
            target,
            source,
            _scopes: held.clone(),
            owner: owner.clone(),
            expires_at: cutoff,
            may_subscribe,
        };
        let read = NativeVideoRestoreRead {
            old: make(old_facts),
            pairs: pair_facts.into_iter().map(|(k, v)| (k, make(v))).collect(),
        };
        if !read.current() {
            return Err(Error::forbidden());
        }
        Ok(read)
    }
}
