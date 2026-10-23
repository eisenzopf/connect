//! Current, single-use provider dispatch and owned transport custody.
use crate::{
    identity::actor_from_connection, AuthorityGuardSet, AuthorityMode, AuthorityScope,
    AuthorityWait, ProviderJob, ProviderWorkerOwnership, Store,
};
use connect_connector_api::{dispatch, DispatchPermit};
use connect_protocol::{Error, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
const MAX_ATTEMPT: Duration = Duration::from_secs(35);
struct Custody {
    _scopes: Arc<AuthorityGuardSet>,
    role: Arc<ProviderWorkerOwnership>,
    expires_at: i64,
    deadline: Instant,
}
impl dispatch::Authority for Custody {
    fn current(&self) -> bool {
        self.role.current() && crate::now() < self.expires_at
    }
    fn deadline(&self) -> Instant {
        self.deadline
    }
}
struct Admission {
    scopes: Vec<AuthorityScope>,
    expires_at: i64,
}
fn authorization(
    c: &Connection,
    store: &Store,
    job: &ProviderJob,
    configuration: Option<&std::collections::BTreeMap<String, String>>,
) -> Result<Admission> {
    let (actual, state) =
        crate::connector_observations::authorized_job(c, &job.id, &job.route.connector)?;
    if state != "sending"
        || actual.route != job.route
        || actual.body != job.body
        || actual.context != job.context
    {
        return Err(Error::forbidden());
    }
    let (runtime,token,previous,admitted):(String,String,Option<String>,bool)=c.query_row("SELECT runtime_id,claim_token,previous_chat_id,admitted FROM provider_dispatch WHERE job_id=?1", [&job.id], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)?;
    if runtime != store.runtime_epoch()
        || job.claim_token.as_deref() != Some(&token)
        || previous != job.previous_chat_id
        || admitted
    {
        return Err(Error::new(
            "conflict",
            "Dispatch was already admitted or belongs to another claim",
        ));
    }
    crate::tasks::authorize_effect_link(c, "provider_job", &job.id, crate::now())?;
    crate::connector_registry::require_operation(
        c,
        &actual.route.connector,
        connect_connector_api::manifest::ConnectorOperation::MessageSend,
    )?;
    let credential: String = c
        .query_row(
            "SELECT credential_id FROM provider_jobs WHERE id=?1",
            [&job.id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let actor = actor_from_connection(c, &credential)?;
    let delegate = actor_from_connection(c, &actual.route.credential_id)?;
    let mut expires_at = actor.expires_at.min(delegate.expires_at);
    let policy =
        crate::provider_chat_context::authority_policy(store, c, &credential, &actual.route)?;
    let saved: String = c
        .query_row(
            "SELECT authority_policy FROM provider_jobs WHERE id=?1",
            [&job.id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if saved != policy {
        return Err(Error::forbidden());
    }
    if let Some(raw) = &job.context {
        if let Some(until) = crate::records::context::encoded_deadline(raw)? {
            expires_at = expires_at.min(until);
        }
        use connect_connector_api::host_context::HostContext;
        let selected = HostContext::decode(raw, 512 * 1024).map_err(|_| Error::forbidden())?;
        let current = match selected {
            HostContext::Conversation(ref view) => {
                let view = crate::context::view_for_scope_at(
                    store,
                    c,
                    &actor,
                    &job.route.conversation_id,
                    1000,
                    Some(&delegate.identity_id),
                    Some(&delegate),
                    Some(view.watermark),
                )?;
                serde_json::to_value(view).map_err(|_| Error::storage())?
            }
            HostContext::Task(_) => match crate::tasks::provider_task_context(c, &job.id, store)? {
                crate::tasks::ProviderTaskContext::Selected {
                    json, actor: owner, ..
                } => serde_json::from_str(&crate::provider_chat_context::export_task_selection(
                    store,
                    &owner,
                    c,
                    &job.route.conversation_id,
                    &json,
                    &delegate.identity_id,
                    Some(&delegate),
                )?)
                .map_err(|_| Error::storage())?,
                _ => return Err(Error::forbidden()),
            },
        };
        let selected: serde_json::Value =
            serde_json::from_str(raw).map_err(|_| Error::storage())?;
        if current != selected {
            return Err(Error::forbidden());
        }
    }
    let task_deadline:Option<i64>=c.query_row("SELECT MIN(bound) FROM (SELECT t.expires_at bound FROM task_effect_links l JOIN task_effects e ON e.id=l.effect_id JOIN tasks t ON t.id=e.task_id WHERE l.kind='provider_job' AND l.resource_id=?1 UNION ALL SELECT g.expires_at FROM task_effect_links l JOIN task_effect_authority a ON a.effect_id=l.effect_id JOIN task_grants g ON g.id=a.grant_id WHERE l.kind='provider_job' AND l.resource_id=?1 UNION ALL SELECT k.expires_at FROM task_effect_links l JOIN task_effect_authority a ON a.effect_id=l.effect_id JOIN credentials k ON k.id=a.credential_id WHERE l.kind='provider_job' AND l.resource_id=?1 UNION ALL SELECT k.expires_at FROM task_effect_links l JOIN task_effects e ON e.id=l.effect_id JOIN tasks t ON t.id=e.task_id JOIN credentials k ON k.id=t.credential_id WHERE l.kind='provider_job' AND l.resource_id=?1)",[&job.id],|r|r.get(0)).map_err(|_|Error::storage())?;
    if let Some(deadline) = task_deadline {
        expires_at = expires_at.min(deadline);
    }
    if store.connector(&job.route.connector)?.sms {
        let sender = job.route.source.as_deref().unwrap_or_default();
        let selected = crate::tasks::task_sms_decision(
            c,
            &job.id,
            &job.route.target,
            sender,
            &job.route.conversation_id,
        )?
        .unwrap_or(crate::sms_eligibility::eligible(
            c,
            &job.route.target,
            sender,
            &job.route.conversation_id,
        )?)
        .map_err(|_| Error::forbidden())?;
        let boundary:i64=c.query_row("SELECT MIN(MIN(g.expires_at,a.valid_until)) FROM sms_grants g JOIN sms_authorities a ON a.source=g.source WHERE g.source || ':' || g.id=?1 AND g.phone=?2 AND g.sender=?3 AND g.conversation_id=?4 AND g.active=1 AND g.expires_at>?5 AND a.valid_until>?5", params![selected,job.route.target,sender,job.route.conversation_id,crate::now()], |r|r.get(0)).map_err(|_|Error::forbidden())?;
        expires_at = expires_at.min(boundary);
    }
    if expires_at <= crate::now() {
        return Err(Error::forbidden());
    }
    let mut scopes = crate::authority_admission::conversation_scopes(
        c,
        &job.route.conversation_id,
        AuthorityMode::Read,
    )?;
    if let Some(binding) = crate::provider_message_endpoints::current(c, &actual.route)? {
        crate::provider_message_endpoints::configuration(
            &binding,
            configuration.ok_or_else(Error::forbidden)?,
        )?;
        let (expiry, owned) = crate::provider_message_endpoints::account(c, &binding)?;
        expires_at = expires_at.min(expiry);
        scopes.extend(owned);
    }
    Ok(Admission { scopes, expires_at })
}
impl Store {
    /// Protected, single-use last-mile admission. The adapter must transfer this
    /// non-wire custody into the transport, not discard it after checking a bool.
    pub fn admit_provider_dispatch(
        &self,
        role: &Arc<ProviderWorkerOwnership>,
        job: &ProviderJob,
        wait: &AuthorityWait,
    ) -> Result<DispatchPermit> {
        self.admit_provider_dispatch_inner(role, job, wait, None)
    }
    pub fn admit_configured_provider_dispatch(
        &self,
        role: &Arc<ProviderWorkerOwnership>,
        job: &ProviderJob,
        wait: &AuthorityWait,
        configuration: &std::collections::BTreeMap<String, String>,
    ) -> Result<DispatchPermit> {
        self.admit_provider_dispatch_inner(role, job, wait, Some(configuration))
    }
    fn admit_provider_dispatch_inner(
        &self,
        role: &Arc<ProviderWorkerOwnership>,
        job: &ProviderJob,
        wait: &AuthorityWait,
        configuration: Option<&std::collections::BTreeMap<String, String>>,
    ) -> Result<DispatchPermit> {
        self.require_provider_worker(role)?;
        let scopes = self.stable_authority(wait, || {
            self.require_provider_worker(role)?;
            authorization(&*self.connection()?, self, job, configuration).map(|a| a.scopes)
        })?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        self.require_provider_worker(role)?;
        let current = authorization(&tx, self, job, configuration)?;
        if !self.authority_covers(&scopes, &current.scopes) {
            return Err(Error::forbidden());
        }
        let duration = std::time::UNIX_EPOCH
            .checked_add(Duration::from_secs(current.expires_at as u64))
            .and_then(|at| at.duration_since(std::time::SystemTime::now()).ok())
            .ok_or_else(Error::forbidden)?
            .min(MAX_ATTEMPT);
        let deadline = Instant::now() + duration;
        if tx
            .execute(
                "UPDATE provider_dispatch SET admitted=1 WHERE job_id=?1 AND admitted=0",
                [&job.id],
            )
            .map_err(|_| Error::storage())?
            != 1
        {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok(DispatchPermit::from_host(Arc::new(Custody {
            _scopes: scopes,
            role: role.clone(),
            expires_at: current.expires_at,
            deadline,
        })))
    }
    /// A denied claim with no dispatch admission is a known local non-effect.
    /// Never use this path to turn an admitted/uncertain send back into pending.
    pub fn reject_provider_claim(&self, job: &ProviderJob) -> Result<()> {
        let authority = self.runtime_query_authority(
            crate::authority_admission::runtime::PROVIDER_JOB,
            &[&job.id],
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        self.require_runtime_query(
            &tx,
            &authority,
            crate::authority_admission::runtime::PROVIDER_JOB,
            &[&job.id],
        )?;
        let count=tx.execute("UPDATE provider_jobs SET state='cancelled' WHERE id=?1 AND state='sending' AND EXISTS(SELECT 1 FROM provider_dispatch d WHERE d.job_id=provider_jobs.id AND d.runtime_id=?2 AND d.claim_token=?3 AND d.admitted=0)",params![job.id,self.runtime_epoch(),job.claim_token]).map_err(|_|Error::storage())?;
        if count != 1 {
            return Err(Error::forbidden());
        }
        crate::provider_chat_context::finish(&tx, &job.id, "failed", None)?;
        tx.commit().map_err(|_| Error::storage())
    }
}

#[cfg(test)]
pub(crate) mod tests;

const RECOVERABLE:&str="j.state='sending' OR (j.state='pending' AND EXISTS(SELECT 1 FROM provider_dispatch d WHERE d.job_id=j.id AND d.state_identity!=?1))";
fn recovery_rows(c: &Connection, state: &str, only: Option<&str>) -> Result<Vec<(String, String)>> {
    let suffix = if only.is_some() {
        " AND j.id=?2"
    } else {
        " ORDER BY j.sequence LIMIT ?2"
    };
    let sql=format!("SELECT j.id,r.conversation_id FROM provider_jobs j JOIN connector_routes r ON r.id=j.route_id WHERE ({RECOVERABLE}){suffix}");
    let value = only
        .map(|id| rusqlite::types::Value::Text(id.into()))
        .unwrap_or(1000_i64.into());
    let mut statement = c.prepare(&sql).map_err(|_| Error::storage())?;
    let rows = statement
        .query_map(params![state, value], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    Ok(rows)
}
fn recovery_prefix(c: &Connection, state: &str) -> Result<(Vec<String>, Vec<AuthorityScope>)> {
    let mut ids = vec![];
    let mut scopes = std::collections::BTreeSet::new();
    for (id, cid) in recovery_rows(c, state, None)? {
        let scope = AuthorityScope::conversation(&cid, AuthorityMode::Write)?;
        if scopes.len() == 64 && !scopes.contains(&scope) {
            break;
        }
        scopes.insert(scope);
        ids.push(id);
    }
    Ok((ids, scopes.into_iter().collect()))
}
impl Store {
    pub(crate) fn recover_provider_batch(&self) -> Result<usize> {
        let state = self.authority.state_identity();
        let wait = AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT);
        let (ids, guard) = loop {
            let (_, scopes) = recovery_prefix(&*self.connection()?, state)?;
            let guard = self.acquire_authority(&scopes, &wait)?;
            let (ids, current) = recovery_prefix(&*self.connection()?, state)?;
            if self.authority_covers(&guard, &current) {
                break (ids, guard);
            }
        };
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let mut selected = vec![];
        for id in ids {
            for (id, cid) in recovery_rows(&tx, state, Some(&id))? {
                if !self.authority_covers(
                    &guard,
                    &[AuthorityScope::conversation(&cid, AuthorityMode::Write)?],
                ) {
                    return Err(Error::new(
                        "authority-unavailable",
                        "Provider recovery targets changed",
                    ));
                }
                selected.push(id);
            }
        }
        for id in &selected {
            let known_absent:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM provider_dispatch d WHERE d.job_id=?1 AND d.state_identity=?2 AND d.admitted=0)",params![id,state],|r|r.get(0)).map_err(|_|Error::storage())?;
            let outcome = if known_absent {
                "cancelled"
            } else {
                "uncertain"
            };
            tx.execute("UPDATE context_deliveries SET state=?2,updated_at=?3 WHERE id='provider-job:' || ?1 AND state='pending'",params![id,if known_absent{"failed"}else{"uncertain"},crate::now()]).map_err(|_|Error::storage())?;
            tx.execute(
                "UPDATE provider_jobs SET state=?2 WHERE id=?1",
                params![id, outcome],
            )
            .map_err(|_| Error::storage())?;
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok(selected.len())
    }
}
