//! Metadata memoization only. It never owns a grant, reader, credential or body.
//! Native reads still acquire/check current authority on both sides of lookup.
use super::*;
use std::{collections::BTreeMap, sync::Mutex};
#[derive(Clone, Copy, PartialEq, Eq)]
struct Version {
    data: i64,
    schema: i64,
    changes: u64,
    second: i64,
}
struct Entry {
    version: Version,
    arguments: [u8; 32],
    expires: Option<i64>,
}
#[derive(Default)]
pub(crate) struct Cache(Mutex<BTreeMap<String, Entry>>);
fn version(c: &Connection) -> Result<Version> {
    Ok(Version {
        data: c
            .query_row("PRAGMA data_version", [], |r| r.get(0))
            .map_err(|_| Error::storage())?,
        schema: c
            .query_row("PRAGMA schema_version", [], |r| r.get(0))
            .map_err(|_| Error::storage())?,
        changes: c.total_changes(),
        second: crate::now(),
    })
}
pub(crate) fn ready_until(
    c: &Connection,
    store: &Store,
    root: &str,
    selection: &str,
    audience: &str,
) -> Result<Option<i64>> {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(b"private-provider-readiness-memo/1\0");
    hash.update((selection.len() as u64).to_be_bytes());
    hash.update(selection.as_bytes());
    hash.update(audience.as_bytes());
    let arguments: [u8; 32] = hash.finalize().into();
    for _ in 0..3 {
        let before = version(c)?;
        {
            let entries = store
                .private_provider_cache
                .0
                .lock()
                .map_err(|_| Error::storage())?;
            if let Some(entry) = entries.get(root) {
                if entry.version == before && entry.arguments == arguments {
                    return Ok(entry.expires.filter(|t| *t > before.second));
                }
            }
        }
        let result = super::authority::uncached_ready_until(c, store, root, selection, audience)?;
        // Own-connection writes (including rolled-back writes), other SQLite
        // connections, schema changes and every integer policy-clock boundary all
        // invalidate the memo. Never publish a result assembled across versions.
        let after = version(c)?;
        if after.data != before.data
            || after.schema != before.schema
            || after.changes != before.changes
        {
            return Err(Error::new(
                "authority-busy",
                "Private provider policy changed; retry current authority",
            ));
        }
        if after.second != before.second {
            continue;
        }

        let mut entries = store
            .private_provider_cache
            .0
            .lock()
            .map_err(|_| Error::storage())?;
        if entries.len() >= store.limits.connections.min(512).max(1) && !entries.contains_key(root)
        {
            entries.pop_first();
        }
        entries.insert(
            root.into(),
            Entry {
                version: before,
                arguments,
                expires: result,
            },
        );
        return Ok(result);
    }
    Err(Error::new(
        "authority-busy",
        "Private provider policy clock advanced; retry current authority",
    ))
}
