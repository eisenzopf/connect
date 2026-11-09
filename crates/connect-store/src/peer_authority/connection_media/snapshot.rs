//! SQL-versioned metadata; fresh admission or exact active file custody.
use super::*;
use crate::installation_ownership::PacketConnectionGuard;
use crate::packet_observation::{Observation, ObserverId};
use std::sync::Mutex;
type ErasedMetadata = dyn std::any::Any + Send + Sync;
pub(super) type MetadataToken = Arc<ErasedMetadata>;

pub(super) fn new_metadata() -> MetadataToken {
    Arc::new(Metadata::default())
}
pub(super) fn metadata(origin: &NativeMediaOrigin) -> Result<&Metadata> {
    origin.metadata.downcast_ref().ok_or_else(Error::storage)
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Version {
    connection: ObserverId,
    external: i64,
    schema: i64,
    changes: u64,
    second: i64,
}
impl Version {
    fn same_database(self, other: Self) -> bool {
        Self {
            second: self.second,
            ..other
        } == self
    }
}
const DATA_VERSION_SQL: &str = "PRAGMA data_version";
const SCHEMA_VERSION_SQL: &str = "PRAGMA schema_version";

fn version(c: &Connection, observer: ObserverId) -> Result<Version> {
    let external = c
        .prepare_cached(DATA_VERSION_SQL)
        .and_then(|mut statement| statement.query_row([], |row| row.get(0)))
        .map_err(|_| Error::storage())?;
    Ok(Version {
        connection: observer,
        external,
        schema: c
            .prepare_cached(SCHEMA_VERSION_SQL)
            .and_then(|mut statement| statement.query_row([], |row| row.get(0)))
            .map_err(|_| Error::storage())?,
        changes: c.total_changes(),
        second: crate::now(),
    })
}

// A failed multi-query resolution needs the same snapshot boundary as a
// successful one. A concurrent commit or clock edge can mix pre-transition
// context with post-transition execution and manufacture a definitive refusal.
// Such an observation grants no IO and is not proof of physical retirement.
fn current_refusal<T>(c: &Connection, before: Version, result: Result<T>) -> Result<T> {
    if result.is_err() && version(c, before.connection)? != before {
        if let Err(error) = &result {
            tracing::debug!(target: "connect_store::canonical_snapshot", code = %error.code,
                reason = %error.message, "Refused packet resolution crossed a snapshot boundary");
        }
        return Err(Error::new(
            "authority-busy",
            "Current packet state changed during refused resolution",
        ));
    }
    result
}

type MediaFacts = (
    PeerConnectionProjection,
    Option<PeerConnectionProjection>,
    i64,
    bool,
);
/// Four purpose-separated memos per live origin. Each SQL observer retains one unary capture or
/// at most four original counterparties, covering the four-participant profile
/// without replacing paired facts on every mixed packet. Eviction only causes
/// fresh derivation; it does not limit eligible peers. SQL and source stamps
/// remain exact on every read. Positive MediaFacts may span clock seconds only
/// inside their original validated interval; other metadata remains exact-clock.
/// No reader, execution handle or IO grant can be retained by this metadata.
#[derive(Default)]
pub(super) struct Metadata {
    pub(super) lifetime: Memo<i64>,
    pub(super) unary: Memo<MediaFacts>,
    pub(super) paired: Memo<MediaFacts>,
    // Pending publication facts never share the ordinary unary/paired namespace.
    pub(super) continuation: Memo<MediaFacts>,
}
pub(super) struct Memo<T>(Mutex<Vec<Entry<T>>>, #[cfg(test)] MemoCounts);
#[cfg(test)]
#[derive(Default)]
struct MemoCounts {
    derive: std::sync::atomic::AtomicU64,
    hit: std::sync::atomic::AtomicU64,
    publish: std::sync::atomic::AtomicU64,
}
enum ProjectionCapture<T> {
    Owned(T),
    Shared(Arc<(T, i64)>),
}
impl<T> ProjectionCapture<T> {
    fn facts(&self) -> &T {
        match self {
            Self::Owned(facts) => facts,
            Self::Shared(facts) => &facts.0,
        }
    }
}
const SOURCES_PER_OBSERVER: usize = 4;
impl<T> Default for Memo<T> {
    fn default() -> Self {
        Self(
            Mutex::new(Vec::new()),
            #[cfg(test)]
            MemoCounts::default(),
        )
    }
}
#[derive(Clone)]
struct Entry<T> {
    version: Version,
    source: Option<Weak<ErasedMetadata>>,
    facts: Arc<T>,
    scopes: Vec<AuthorityScope>,
}
impl<T> Entry<T> {
    fn matches(&self, version: Version, source: Option<&MetadataToken>) -> bool {
        self.version == version && self.same_source(source)
    }
    fn same_source(&self, source: Option<&MetadataToken>) -> bool {
        match (&self.source, source) {
            (None, None) => true,
            (Some(old), Some(current)) => old.ptr_eq(&Arc::downgrade(current)),
            _ => false,
        }
    }
}

fn media_clock_matches(captured: Version, observed: Version, expires: i64) -> bool {
    captured.same_database(observed)
        && captured.second <= observed.second
        && observed.second < expires
}

impl Entry<MediaFacts> {
    // A successful media projection already carries the earliest absolute
    // expiry of every source/recipient predicate. Keep its original interval;
    // a hit never advances the capture second or extends that expiry.
    fn matches_current_media(&self, version: Version, source: Option<&MetadataToken>) -> bool {
        media_clock_matches(self.version, version, self.facts.2) && self.same_source(source)
    }
}

#[cfg(test)]
pub(super) fn media_memo_counts(metadata: &MetadataToken, paired: bool) -> [u64; 3] {
    use std::sync::atomic::Ordering::Relaxed;
    let metadata = metadata.downcast_ref::<Metadata>().unwrap();
    let memo = if paired {
        &metadata.paired
    } else {
        &metadata.unary
    };
    [
        memo.1.derive.load(Relaxed),
        memo.1.hit.load(Relaxed),
        memo.1.publish.load(Relaxed),
    ]
}

// Only MediaFacts opt into their proven positive validity interval. Lifetime,
// peek, signal and generic compound projections keep exact-clock matching.
// Every invocation still takes current SQL and file authority and seals both.
pub(super) fn cached_media(
    store: &Store,
    memo: &Memo<MediaFacts>,
    source: Option<&MetadataToken>,
    held: Option<&Arc<AuthorityGuardSet>>,
    derive: impl FnMut(&Connection) -> Result<(MediaFacts, Vec<AuthorityScope>)>,
    finish: impl FnOnce(&Connection) -> Result<()>,
) -> Result<(MediaFacts, Arc<AuthorityGuardSet>)> {
    cached_with_matching(
        store,
        memo,
        source,
        held,
        derive,
        |c, _| finish(c),
        Entry::matches_current_media,
    )
}

#[cfg(test)]
pub(super) fn continuation_memo_counts(metadata: &MetadataToken) -> [u64; 3] {
    use std::sync::atomic::Ordering::Relaxed;
    let memo = &metadata.downcast_ref::<Metadata>().unwrap().continuation;
    [
        memo.1.derive.load(Relaxed),
        memo.1.hit.load(Relaxed),
        memo.1.publish.load(Relaxed),
    ]
}

/// Read already-validated metadata for an observation that makes no effect.
/// This owns no authority reader and must never be used as an IO/control grant.
/// Exact SQL/clock/source stamps still apply; a miss requires normal admission.
pub(super) fn peek_map<T, U>(
    store: &Store,
    memo: &Memo<T>,
    source: Option<&MetadataToken>,
    project: impl FnOnce(&T) -> U,
) -> Result<Option<U>> {
    let observation = store.packet_connections.observe()?;
    let c = store.try_packet_connection(&observation)?;
    let before = version(&c, c.id())?;
    let observed = memo.0.try_lock().ok().and_then(|mut slot| {
        slot.retain(|entry| observation.current(entry.version.connection));
        slot.iter()
            .find(|entry| entry.matches(before, source))
            .map(|entry| project(&entry.facts))
    });
    if version(&c, c.id())? != before {
        return Err(Error::new("authority-busy", "Observation metadata changed"));
    }
    Ok(observed)
}

pub(super) fn cached<T: Clone>(
    store: &Store,
    memo: &Memo<T>,
    source: Option<&MetadataToken>,
    derive: impl FnMut(&Connection) -> Result<(T, Vec<AuthorityScope>)>,
    finish: impl FnOnce(&Connection) -> Result<()>,
) -> Result<(T, Arc<AuthorityGuardSet>)> {
    cached_with(store, memo, source, None, derive, |c, _| finish(c))
}

/// Recheck current metadata under an active caller-owned complete interval.
/// This lends file custody only; no fact or permission comes from the holder.
pub(super) fn cached_under<T: Clone>(
    store: &Store,
    memo: &Memo<T>,
    source: Option<&MetadataToken>,
    held: &Arc<AuthorityGuardSet>,
    derive: impl FnMut(&Connection) -> Result<(T, Vec<AuthorityScope>)>,
    finish: impl FnOnce(&Connection) -> Result<()>,
) -> Result<(T, Arc<AuthorityGuardSet>)> {
    cached_with(store, memo, source, Some(held), derive, |c, _| finish(c))
}

fn cached_with<T: Clone>(
    store: &Store,
    memo: &Memo<T>,
    source: Option<&MetadataToken>,
    held: Option<&Arc<AuthorityGuardSet>>,
    derive: impl FnMut(&Connection) -> Result<(T, Vec<AuthorityScope>)>,
    finish: impl FnOnce(&Connection, &T) -> Result<()>,
) -> Result<(T, Arc<AuthorityGuardSet>)> {
    cached_with_matching(store, memo, source, held, derive, finish, Entry::matches)
}

fn cached_with_matching<T: Clone>(
    store: &Store,
    memo: &Memo<T>,
    source: Option<&MetadataToken>,
    held: Option<&Arc<AuthorityGuardSet>>,
    mut derive: impl FnMut(&Connection) -> Result<(T, Vec<AuthorityScope>)>,
    finish: impl FnOnce(&Connection, &T) -> Result<()>,
    matches: impl Fn(&Entry<T>, Version, Option<&MetadataToken>) -> bool,
) -> Result<(T, Arc<AuthorityGuardSet>)> {
    let observation = store.packet_connections.observe()?;
    if let Ok(mut entries) = memo.0.try_lock() {
        entries.retain(|entry| observation.current(entry.version.connection));
    }
    let mut captured = None;
    let mut capture = |c: &Connection, version: Version| {
        if let Ok(entry) = memo.0.try_lock() {
            if let Some(entry) = entry.iter().find(|entry| matches(entry, version, source)) {
                #[cfg(test)]
                memo.1
                    .hit
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return Ok((entry.facts.clone(), entry.scopes.clone()));
            }
        }
        #[cfg(test)]
        memo.1
            .derive
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (facts, scopes) = derive(c)?;
        let facts = Arc::new(facts);
        captured = Some(Entry {
            version,
            source: source.map(Arc::downgrade),
            facts: facts.clone(),
            scopes: scopes.clone(),
        });
        Ok((facts, scopes))
    };
    let mut finish = Some(|c: &Connection, facts: &Arc<T>| finish(c, facts.as_ref()));
    let mut optimistic = None;
    if held.is_none() {
        // Copy metadata outside SQL and use only its old scope names to try a
        // new file interval. It lends no grant. Resolve current facts under
        // this reader and one SQL interval, rederiving on any stamp mismatch.
        // Only complete current scope coverage permits sealing; otherwise drop
        // SQL and the disposable reader before normal derivation/admission.
        let previous = memo.0.try_lock().ok().and_then(|slot| {
            slot.iter()
                .find(|entry| {
                    observation.current(entry.version.connection) && entry.same_source(source)
                })
                .cloned()
        });
        if let Some(previous) = previous {
            if let Ok(reader) = store.try_acquire_authority(&previous.scopes) {
                let c = store.try_packet_connection(&observation)?;
                let before = version(&c, c.id())?;
                // A commit invalidates facts; a clock edge does so for generic
                // metadata or outside a positive MediaFacts validity interval.
                // Neither necessarily changes the scope set. Releasing this valid fresh reader
                // and SQL merely to acquire the same scopes again creates a
                // needless exact-observer reacquisition race. capture still
                // checks this observer's exact version and original source.
                let (facts, scopes) = current_refusal(&c, before, capture(&c, before))?;
                if store.authority_covers(&reader, &scopes) {
                    optimistic = Some(seal(
                        store,
                        &c,
                        before,
                        facts,
                        scopes,
                        reader,
                        &mut capture,
                        finish.take().ok_or_else(Error::storage)?,
                    ));
                }
            }
        }
    }
    let result = match optimistic {
        Some(result) => result,
        None => match held {
            Some(held) => acquire_under_at(
                store,
                &observation,
                &mut capture,
                finish.take().ok_or_else(Error::storage)?,
                held,
            ),
            None => acquire_with_facts_at(
                store,
                &observation,
                &mut capture,
                finish.take().ok_or_else(Error::storage)?,
                |scopes| store.try_acquire_authority(scopes),
            ),
        },
    }?;
    // Publish only a fully admitted unchanged capture. Neither cache lock nor
    // SQL is retained by the caller's owned fresh authority guard.
    // An optimistic capture with uncovered scopes can precede a fallback memo
    // hit on another observer. Publish only the actual successfully sealed facts.
    if let Some(entry) = captured.filter(|entry| Arc::ptr_eq(&entry.facts, &result.0)) {
        if let Ok(mut slot) = memo.0.try_lock() {
            #[cfg(test)]
            memo.1
                .publish
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            // Drop dead original sources and superseded versions for this
            // observer. A later media capture does not evict another unchanged
            // positive whose ORIGINAL validity interval still covers it.
            // Generic matching retains the previous exact-version behavior.
            // Retained facts own no reader or grant.
            slot.retain(|previous| {
                observation.current(previous.version.connection)
                    && previous
                        .source
                        .as_ref()
                        .is_none_or(|source| source.strong_count() > 0)
                    && (previous.version.connection != entry.version.connection
                        || matches(
                            previous,
                            entry.version,
                            previous.source.as_ref().and_then(Weak::upgrade).as_ref(),
                        ))
            });
            if let Some(previous) = slot.iter_mut().find(|previous| {
                previous.version.connection == entry.version.connection
                    && previous.same_source(source)
            }) {
                *previous = entry;
            } else {
                let per_observer = if source.is_some() {
                    SOURCES_PER_OBSERVER
                } else {
                    1
                };
                while slot
                    .iter()
                    .filter(|previous| previous.version.connection == entry.version.connection)
                    .count()
                    >= per_observer
                {
                    if let Some(index) = slot.iter().position(|previous| {
                        previous.version.connection == entry.version.connection
                    }) {
                        slot.remove(index);
                    } else {
                        break;
                    }
                }
                if slot.len() >= crate::packet_observation::OBSERVERS * SOURCES_PER_OBSERVER {
                    slot.remove(0);
                }
                slot.push(entry);
            }
        }
    }
    // Only immutable metadata references are copied under SQL. A potentially
    // large caller projection is cloned after both SQL and the memo mutex end;
    // the separately acquired current file interval remains caller-owned.
    Ok((result.0.as_ref().clone(), result.1))
}

/// Capture under short immediate SQL custody, release SQL for all file admission,
/// then reacquire SQL and seal the exact unchanged version under the new reader.
/// No caller waits with SQL or retains a partial reader while retrying.
impl Store {
    /// Build a fresh compound read under one complete current file interval.
    /// Command SQL is never acquired. The same observer's SQL/schema/clock
    /// stamp is sealed around both derivation and the final read construction;
    /// no metadata, partial reader or permission survives a refused capture.
    /// Pure final construction can run again at a clock edge, under that SAME
    /// interval and the first derivation's upper expiry, never another reader.
    pub(crate) fn try_current_packet_projection<T, U>(
        &self,
        derive: impl FnMut(&Connection) -> Result<(T, Vec<AuthorityScope>, i64)>,
        finish: impl FnMut(&Connection, &T, Arc<AuthorityGuardSet>, i64) -> Result<U>,
    ) -> Result<U> {
        self.current_packet_projection_with_custody(derive, finish, None, None)
    }
    /// Derive the same complete current facts and reseal the same SQL observer.
    /// Only an exact still-owned packet interval can replace file admission;
    /// uncovered scopes refuse without acquiring another reader or falling back.
    pub(crate) fn try_current_packet_projection_under<T, U>(
        &self,
        derive: impl FnMut(&Connection) -> Result<(T, Vec<AuthorityScope>, i64)>,
        finish: impl FnMut(&Connection, &T, Arc<AuthorityGuardSet>, i64) -> Result<U>,
        held: &Arc<AuthorityGuardSet>,
        original_expiry: i64,
    ) -> Result<U> {
        self.current_packet_projection_with_custody(
            derive,
            finish,
            Some((held, original_expiry)),
            None,
        )
    }
    // Same compound boundary as uncached projection: the memo contains only
    // immutable SQL facts. Every invocation still admits current authority and
    // performs the original AFTER-finish database/clock seal.
    pub(crate) fn try_memoized_current_packet_projection<T: Send + Sync + 'static, U>(
        &self,
        metadata: &(dyn std::any::Any + Send + Sync),
        derive: impl FnMut(&Connection) -> Result<(T, Vec<AuthorityScope>, i64)>,
        finish: impl FnMut(&Connection, &T, Arc<AuthorityGuardSet>, i64) -> Result<U>,
        custody: Option<(&Arc<AuthorityGuardSet>, i64)>,
    ) -> Result<U> {
        let memo = metadata
            .downcast_ref::<Memo<(T, i64)>>()
            .ok_or_else(Error::storage)?;
        self.current_packet_projection_with_custody(derive, finish, custody, Some(memo))
    }
    #[cfg(test)]
    pub(crate) fn packet_projection_memo_counts_for_test<T: Send + Sync + 'static>(
        metadata: &(dyn std::any::Any + Send + Sync),
    ) -> [u64; 3] {
        use std::sync::atomic::Ordering::Relaxed;
        let memo = metadata.downcast_ref::<Memo<(T, i64)>>().unwrap();
        [
            memo.1.derive.load(Relaxed),
            memo.1.hit.load(Relaxed),
            memo.1.publish.load(Relaxed),
        ]
    }
    fn current_packet_projection_with_custody<T, U>(
        &self,
        derive: impl FnMut(&Connection) -> Result<(T, Vec<AuthorityScope>, i64)>,
        mut finish: impl FnMut(&Connection, &T, Arc<AuthorityGuardSet>, i64) -> Result<U>,
        custody: Option<(&Arc<AuthorityGuardSet>, i64)>,
        memo: Option<&Memo<(T, i64)>>,
    ) -> Result<U> {
        let derive = std::cell::RefCell::new(derive);
        let captured = std::cell::RefCell::new(None::<Entry<(T, i64)>>);
        let sealed_metadata = std::cell::RefCell::new(None::<Arc<(T, i64)>>);
        let expires = std::cell::Cell::new(custody.map_or(i64::MAX, |(_, expiry)| expiry));
        let capture = |c: &Connection, stamp| {
            if let Some(memo) = memo {
                let hit = memo.0.try_lock().ok().and_then(|entries| {
                    entries
                        .iter()
                        .find(|entry| entry.matches(stamp, None))
                        .map(|entry| (entry.facts.clone(), entry.scopes.clone()))
                });
                if let Some((facts, scopes)) = hit {
                    expires.set(expires.get().min(facts.1));
                    if crate::now() >= expires.get() {
                        return Err(Error::forbidden());
                    }
                    #[cfg(test)]
                    memo.1
                        .hit
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    return Ok(((ProjectionCapture::Shared(facts), stamp), scopes));
                }
                #[cfg(test)]
                memo.1
                    .derive
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            let (facts, scopes, until) = derive.borrow_mut()(c)?;
            expires.set(expires.get().min(until));
            if crate::now() >= expires.get() {
                return Err(Error::forbidden());
            }
            let facts = if memo.is_some() {
                let facts = Arc::new((facts, until));
                captured.replace(Some(Entry {
                    version: stamp,
                    source: None,
                    facts: facts.clone(),
                    scopes: scopes.clone(),
                }));
                ProjectionCapture::Shared(facts)
            } else {
                ProjectionCapture::Owned(facts)
            };
            Ok(((facts, stamp), scopes))
        };
        let interval = std::cell::RefCell::new(None::<Arc<AuthorityGuardSet>>);
        let mut projection = None;
        let observation = self.packet_connections.observe()?;
        acquire_with_facts_at(
            self,
            &observation,
            capture,
            |c, (facts, stamp)| {
                let held = interval.borrow().clone().ok_or_else(Error::storage)?;
                let mut stamp = *stamp;
                let mut fresh = None;
                let mut refresh = false;
                // Bound pure metadata reconstruction. A second clock edge can
                // revalidate once more; no SQL/schema change can be retried.
                for _ in 0..3 {
                    let before = version(c, stamp.connection)?;
                    if !stamp.same_database(before) {
                        return Err(Error::new(
                            "authority-busy",
                            "Current packet state changed before projection",
                        ));
                    }
                    if refresh || before.second != stamp.second {
                        let ((current, seen), scopes) =
                            current_refusal(c, before, capture(c, before))?;
                        if !self.authority_covers(&held, &scopes) {
                            return Err(Error::forbidden());
                        }
                        fresh = Some(current);
                        stamp = seen;
                    }
                    if version(c, stamp.connection)? != stamp {
                        refresh = true;
                        continue;
                    }
                    let value = finish(
                        c,
                        fresh.as_ref().unwrap_or(facts).facts(),
                        held.clone(),
                        expires.get(),
                    );
                    let after = version(c, stamp.connection)?;
                    if after == stamp {
                        projection = Some(value?);
                        if let ProjectionCapture::Shared(facts) = fresh.as_ref().unwrap_or(facts) {
                            sealed_metadata.replace(Some(facts.clone()));
                        }
                        return Ok(());
                    }
                    // Even a completed compound read is discarded before any
                    // fresh clock derivation. Its guards lend no new admission.
                    drop(value);
                    if !stamp.same_database(after) {
                        return Err(Error::new(
                            "authority-busy",
                            "Current packet state changed during projection",
                        ));
                    }
                    refresh = true;
                }
                Err(Error::new(
                    "authority-busy",
                    "Current packet clock kept changing during projection",
                ))
            },
            |scopes| {
                let held = match custody {
                    Some((held, expiry)) => {
                        if crate::now() >= expiry || !self.authority_covers(held, scopes) {
                            return Err(Error::forbidden());
                        }
                        held.clone()
                    }
                    None => self.try_acquire_authority(scopes)?,
                };
                interval.replace(Some(held.clone()));
                Ok(held)
            },
        )?;
        if let (Some(memo), Some(entry), Some(sealed)) = (
            memo,
            captured.borrow_mut().take(),
            sealed_metadata.borrow_mut().take(),
        ) {
            if Arc::ptr_eq(&entry.facts, &sealed) {
                if let Ok(mut entries) = memo.0.try_lock() {
                    entries.retain(|old| {
                        observation.current(old.version.connection)
                            && old.version.connection != entry.version.connection
                    });
                    if entries.len() >= crate::packet_observation::OBSERVERS {
                        entries.remove(0);
                    }
                    entries.push(entry);
                    #[cfg(test)]
                    memo.1
                        .publish
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }
        projection.ok_or_else(Error::storage)
    }

    /// Type-erased immutable facts only, exactly like a native origin's memo.
    /// Entries never own SQL, file authority, execution or a media IO grant.
    pub(crate) fn new_packet_projection_metadata<T: Send + Sync + 'static>(
    ) -> Arc<dyn std::any::Any + Send + Sync> {
        Arc::new(Memo::<T>::default())
    }

    /// Share native SQL/schema/clock/source memoization, fresh complete file
    /// admission and sealing. A hit skips projection work, never authorization.
    /// `finish` sees the actual sealed facts on both a hit and a miss.
    pub(crate) fn try_cached_packet_projection<T: Clone + Send + Sync + 'static>(
        &self,
        metadata: &(dyn std::any::Any + Send + Sync),
        source: Option<&Arc<dyn std::any::Any + Send + Sync>>,
        derive: impl FnMut(&Connection) -> Result<(T, Vec<AuthorityScope>)>,
        finish: impl FnOnce(&Connection, &T) -> Result<()>,
    ) -> Result<(T, Arc<AuthorityGuardSet>)> {
        let memo = metadata
            .downcast_ref::<Memo<T>>()
            .ok_or_else(Error::storage)?;
        cached_with(self, memo, source, None, derive, finish)
    }
}

pub(super) fn acquire<T>(
    store: &Store,
    mut derive: impl FnMut(&Connection) -> Result<(T, Vec<AuthorityScope>)>,
) -> Result<(T, Arc<AuthorityGuardSet>)> {
    acquire_at(store, |c, _| derive(c), |_| Ok(()))
}

fn acquire_at<T>(
    store: &Store,
    derive: impl FnMut(&Connection, Version) -> Result<(T, Vec<AuthorityScope>)>,
    finish: impl FnOnce(&Connection) -> Result<()>,
) -> Result<(T, Arc<AuthorityGuardSet>)> {
    acquire_with(store, derive, finish, |scopes| {
        store.try_acquire_authority(scopes)
    })
}

fn acquire_with<T>(
    store: &Store,
    derive: impl FnMut(&Connection, Version) -> Result<(T, Vec<AuthorityScope>)>,
    finish: impl FnOnce(&Connection) -> Result<()>,
    admit: impl FnOnce(&[AuthorityScope]) -> Result<Arc<AuthorityGuardSet>>,
) -> Result<(T, Arc<AuthorityGuardSet>)> {
    acquire_with_facts(store, derive, |c, _| finish(c), admit)
}

fn acquire_with_facts<T>(
    store: &Store,
    derive: impl FnMut(&Connection, Version) -> Result<(T, Vec<AuthorityScope>)>,
    finish: impl FnOnce(&Connection, &T) -> Result<()>,
    admit: impl FnOnce(&[AuthorityScope]) -> Result<Arc<AuthorityGuardSet>>,
) -> Result<(T, Arc<AuthorityGuardSet>)> {
    let observation = store.packet_connections.observe()?;
    acquire_with_facts_at(store, &observation, derive, finish, admit)
}
fn acquire_with_facts_at<T>(
    store: &Store,
    observation: &Observation,
    mut derive: impl FnMut(&Connection, Version) -> Result<(T, Vec<AuthorityScope>)>,
    finish: impl FnOnce(&Connection, &T) -> Result<()>,
    admit: impl FnOnce(&[AuthorityScope]) -> Result<Arc<AuthorityGuardSet>>,
) -> Result<(T, Arc<AuthorityGuardSet>)> {
    // This local resource-free reservation ends before any current read,
    // authority interval or memoized facts are returned. SQL still ends before
    // file admission, but another observation cannot steal this exact slot.
    let reservation = store.packet_connections.reserve(observation)?;
    let result = (|| {
        let (before, facts, scopes) = {
            let c = store.try_packet_connection_reserved(observation, &reservation)?;
            let before = version(&c, c.id())?;
            let (facts, scopes) = current_refusal(&c, before, derive(&c, before))?;
            (before, facts, scopes)
        };
        // Original SQL guard has ended; the exact slot remains claimed.
        reservation.begin_file_admission();
        let held = admit(&scopes)?;
        reservation.begin_reseal();
        let c = store.try_packet_connection_reserved(observation, &reservation)?;
        if c.id() != before.connection {
            return Err(Error::storage());
        }
        seal(store, &c, before, facts, scopes, held, &mut derive, finish)
    })();
    reservation.observe_result(result.is_ok());
    result
}

// The complete file interval is already owned. Coverage checks perform no file
// admission or IO, so retain one short SQL interval through capture and sealing.
// External commits and clock edges still require the same exact version checks.
/// One complete current projection under an original caller-owned interval.
/// Retains neither SQL nor a new/partial authority reader after return.
pub(super) fn current_under<T>(
    store: &Store,
    held: &Arc<AuthorityGuardSet>,
    mut derive: impl FnMut(&Connection) -> Result<(T, Vec<AuthorityScope>)>,
) -> Result<T> {
    acquire_under(store, |c, _| derive(c), |_, _| Ok(()), held).map(|(facts, _)| facts)
}

fn acquire_under<T>(
    store: &Store,
    derive: impl FnMut(&Connection, Version) -> Result<(T, Vec<AuthorityScope>)>,
    finish: impl FnOnce(&Connection, &T) -> Result<()>,
    held: &Arc<AuthorityGuardSet>,
) -> Result<(T, Arc<AuthorityGuardSet>)> {
    let observation = store.packet_connections.observe()?;
    acquire_under_at(store, &observation, derive, finish, held)
}
fn acquire_under_at<T>(
    store: &Store,
    observation: &Observation,
    mut derive: impl FnMut(&Connection, Version) -> Result<(T, Vec<AuthorityScope>)>,
    finish: impl FnOnce(&Connection, &T) -> Result<()>,
    held: &Arc<AuthorityGuardSet>,
) -> Result<(T, Arc<AuthorityGuardSet>)> {
    let c = store.try_packet_connection(observation)?;
    let before = version(&c, c.id())?;
    let (facts, scopes) = current_refusal(&c, before, derive(&c, before))?;
    if !store.authority_covers(held, &scopes) {
        return Err(Error::forbidden());
    }
    seal(
        store,
        &c,
        before,
        facts,
        scopes,
        held.clone(),
        &mut derive,
        finish,
    )
}

fn seal<T>(
    store: &Store,
    c: &PacketConnectionGuard<'_>,
    before: Version,
    mut facts: T,
    mut scopes: Vec<AuthorityScope>,
    held: Arc<AuthorityGuardSet>,
    derive: &mut impl FnMut(&Connection, Version) -> Result<(T, Vec<AuthorityScope>)>,
    finish: impl FnOnce(&Connection, &T) -> Result<()>,
) -> Result<(T, Arc<AuthorityGuardSet>)> {
    let after = version(c, c.id())?;
    if (Version {
        second: before.second,
        ..after
    }) != before
    {
        return Err(Error::new(
            "authority-busy",
            "Current packet state changed during admission",
        ));
    }
    if after.second != before.second {
        // Revalidate a clock edge under this already-owned reader. No second
        // file admission behind a queued writer and no original expiry extension.
        (facts, scopes) = current_refusal(c, after, derive(c, after))?;
        if version(c, c.id())? != after {
            return Err(Error::new(
                "authority-busy",
                "Current packet state changed during clock revalidation",
            ));
        }
    }
    if !store.authority_covers(&held, &scopes) {
        return Err(Error::forbidden());
    }
    finish(c, &facts)?;
    Ok((facts, held))
}

#[cfg(test)]
mod tests;
