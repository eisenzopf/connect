//! Bounded, exact startup targets. Installation ownership is separate from the
//! per-resource write fences held through each immediate maintenance transaction.
use crate::{AuthorityGuardSet, AuthorityMode, AuthorityScope, AuthorityWait, Store};
use connect_protocol::{Error, Result};
use rusqlite::{params, Connection};
use std::{collections::BTreeSet, sync::Arc};

#[derive(Clone, Copy)]
pub(crate) enum Kind {
    ConnectionEpoch,
    VideoBindings,
}
pub(crate) struct Candidate {
    pub id: String,
    cid: String,
    owner: Option<String>,
}
impl Candidate {
    fn scopes(&self) -> Result<Vec<AuthorityScope>> {
        let mut result = vec![AuthorityScope::conversation(
            &self.cid,
            AuthorityMode::Write,
        )?];
        if let Some(owner) = &self.owner {
            result.push(AuthorityScope::identity(owner, AuthorityMode::Write)?);
        }
        Ok(result)
    }
}
impl Kind {
    fn query(self) -> &'static str {
        match self {
            Self::ConnectionEpoch => "SELECT c.id,s.conversation_id,c.identity_id FROM connections c JOIN sessions s ON s.id=c.session_id WHERE c.state IN ('preparing','ready','active','held','retiring') AND (c.observed_runtime_id IS NULL OR c.observed_runtime_id!=?1)",
            Self::VideoBindings => "SELECT s.id,s.conversation_id,NULL FROM sessions s WHERE EXISTS(SELECT 1 FROM session_bindings b WHERE b.session_id=s.id AND b.state='ready') AND EXISTS(SELECT 1 FROM json_each(s.media) m WHERE m.value='video')",
        }
    }
    fn id_column(self) -> &'static str {
        match self {
            Self::ConnectionEpoch => "c.id",
            Self::VideoBindings => "s.id",
        }
    }
}
fn select(
    epoch: &str,
    c: &Connection,
    kind: Kind,
    limit: u32,
    record: Option<&str>,
) -> Result<Vec<Candidate>> {
    let sql = if record.is_some() {
        format!("{} AND {}=?2", kind.query(), kind.id_column())
    } else {
        format!("{} ORDER BY {} LIMIT ?2", kind.query(), kind.id_column())
    };
    let value = record
        .map(|id| rusqlite::types::Value::Text(id.to_owned()))
        .unwrap_or_else(|| i64::from(limit).into());
    let mut st = c.prepare(&sql).map_err(|_| Error::storage())?;
    let result = st
        .query_map(params![epoch, value], |r| {
            Ok(Candidate {
                id: r.get(0)?,
                cid: r.get(1)?,
                owner: r.get(2)?,
            })
        })
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    Ok(result)
}
fn prefix(
    epoch: &str,
    c: &Connection,
    kind: Kind,
    limit: u32,
) -> Result<(Vec<String>, Vec<AuthorityScope>)> {
    let mut ids = vec![];
    let mut scopes = BTreeSet::new();
    for candidate in select(epoch, c, kind, limit, None)? {
        let extra = candidate.scopes()?;
        if scopes.len() + extra.iter().filter(|s| !scopes.contains(*s)).count() > 64 {
            break;
        }
        scopes.extend(extra);
        ids.push(candidate.id);
    }
    Ok((ids, scopes.into_iter().collect()))
}
impl Store {
    pub(crate) fn startup_batch(
        &self,
        kind: Kind,
        limit: u32,
    ) -> Result<(Vec<String>, Arc<AuthorityGuardSet>)> {
        let _runtime = self.require_runtime_owner()?;
        if limit == 0 || limit > 1000 {
            return Err(Error::new("invalid-request", "Invalid startup batch limit"));
        }
        let wait = AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT);
        loop {
            let (_, scopes) = prefix(self.runtime_epoch(), &*self.connection()?, kind, limit)?;
            let guard = self.acquire_authority(&scopes, &wait)?;
            let (ids, scopes) = prefix(self.runtime_epoch(), &*self.connection()?, kind, limit)?;
            if self.authority_covers(&guard, &scopes) {
                return Ok((ids, guard));
            }
        }
    }
    pub(crate) fn startup_rows(
        &self,
        c: &Connection,
        kind: Kind,
        ids: &[String],
        guard: &AuthorityGuardSet,
    ) -> Result<Vec<Candidate>> {
        let _runtime = self.require_runtime_owner()?;
        let mut result = vec![];
        for id in ids {
            for candidate in select(self.runtime_epoch(), c, kind, 1, Some(id))? {
                if !self.authority_covers(guard, &candidate.scopes()?) {
                    return Err(Error::new(
                        "authority-unavailable",
                        "Startup authority targets changed",
                    ));
                }
                result.push(candidate);
            }
        }
        Ok(result)
    }
}
