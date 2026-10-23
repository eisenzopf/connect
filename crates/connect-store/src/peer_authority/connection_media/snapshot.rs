//! One fresh SQL capture through nonblocking authority admission, never a cache.
use super::*;

fn version(c: &Connection) -> Result<(i64, u64)> {
    let external = c
        .query_row("PRAGMA data_version", [], |row| row.get(0))
        .map_err(|_| Error::storage())?;
    Ok((external, c.total_changes()))
}

/// The caller retains its immediate Store connection mutex until return. No
/// authority wait is allowed here: a queued writer must release SQL immediately.
/// Other Store instances/processes have independent SQL mutexes, so an external
/// commit during derivation/admission invalidates the entire capture. A failed
/// capture drops every partial reader before the caller can retry. After success,
/// the complete owned scope set prevents relevant mutations during actual IO.
pub(super) fn acquire<T>(
    store: &Store,
    c: &Connection,
    derive: impl FnOnce(&Connection) -> Result<(T, Vec<AuthorityScope>)>,
) -> Result<(T, Arc<AuthorityGuardSet>)> {
    let before = version(c)?;
    let (facts, scopes) = derive(c)?;
    let held = store.try_acquire_authority(&scopes)?;
    if version(c)? != before {
        return Err(Error::new(
            "authority-busy",
            "Current packet state changed during admission",
        ));
    }
    if !store.authority_covers(&held, &scopes) {
        return Err(Error::forbidden());
    }
    Ok((facts, held))
}

#[cfg(test)]
mod tests;
