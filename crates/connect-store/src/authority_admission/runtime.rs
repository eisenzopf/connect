//! Persisted runtime targets, never a caller-supplied copy of a claimed job.
//! Scope queries are static internal SQL and run under short reads or inside the
//! already-admitted transaction. No lock wait occurs while SQLite is held.
use crate::{AuthorityGuardSet, AuthorityMode, AuthorityScope, AuthorityWait, Store};
use connect_protocol::{Error, Result};
use rusqlite::{Connection, ToSql};
use std::{collections::BTreeSet, sync::Arc};

pub(crate) const PROVIDER_JOB: &str = "SELECT r.conversation_id FROM provider_jobs j JOIN connector_routes r ON r.id=j.route_id WHERE j.id=?1";
pub(crate) const ROUTE: &str = "SELECT conversation_id FROM connector_routes WHERE id=?1";
pub(crate) const VOICE_CALL: &str = "SELECT conversation_id FROM voice_calls WHERE id=?1";
pub(crate) const VOICE_ROUTE: &str = "SELECT conversation_id FROM voice_routes WHERE id=?1";
pub(crate) const VOICE_PENDING: &str = "SELECT conversation_id FROM voice_calls WHERE state='pending' OR (provider_id IS NOT NULL AND state NOT IN ('ended','failed','cancelled','forbidden','starting','ending') AND next_poll<=?1) ORDER BY end_requested DESC,sequence LIMIT 1";
pub(crate) const VOICE_RECOVERY: &str =
    "SELECT DISTINCT conversation_id FROM voice_calls WHERE state IN ('starting','ending')";
pub(crate) const SIP_LEG: &str = "SELECT conversation_id FROM sip_legs WHERE id=?1";
pub(crate) const SIP_PENDING: &str = "SELECT conversation_id FROM sip_legs WHERE state='pending' OR (state='active' AND end_requested=1) ORDER BY sequence LIMIT 1";
pub(crate) const SIP_RECOVERY: &str =
    "SELECT DISTINCT conversation_id FROM sip_legs WHERE state IN ('starting','active','ending')";
pub(crate) const SIP_CONNECTION: &str = "SELECT DISTINCT conversation_id FROM sip_legs WHERE connection_id=?1 AND state NOT IN ('ended','failed','cancelled')";
pub(crate) const SMS_GRANTS: &str = "SELECT DISTINCT conversation_id FROM sms_grants";
pub(crate) const SMS_PHONE: &str = "SELECT conversation_id FROM sms_grants WHERE phone=?1 UNION SELECT conversation_id FROM connector_routes WHERE target=?1";

pub(crate) fn conversations(
    c: &Connection,
    sql: &'static str,
    values: &[&dyn ToSql],
) -> Result<Vec<String>> {
    let mut statement = c.prepare(sql).map_err(|_| Error::storage())?;
    let mut rows = statement.query(values).map_err(|_| Error::storage())?;
    let mut ids = BTreeSet::new();
    while let Some(row) = rows.next().map_err(|_| Error::storage())? {
        ids.insert(row.get::<_, String>(0).map_err(|_| Error::storage())?);
        if ids.len() > 1024 {
            return Err(Error::new(
                "capacity-exceeded",
                "Authority Conversation capacity exceeded",
            ));
        }
    }
    Ok(ids.into_iter().collect())
}
fn scopes(c: &Connection, ids: &[String]) -> Result<Vec<AuthorityScope>> {
    let mut scopes = vec![];
    for cid in ids {
        for scope in super::conversation_scopes(c, cid, AuthorityMode::Write)? {
            if !scopes.contains(&scope) {
                if scopes.len() >= 1024 {
                    return Err(Error::new(
                        "capacity-exceeded",
                        "Authority scope capacity exceeded",
                    ));
                }
                scopes.push(scope);
            }
        }
    }
    Ok(scopes)
}
impl Store {
    pub(crate) fn runtime_query_authority(
        &self,
        sql: &'static str,
        values: &[&dyn ToSql],
    ) -> Result<Arc<AuthorityGuardSet>> {
        let guard = self.runtime_conversation_authority(|c| conversations(c, sql, values))?;
        #[cfg(test)]
        test_boundary()?;
        Ok(guard)
    }
    pub(crate) fn require_runtime_conversations(
        &self,
        c: &Connection,
        held: &AuthorityGuardSet,
        ids: &[String],
    ) -> Result<()> {
        if !self.authority_covers(held, &scopes(c, ids)?) {
            return Err(Error::new(
                "authority-unavailable",
                "Runtime authority changed before transaction; retry without creating new intent",
            ));
        }
        Ok(())
    }
    pub(crate) fn require_runtime_query(
        &self,
        c: &Connection,
        held: &AuthorityGuardSet,
        sql: &'static str,
        values: &[&dyn ToSql],
    ) -> Result<()> {
        self.require_runtime_conversations(c, held, &conversations(c, sql, values)?)
    }
    pub(crate) fn admin_runtime_authority(
        &self,
        credential: &str,
        sql: &'static str,
        values: &[&dyn ToSql],
    ) -> Result<Arc<AuthorityGuardSet>> {
        self.stable_authority(
            &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                let ids = self.admin_runtime_targets(&c, credential, sql, values)?;
                scopes(&c, &ids)
            },
        )
    }
    pub(crate) fn admin_runtime_targets(
        &self,
        c: &Connection,
        credential: &str,
        sql: &'static str,
        values: &[&dyn ToSql],
    ) -> Result<Vec<String>> {
        let actor = crate::identity::actor_from_connection(c, credential)?;
        actor.require("admin")?;
        let ids = conversations(c, sql, values)?;
        for cid in &ids {
            let allowed:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM conversations v JOIN identities i ON i.tenant_id=v.tenant_id WHERE v.id=?1 AND i.id=?2 AND i.revoked=0)",rusqlite::params![cid,actor.identity_id],|r|r.get(0)).map_err(|_| Error::storage())?;
            if !allowed {
                return Err(Error::forbidden());
            }
        }
        Ok(ids)
    }
}

#[cfg(test)]
thread_local! {
    static ADMISSION_BOUNDARY: std::cell::RefCell<Option<(std::sync::mpsc::Sender<()>,std::sync::mpsc::Receiver<()>)>>=const {std::cell::RefCell::new(None)};
}
#[cfg(test)]
pub(crate) fn pause_next_admission(
    entered: std::sync::mpsc::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
) {
    ADMISSION_BOUNDARY.with(|slot| *slot.borrow_mut() = Some((entered, release)));
}
#[cfg(test)]
fn test_boundary() -> Result<()> {
    if let Some((entered, release)) = ADMISSION_BOUNDARY.with(|slot| slot.borrow_mut().take()) {
        entered.send(()).map_err(|_| Error::storage())?;
        release
            .recv_timeout(std::time::Duration::from_secs(8))
            .map_err(|_| Error::storage())?;
    }
    Ok(())
}
