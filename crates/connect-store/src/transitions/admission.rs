//! Operation metadata writers own the owner's inbox and source Conversation.
//! Graph/role commits retain their broader member-Identity write footprint.
use super::*;
use crate::{AuthorityGuardSet, AuthorityMode, AuthorityScope, AuthorityWait};
use std::{collections::BTreeSet, sync::Arc};

pub(super) fn scopes(owner: &str, cid: Option<&str>) -> Result<Vec<AuthorityScope>> {
    let mut result = vec![AuthorityScope::identity(owner, AuthorityMode::Write)?];
    if let Some(cid) = cid {
        result.push(AuthorityScope::conversation(cid, AuthorityMode::Write)?);
    }
    Ok(result)
}
pub(super) fn row_scopes(row: &rows::Saved) -> Result<Vec<AuthorityScope>> {
    if row.native
        || row.session_codecs
        || row.native_stream
        || (row.native_consultation || row.native_provider)
        || (row.native_return || row.native_handoff || row.native_replacement)
        || row.native_decision
    {
        let mut result = vec![AuthorityScope::identity(&row.owner, AuthorityMode::Read)?];
        if let Some(cid) = &row.record.conversation_id {
            result.push(AuthorityScope::conversation(cid, AuthorityMode::Read)?);
        }
        return Ok(result);
    }
    scopes(&row.owner, row.record.conversation_id.as_deref())
}
pub(super) fn require_row(
    store: &Store,
    guard: &AuthorityGuardSet,
    row: &rows::Saved,
) -> Result<()> {
    if !store.authority_covers(guard, &row_scopes(row)?) {
        return Err(Error::new(
            "authority-unavailable",
            "Operation authority changed",
        ));
    }
    Ok(())
}
pub(crate) fn transition_admission_scopes(
    store: &Store,
    c: &Connection,
    actor: &Actor,
    request: &TransitionRequest,
) -> Result<Vec<AuthorityScope>> {
    actor.require(request.intent.required_scope())?;
    let tenant = authority::tenant(c, actor)?;
    if let Some(row) = rows::find(c, &tenant, &actor.identity_id, &request.operation_id)? {
        if row.ledger {
            authority::read(c, actor, &row)?;
            let mut result = row_scopes(&row)?;
            // Transition replies project their stored operation sources, not a
            // current Conversation snapshot. Do not widen a retained proof to
            // unrelated members after the first receipt is created.
            let mut members = c.prepare("SELECT identity_id FROM operation_members WHERE operation_record_id=?1 ORDER BY identity_id")
                .map_err(|_| Error::storage())?;
            for identity in members
                .query_map([&row.record.record_id], |r| r.get::<_, String>(0))
                .map_err(|_| Error::storage())?
            {
                result.push(AuthorityScope::identity(
                    &identity.map_err(|_| Error::storage())?,
                    AuthorityMode::Read,
                )?);
            }
            return Ok(result);
        }
        return peer::operation_source_scopes(
            c,
            actor,
            &crate::PeerOperationClaim {
                action: OperationAction::Get {
                    operation_id: request.operation_id.clone(),
                },
                conversation_id: None,
                session_id: None,
                connection_id: None,
            },
            false,
        );
    }
    let mut result = scopes(&actor.identity_id, Some(&request.conversation_id))?;
    result.extend(authority::admission_readers(c, actor, request, store)?);
    Ok(result)
}
impl Store {
    pub(super) fn transition_operation_authority(
        &self,
        credential: &str,
        operation: &str,
        required: &[&str],
        held: Option<&Arc<AuthorityGuardSet>>,
    ) -> Result<Arc<AuthorityGuardSet>> {
        self.stable_authority_with(
            held,
            &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                let actor = actor_from_connection(&c, credential)?;
                for scope in required {
                    actor.require(scope)?;
                }
                let tenant = authority::tenant(&c, &actor)?;
                let row = rows::find(&c, &tenant, &actor.identity_id, operation)?
                    .ok_or_else(Error::forbidden)?;
                authority::read(&c, &actor, &row)?;
                row_scopes(&row)
            },
        )
    }
    pub(super) fn transition_lease_authority(
        &self,
        credential: &str,
        lease: &TransitionLease,
    ) -> Result<Arc<AuthorityGuardSet>> {
        self.stable_authority(
            &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                let actor = actor_from_connection(&c, credential)?;
                row_scopes(&workers::lease_authority(&c, &actor, lease, self, true)?)
            },
        )
    }
    pub(super) fn transition_admission_authority(
        &self,
        credential: &str,
        request: &TransitionRequest,
        held: Option<&Arc<AuthorityGuardSet>>,
    ) -> Result<Arc<AuthorityGuardSet>> {
        self.stable_authority_with(
            held,
            &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                let actor = actor_from_connection(&c, credential)?;
                transition_admission_scopes(self, &c, &actor, request)
            },
        )
    }
    pub(super) fn transition_cleanup_authority(
        &self,
        credential: &str,
        record: &str,
    ) -> Result<Arc<AuthorityGuardSet>> {
        self.stable_authority(
            &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                let actor = actor_from_connection(&c, credential)?;
                let row = rows::by_record(&c, record)?;
                cleanup::authorize(&c, &actor, &row)?;
                let mut result = row_scopes(&row)?;
                // Repair can belong to another owner and creates the actor's receipt.
                result.push(AuthorityScope::identity(
                    &actor.identity_id,
                    AuthorityMode::Write,
                )?);
                Ok(result)
            },
        )
    }
}

#[derive(Clone, Copy)]
pub(super) enum BatchKind {
    Recover,
    Fence,
    Purge,
}
impl BatchKind {
    fn predicate(self) -> &'static str {
        match self {
            Self::Recover => "o.ledger_version=1 AND o.state NOT IN ('completed','cancelled','failed','unknown') AND ((o.lease_owner IS NOT NULL AND ((?3 AND o.worker_runtime IS NOT ?1) OR o.lease_expires_at IS NULL OR o.lease_expires_at<=?2)) OR o.expires_at<=?2)",
            Self::Fence => "o.ledger_version=1 AND o.state NOT IN ('completed','cancelled','failed') AND o.lease_owner IS NOT NULL AND o.worker_runtime IS NOT 'fenced'",
            // Keep this unsettled predicate equivalent to rows::unsettled: a
            // retained unresolved row must not starve later purgeable results.
            Self::Purge => "o.ledger_version=1 AND o.tombstoned=0 AND o.state IN ('completed','cancelled','failed') AND o.updated_at<=?2-30*86400 AND EXISTS(SELECT 1 FROM conversations v WHERE v.id=o.conversation_id AND v.closed=1 AND v.closed_at<=?2-30*86400) AND NOT EXISTS(SELECT 1 FROM operation_effects e WHERE e.operation_record_id=o.record_id AND (e.state IN ('submitting','unknown') OR (e.state='confirmed' AND e.compensates_effect_id IS NULL AND NOT EXISTS(SELECT 1 FROM operation_effects cleanup WHERE cleanup.compensates_effect_id=e.id AND cleanup.state='confirmed'))))",
        }
    }
}
pub(super) struct ScopeRow {
    pub id: String,
    owner: String,
    cid: Option<String>,
    pub generation: u64,
}
impl ScopeRow {
    fn scopes(&self) -> Result<Vec<AuthorityScope>> {
        scopes(&self.owner, self.cid.as_deref())
    }
}
fn select(
    c: &Connection,
    store: &Store,
    kind: BatchKind,
    limit: u32,
    record: Option<&str>,
) -> Result<Vec<ScopeRow>> {
    let suffix = if record.is_some() {
        " AND o.record_id=?4"
    } else {
        match kind {
            BatchKind::Purge => " ORDER BY o.updated_at,o.record_id LIMIT ?4",
            _ => " ORDER BY o.created_at,o.record_id LIMIT ?4",
        }
    };
    let sql = format!("SELECT o.record_id,o.owner_id,o.conversation_id,o.worker_generation FROM operations o WHERE {}{}", kind.predicate(), suffix);
    let fourth = record
        .map(|value| rusqlite::types::Value::Text(value.to_owned()))
        .unwrap_or_else(|| i64::from(limit).into());
    let mut st = c.prepare(&sql).map_err(|_| Error::storage())?;
    let result = st
        .query_map(
            params![
                store.transition_runtime,
                crate::now(),
                store.recover_foreign_runtime,
                fourth
            ],
            |r| {
                Ok(ScopeRow {
                    id: r.get(0)?,
                    owner: r.get(1)?,
                    cid: r.get(2)?,
                    generation: r.get(3)?,
                })
            },
        )
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    Ok(result)
}
fn prefix(
    c: &Connection,
    store: &Store,
    kind: BatchKind,
    limit: u32,
) -> Result<(Vec<String>, Vec<AuthorityScope>)> {
    let mut ids = vec![];
    let mut merged = BTreeSet::new();
    for row in select(c, store, kind, limit, None)? {
        let scopes = row.scopes()?;
        let extra = scopes.iter().filter(|s| !merged.contains(*s)).count();
        // At most 128 held files (admission + resource), independent of state size.
        if merged.len() + extra > 64 {
            break;
        }
        merged.extend(scopes);
        ids.push(row.id);
    }
    Ok((ids, merged.into_iter().collect()))
}
impl Store {
    pub(super) fn transition_batch_authority(
        &self,
        kind: BatchKind,
        limit: u32,
    ) -> Result<(Vec<String>, Arc<AuthorityGuardSet>)> {
        if limit == 0 || limit > 1000 {
            return Err(invalid());
        }
        let wait = AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT);
        loop {
            let (_, scopes) = prefix(&*self.connection()?, self, kind, limit)?;
            let guard = self.acquire_authority(&scopes, &wait)?;
            let (ids, current) = prefix(&*self.connection()?, self, kind, limit)?;
            if self.authority_covers(&guard, &current) {
                return Ok((ids, guard));
            }
        }
    }
    /// Validate every selected current row before mutating any row. New eligible
    /// rows cannot slip into a transaction behind a previously admitted prefix.
    pub(super) fn transition_batch_rows(
        &self,
        c: &Connection,
        kind: BatchKind,
        ids: &[String],
        guard: &AuthorityGuardSet,
    ) -> Result<Vec<ScopeRow>> {
        let mut result = vec![];
        for id in ids {
            for row in select(c, self, kind, 1, Some(id))? {
                if !self.authority_covers(guard, &row.scopes()?) {
                    return Err(Error::new(
                        "authority-unavailable",
                        "Operation batch authority changed",
                    ));
                }
                result.push(row);
            }
        }
        Ok(result)
    }
}
