//! SQL-versioned metadata; fresh admission or exact active file custody.
use super::*;
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
    connection: usize,
    external: i64,
    schema: i64,
    changes: u64,
    second: i64,
}
fn version(c: &Connection) -> Result<Version> {
    let external = c
        .query_row("PRAGMA data_version", [], |row| row.get(0))
        .map_err(|_| Error::storage())?;
    Ok(Version {
        connection: c as *const Connection as usize,
        external,
        schema: c
            .query_row("PRAGMA schema_version", [], |row| row.get(0))
            .map_err(|_| Error::storage())?,
        changes: c.total_changes(),
        second: crate::now(),
    })
}

type MediaFacts = (
    PeerConnectionProjection,
    Option<PeerConnectionProjection>,
    i64,
    bool,
);
/// At most three entries per original live origin, independent of packet count.
/// No reader, execution handle or IO grant can be retained by this metadata.
#[derive(Default)]
pub(super) struct Metadata {
    pub(super) lifetime: Memo<i64>,
    pub(super) unary: Memo<MediaFacts>,
    pub(super) paired: Memo<MediaFacts>,
}
pub(super) struct Memo<T>(Mutex<Option<Entry<T>>>);
impl<T> Default for Memo<T> {
    fn default() -> Self {
        Self(Mutex::new(None))
    }
}
#[derive(Clone)]
struct Entry<T> {
    version: Version,
    source: Option<Weak<ErasedMetadata>>,
    facts: T,
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

/// Read already-validated metadata for an observation that makes no effect.
/// This owns no authority reader and must never be used as an IO/control grant.
/// Exact SQL/clock/source stamps still apply; a miss requires normal admission.
pub(super) fn peek_map<T, U>(
    store: &Store,
    memo: &Memo<T>,
    source: Option<&MetadataToken>,
    project: impl FnOnce(&T) -> U,
) -> Result<Option<U>> {
    let c = store.try_connection()?;
    let before = version(&c)?;
    let observed = memo.0.try_lock().ok().and_then(|slot| {
        slot.as_ref()
            .filter(|entry| entry.matches(before, source))
            .map(|entry| project(&entry.facts))
    });
    if version(&c)? != before {
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
    cached_with(store, memo, source, None, derive, finish)
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
    cached_with(store, memo, source, Some(held), derive, finish)
}

fn cached_with<T: Clone>(
    store: &Store,
    memo: &Memo<T>,
    source: Option<&MetadataToken>,
    held: Option<&Arc<AuthorityGuardSet>>,
    mut derive: impl FnMut(&Connection) -> Result<(T, Vec<AuthorityScope>)>,
    finish: impl FnOnce(&Connection) -> Result<()>,
) -> Result<(T, Arc<AuthorityGuardSet>)> {
    let mut captured = None;
    let mut capture = |c: &Connection, version: Version| {
        if let Ok(entry) = memo.0.try_lock() {
            if let Some(entry) = entry
                .as_ref()
                .filter(|entry| entry.matches(version, source))
            {
                return Ok((entry.facts.clone(), entry.scopes.clone()));
            }
        }
        let (facts, scopes) = derive(c)?;
        captured = Some(Entry {
            version,
            source: source.map(Arc::downgrade),
            facts: facts.clone(),
            scopes: scopes.clone(),
        });
        Ok((facts, scopes))
    };
    let mut finish = Some(finish);
    let mut optimistic = None;
    if held.is_none() {
        // Copy metadata outside SQL and use only its old scope names to try a
        // new file interval. It lends no grant. The exact current SQL/clock/
        // source version must still match under that new interval. A mismatch
        // drops both SQL and the disposable reader before normal derivation.
        let previous = memo.0.try_lock().ok().and_then(|slot| {
            slot.as_ref()
                .filter(|entry| entry.same_source(source))
                .cloned()
        });
        if let Some(previous) = previous {
            if let Ok(reader) = store.try_acquire_authority(&previous.scopes) {
                let c = store.try_connection()?;
                let before = version(&c)?;
                if previous.matches(before, source) {
                    optimistic = Some(seal(
                        store,
                        &c,
                        before,
                        previous.facts,
                        previous.scopes,
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
            Some(held) => acquire_under(
                store,
                &mut capture,
                finish.take().ok_or_else(Error::storage)?,
                held,
            ),
            None => acquire_with(
                store,
                &mut capture,
                finish.take().ok_or_else(Error::storage)?,
                |scopes| store.try_acquire_authority(scopes),
            ),
        },
    }?;
    // Publish only a fully admitted unchanged capture. Neither cache lock nor
    // SQL is retained by the caller's owned fresh authority guard.
    if let Some(entry) = captured {
        if let Ok(mut slot) = memo.0.try_lock() {
            *slot = Some(entry);
        }
    }
    Ok(result)
}

/// Capture under short immediate SQL custody, release SQL for all file admission,
/// then reacquire SQL and seal the exact unchanged version under the new reader.
/// No caller waits with SQL or retains a partial reader while retrying.
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
    mut derive: impl FnMut(&Connection, Version) -> Result<(T, Vec<AuthorityScope>)>,
    finish: impl FnOnce(&Connection) -> Result<()>,
    admit: impl FnOnce(&[AuthorityScope]) -> Result<Arc<AuthorityGuardSet>>,
) -> Result<(T, Arc<AuthorityGuardSet>)> {
    let (before, facts, scopes) = {
        let c = store.try_connection()?;
        let before = version(&c)?;
        let (facts, scopes) = derive(&c, before)?;
        (before, facts, scopes)
    };
    // File opens, hashing and try-locks over a large scene must not monopolize
    // SQL. This remains immediate fresh admission, with every original scope.
    let held = admit(&scopes)?;
    let c = store.try_connection()?;
    seal(store, &c, before, facts, scopes, held, &mut derive, finish)
}

// The complete file interval is already owned. Coverage checks perform no file
// admission or IO, so retain one short SQL interval through capture and sealing.
// External commits and clock edges still require the same exact version checks.
fn acquire_under<T>(
    store: &Store,
    mut derive: impl FnMut(&Connection, Version) -> Result<(T, Vec<AuthorityScope>)>,
    finish: impl FnOnce(&Connection) -> Result<()>,
    held: &Arc<AuthorityGuardSet>,
) -> Result<(T, Arc<AuthorityGuardSet>)> {
    let c = store.try_connection()?;
    let before = version(&c)?;
    let (facts, scopes) = derive(&c, before)?;
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
    c: &Connection,
    before: Version,
    mut facts: T,
    mut scopes: Vec<AuthorityScope>,
    held: Arc<AuthorityGuardSet>,
    derive: &mut impl FnMut(&Connection, Version) -> Result<(T, Vec<AuthorityScope>)>,
    finish: impl FnOnce(&Connection) -> Result<()>,
) -> Result<(T, Arc<AuthorityGuardSet>)> {
    let after = version(c)?;
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
        (facts, scopes) = derive(c, after)?;
        if version(c)? != after {
            return Err(Error::new(
                "authority-busy",
                "Current packet state changed during clock revalidation",
            ));
        }
    }
    if !store.authority_covers(&held, &scopes) {
        return Err(Error::forbidden());
    }
    finish(c)?;
    Ok((facts, held))
}

#[cfg(test)]
mod tests;
