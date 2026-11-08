//! Positive SQL facts only; every read still owns fresh current authority.
use super::*;
use crate::peer_authority::NativeReplacementPacketFacts;
use std::any::Any;
use std::sync::{Mutex, Weak};
type Memo = Arc<dyn Any + Send + Sync>;
const SCENE_KEYS: usize = 4;
#[derive(Clone)]
struct MediaFacts {
    source: NativeReplacementPacketFacts,
    target: PeerConnectionProjection,
}
#[derive(Clone)]
struct SceneFacts {
    media: MediaFacts,
    reads: crate::peer_authority::SceneFacts,
    audience: std::collections::BTreeMap<String, audience::Origin>,
}
struct SceneMemo {
    sources: Vec<Weak<crate::NativeMediaOrigin>>,
    packet_source: Option<usize>,
    memo: Memo,
}
pub(super) struct Metadata {
    // Exact immutable original provenance identity, not just its Connection ID.
    // Weak keys never pin the provenance or any physical/resource owner.
    root: Weak<NativeReplacementStarted>,
    unary: Memo,
    receiver_epoch: Memo,
    scenes: Mutex<Vec<SceneMemo>>,
}
impl Metadata {
    pub(super) fn new(root: &Arc<NativeReplacementStarted>) -> Self {
        Self {
            root: Arc::downgrade(root),
            unary: Store::new_packet_projection_metadata::<(MediaFacts, i64)>(),
            receiver_epoch: Store::new_packet_projection_metadata::<(MediaFacts, i64)>(),
            scenes: Mutex::new(Vec::new()),
        }
    }
    fn require(
        &self,
        store: &Store,
        origin: &NativeReplacementMediaOrigin,
        checked: &crate::installation_ownership::CheckedRuntimeOwner<'_>,
    ) -> Result<()> {
        if !self.root.ptr_eq(&Arc::downgrade(&origin.started)) {
            return Err(Error::forbidden());
        }
        let prepared = &origin.started.attempt.allocated.attempt.prepared;
        let owner = checked.owner(store)?;
        if prepared.runtime != store.runtime_epoch()
            || !prepared
                .owner
                .upgrade()
                .is_some_and(|old| Arc::ptr_eq(&old, owner))
        {
            return Err(Error::forbidden());
        }
        prepared
            .selection
            .require_native_packet_runtime(store, checked)
    }
    fn scene(
        &self,
        sources: &[Arc<crate::NativeMediaOrigin>],
        packet_source: Option<usize>,
    ) -> Result<Memo> {
        let mut entries = self
            .scenes
            .try_lock()
            .map_err(|_| Error::new("authority-busy", "Current replacement metadata is busy"))?;
        let mut retired = Vec::new();
        let mut index = 0;
        while index < entries.len() {
            if entries[index]
                .sources
                .iter()
                .any(|source| source.strong_count() == 0)
            {
                retired.push(entries.remove(index));
            } else {
                index += 1;
            }
        }
        let found = entries
            .iter()
            .find(|entry| {
                entry.packet_source == packet_source
                    && entry.sources.len() == sources.len()
                    && entry
                        .sources
                        .iter()
                        .zip(sources)
                        .all(|(old, current)| old.ptr_eq(&Arc::downgrade(current)))
            })
            .map(|entry| entry.memo.clone());
        let result = if let Some(found) = found {
            found
        } else {
            let memo = Store::new_packet_projection_metadata::<(SceneFacts, i64)>();
            if entries.len() == SCENE_KEYS {
                retired.push(entries.remove(0));
            }
            entries.push(SceneMemo {
                sources: sources.iter().map(Arc::downgrade).collect(),
                packet_source,
                memo: memo.clone(),
            });
            memo
        };
        drop(entries);
        drop(retired);
        Ok(result)
    }
}
fn capture_media(
    store: &Store,
    c: &Connection,
    origin: &NativeReplacementMediaOrigin,
    receiver: bool,
) -> Result<(MediaFacts, Vec<AuthorityScope>, i64)> {
    let start = &origin.started.attempt;
    let selection = &start.allocated.attempt.prepared.selection;
    let source = selection.native_packet_facts(store, c)?;
    let (member, saved, scopes, expiry) = media_facts_with_packet_capture(
        store,
        c,
        origin,
        Some((source.context(), source.scopes())),
    )?;
    if receiver {
        receiver_epoch::require_current(c, origin, &saved)?;
    }
    let descriptor: serde_json::Value =
        serde_json::from_str(&start.target).map_err(|_| Error::storage())?;
    let target = projection_from_facts(
        source.context(),
        &source.source().session,
        &source.source().accepted_capabilities,
        member,
        &saved,
        &descriptor,
    )?;
    let expiry = expiry.min(source.expires_at());
    Ok((MediaFacts { source, target }, scopes, expiry))
}
fn media_read(
    store: &Store,
    origin: &NativeReplacementMediaOrigin,
    facts: &MediaFacts,
    held: Arc<AuthorityGuardSet>,
    expiry: i64,
    checked: &crate::installation_ownership::CheckedRuntimeOwner<'_>,
) -> Result<NativeReplacementMediaRead> {
    origin.metadata.require(store, origin, checked)?;
    let selection = &origin.started.attempt.allocated.attempt.prepared.selection;
    let source =
        selection.read_native_packet_facts(store, &facts.source, held.clone(), expiry, checked)?;
    Ok(NativeReplacementMediaRead {
        source,
        target: facts.target.clone(),
        interval: held,
    })
}
pub(super) fn media(
    store: &Store,
    origin: &NativeReplacementMediaOrigin,
    receiver: bool,
) -> Result<NativeReplacementMediaRead> {
    let checked = store.checked_runtime_owner()?;
    origin.metadata.require(store, origin, &checked)?;
    let memo = if receiver {
        &origin.metadata.receiver_epoch
    } else {
        &origin.metadata.unary
    };
    let read = store.try_memoized_current_packet_projection(
        memo.as_ref(),
        |c| capture_media(store, c, origin, receiver),
        |_, facts, held, expiry| media_read(store, origin, facts, held, expiry, &checked),
        None,
    )?;
    checked.revalidate()?;
    origin.metadata.require(store, origin, &checked)?;
    if !read.current() {
        return Err(Error::forbidden());
    }
    Ok(read)
}
pub(super) fn scene(
    store: &Store,
    origin: &NativeReplacementMediaOrigin,
    sources: &[Arc<crate::NativeMediaOrigin>],
    custody: Option<(&Arc<AuthorityGuardSet>, i64, &crate::NativeMediaOrigin)>,
) -> Result<NativeReplacementSceneRead> {
    scene_with_final_hook(store, origin, sources, custody, || {})
}
fn scene_with_final_hook(
    store: &Store,
    origin: &NativeReplacementMediaOrigin,
    sources: &[Arc<crate::NativeMediaOrigin>],
    custody: Option<(&Arc<AuthorityGuardSet>, i64, &crate::NativeMediaOrigin)>,
    before_return: impl FnOnce(),
) -> Result<NativeReplacementSceneRead> {
    let checked = store.checked_runtime_owner()?;
    origin.metadata.require(store, origin, &checked)?;
    for source in sources {
        source.require_checked_runtime(store, &checked)?;
    }
    let packet_source = match custody {
        Some((_, _, packet)) => Some(
            sources
                .iter()
                .position(|source| std::ptr::eq(source.as_ref(), packet))
                .ok_or_else(Error::forbidden)?,
        ),
        None => None,
    };
    let memo = origin.metadata.scene(sources, packet_source)?;
    let prepared = &origin.started.attempt.allocated.attempt.prepared;
    let read = store.try_memoized_current_packet_projection(
        memo.as_ref(),
        |c| {
            let (media, mut scopes, expiry) = capture_media(store, c, origin, false)?;
            let context = media.source.context();
            let original_ids = [
                context.source_runtime_connection_id.as_str(),
                context.retained_runtime_connection_id.as_str(),
            ];
            if !original_ids.iter().all(|id| {
                sources.iter().any(|source| source.runtime_connection_id() == *id)
            }) {
                return Err(Error::forbidden());
            }
            let (mut reads, scene_scopes, scene_expiry) =
                store.native_scene_facts_in_packet_projection(
                    c, sources, &context.conversation_id, &context.session_id,
                )?;
            scopes.extend(scene_scopes);
            let reservation_expiry: i64 = c.query_row(
                "SELECT expires_at FROM native_replacements WHERE operation_record_id=?1 AND runtime_epoch=?2 AND reservation_id=?3",
                params![prepared.record, prepared.runtime, prepared.reservation],
                |row| row.get(0),
            ).map_err(|_| Error::storage())?;
            store.native_replacement_scene_pairs(
                c, sources, &mut reads, &context.source_runtime_connection_id,
                None, &media.target,
                origin.started.attempt.request.credentials.last().ok_or_else(Error::forbidden)?,
            )?;
            let audience = audience::origins(c, sources, &media.target, &prepared.record)?;
            let facts = SceneFacts { media, reads, audience };
            let expiry = expiry.min(scene_expiry).min(reservation_expiry);
            Ok((facts, AuthorityScope::merged(&scopes)?, expiry))
        },
        |_, facts, held, expiry| {
            let media = Arc::new(media_read(store, origin, &facts.media, held.clone(), expiry, &checked)?);
            for source in sources {
                source.require_checked_runtime(store, &checked)?;
            }
            Ok(NativeReplacementSceneRead {
                media,
                reads: store.native_scene_reads_checked(facts.reads.clone(), held, expiry, &checked)?,
                audience_sources: facts.audience.clone(),
                telephone_pairs: std::collections::BTreeMap::new(),
            })
        },
        custody.map(|(held, expiry, _)| (held, expiry)),
    )?;
    before_return();
    checked.revalidate()?;
    origin.metadata.require(store, origin, &checked)?;
    for source in sources {
        source.require_checked_runtime(store, &checked)?;
    }
    if !read.current() {
        return Err(Error::forbidden());
    }
    Ok(read)
}
#[cfg(test)]
impl NativeReplacementMediaOrigin {
    // Counts describe currently retained metadata. Eviction/prune may remove
    // historical counters; compare stable-key calls, not cumulative lifetimes.
    pub(crate) fn packet_memo_counts_for_test(&self) -> [u64; 3] {
        let mut counts = [0; 3];
        for memo in [&self.metadata.unary, &self.metadata.receiver_epoch] {
            let values = Store::packet_projection_memo_counts_for_test::<MediaFacts>(memo.as_ref());
            for (sum, value) in counts.iter_mut().zip(values) {
                *sum += value;
            }
        }
        for scene in self.metadata.scenes.lock().unwrap().iter() {
            let values =
                Store::packet_projection_memo_counts_for_test::<SceneFacts>(scene.memo.as_ref());
            for (sum, value) in counts.iter_mut().zip(values) {
                *sum += value;
            }
        }
        counts
    }
    pub(crate) fn packet_scene_memo_keys_for_test(&self) -> usize {
        self.metadata.scenes.lock().unwrap().len()
    }
}

#[cfg(test)]
impl Store {
    pub(crate) fn native_scene_with_final_hook_for_test(
        &self,
        origin: &NativeReplacementMediaOrigin,
        sources: &[Arc<crate::NativeMediaOrigin>],
        before_return: impl FnOnce(),
    ) -> Result<NativeReplacementSceneRead> {
        scene_with_final_hook(self, origin, sources, None, before_return)
    }
}
