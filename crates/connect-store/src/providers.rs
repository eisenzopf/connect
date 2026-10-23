//! Transactional connector boundary. Network effects never run inside SQLite.
use crate::{conversations::authorized_conversation, identity::actor_from_connection, Store};
use connect_protocol::{Error, ProviderDelivery, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Route {
    pub id: String,
    pub identity_id: String,
    pub credential_id: String,
    pub conversation_id: String,
    pub connector: String,
    pub target: String,
    pub source: Option<String>,
    pub enabled: bool,
    pub incoming_to: connect_protocol::messages::MessageAudience,
}
#[derive(Clone)]
pub struct ProviderJob {
    pub id: String,
    pub route: Route,
    pub body: String,
    pub previous_chat_id: Option<String>,
    /// Exact serialized context selection journaled before provider dispatch.
    pub context: Option<String>,
    pub(crate) claim_token: Option<String>,
}
impl std::fmt::Debug for Route {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Route")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for ProviderJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderJob")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}
pub(crate) fn route(c: &Connection, id: &str) -> Result<Route> {
    c.query_row("SELECT id,identity_id,credential_id,conversation_id,connector,target,source,enabled,incoming_to_json FROM connector_routes WHERE id=?1", [id], |r| Ok(Route { id:r.get(0)?,identity_id:r.get(1)?,credential_id:r.get(2)?,conversation_id:r.get(3)?,connector:r.get(4)?,target:r.get(5)?,source:r.get(6)?,enabled:r.get(7)?,incoming_to:serde_json::from_str(&r.get::<_,String>(8)?).map_err(|_|rusqlite::Error::InvalidQuery)? })).map_err(|_|Error::forbidden())
}
pub(crate) fn enqueue(
    c: &Connection,
    message: &str,
    conversation: &str,
    sender: &str,
    credential: &str,
    store: &Store,
) -> Result<()> {
    let max = store.limits.operation_receipts;
    let count: u64 = c
        .query_row("SELECT COUNT(*) FROM provider_jobs", [], |r| r.get(0))
        .map_err(|_| Error::storage())?;
    let selected="SELECT r.id FROM connector_routes r JOIN participants p ON p.conversation_id=r.conversation_id AND p.identity_id=r.identity_id AND p.active=1 JOIN message_audiences a ON a.message_id=?3 AND a.participant_id=p.participant_id AND a.membership_epoch=p.membership_epoch WHERE r.enabled=1 AND r.conversation_id=?1 AND r.identity_id!=?2";
    let routes: u64 = c
        .query_row(
            &format!("SELECT COUNT(*) FROM ({selected})"),
            params![conversation, sender, message],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if count.saturating_add(routes) > max {
        return Err(Error::new(
            "capacity-exceeded",
            "Provider job capacity exceeded",
        ));
    }
    let mut st = c.prepare(selected).map_err(|_| Error::storage())?;
    let ids = st
        .query_map(params![conversation, sender, message], |r| {
            r.get::<_, String>(0)
        })
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    for id in ids {
        let job_id = uuid::Uuid::new_v4().to_string();
        let supported:bool=c.query_row("SELECT content_type='text/plain' AND body_encoding='utf8' AND NOT EXISTS(SELECT 1 FROM message_attachments WHERE message_id=messages.id) FROM messages WHERE id=?1",[message],|r|r.get(0)).map_err(|_|Error::storage())?;
        c.execute(
            "INSERT INTO provider_jobs(id,route_id,message_id,credential_id,state,refusal_code) VALUES(?1,?2,?3,?4,?5,?6)",
            params![job_id, id, message, credential,if supported {"pending"}else{"unsupported"},if supported {None}else{Some("unsupported-message-content")}],
        )
        .map_err(|_| Error::storage())?;
        c.execute(
            "INSERT INTO provider_dispatch(job_id,state_identity) VALUES(?1,?2)",
            params![job_id, store.authority.state_identity()],
        )
        .map_err(|_| Error::storage())?;
    }
    Ok(())
}
pub(crate) fn status(
    c: &Connection,
    actor: &crate::Actor,
    conversation: &str,
    limit: u32,
) -> Result<Vec<ProviderDelivery>> {
    let mut st=c.prepare("SELECT j.id,j.message_id,r.connector,j.state,j.provider_id,j.sms_decision,j.sms_grant_id,j.refusal_code FROM provider_jobs j JOIN connector_routes r ON j.route_id=r.id JOIN messages m ON m.id=j.message_id WHERE r.conversation_id=?1 AND (m.sender=?3 OR EXISTS(SELECT 1 FROM json_each(?4) WHERE value IN(m.sender,'*'))) AND (m.sender=?3 OR r.identity_id=?3) AND (r.identity_id=?3 OR EXISTS(SELECT 1 FROM json_each(?4) WHERE value IN(r.identity_id,'*'))) AND EXISTS(SELECT 1 FROM content_audiences a JOIN participants p ON p.participant_id=a.participant_id AND p.membership_epoch=a.membership_epoch AND p.active=1 WHERE a.kind='message' AND a.id=m.id AND p.identity_id=?3) ORDER BY j.sequence DESC LIMIT ?2").map_err(|_|Error::storage())?;
    let rows = st
        .query_map(
            params![
                conversation,
                limit,
                actor.identity_id,
                serde_json::to_string(&actor.scope.recipients).map_err(|_| Error::storage())?
            ],
            |r| {
                Ok(ProviderDelivery {
                    job_id: r.get(0)?,
                    message_id: r.get(1)?,
                    connector: r.get(2)?,
                    state: r.get(3)?,
                    provider_id: r.get(4)?,
                    sms_decision: r.get(5)?,
                    sms_grant_id: r.get(6)?,
                    refusal_code: r.get(7)?,
                })
            },
        )
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    Ok(rows)
}
impl Store {
    /// Local operator command, while the installation is stopped. Bindings are
    /// immutable; only enable/disable is mutable to avoid rerouting queued work.
    pub fn configure_route(&self, operator: &str, value: &Route) -> Result<()> {
        let mut normalized = value.clone();
        normalized.incoming_to.normalize()?;
        let value = &normalized;
        let authority = self.admin_runtime_authority(
            operator,
            "SELECT ?2 UNION SELECT conversation_id FROM connector_routes WHERE id=?1",
            &[&value.id, &value.conversation_id],
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let targets = self.admin_runtime_targets(
            &tx,
            operator,
            "SELECT ?2 UNION SELECT conversation_id FROM connector_routes WHERE id=?1",
            &[&value.id, &value.conversation_id],
        )?;
        self.require_runtime_conversations(&tx, &authority, &targets)?;
        let administrator = actor_from_connection(&tx, operator)?;
        administrator.require("admin")?;
        let tenant:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM identities i JOIN conversations c ON c.tenant_id=i.tenant_id WHERE i.id=?1 AND c.id=?2)",params![administrator.identity_id,value.conversation_id],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !tenant || !administrator.permits_recipient(&value.identity_id) {
            return Err(Error::forbidden());
        }
        if value.id.is_empty()
            || value.id.len() > 128
            || !value
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            || value.target.len() > 256
            || value.source.as_ref().is_some_and(|s| s.len() > 256)
        {
            return Err(Error::new("invalid-request", "Invalid route binding"));
        }
        let sender = actor_from_connection(&tx, &value.credential_id)?;
        sender.require("send")?;
        if sender.identity_id != value.identity_id {
            return Err(Error::forbidden());
        }
        if authorized_conversation(&tx, &sender, &value.conversation_id)?.closed {
            return Err(Error::new(
                "conversation-closed",
                "Cannot bind closed conversation",
            ));
        }
        if let connect_protocol::messages::MessageAudience::Participants(ids) = &value.incoming_to {
            for id in ids {
                let identity:Option<String>=tx.query_row("SELECT identity_id FROM participants WHERE participant_id=?1 AND conversation_id=?2 AND active=1",params![id,value.conversation_id],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
                let identity = identity.ok_or_else(Error::forbidden)?;
                if !administrator.permits_recipient(&identity)
                    || !sender.permits_recipient(&identity)
                {
                    return Err(Error::forbidden());
                }
            }
        }
        let descriptor = self.connector(&value.connector)?;
        crate::connector_registry::validate(descriptor, &value.target, value.source.as_deref())?;
        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM connector_routes WHERE id=?1)",
                [&value.id],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        if exists {
            let mut old = route(&tx, &value.id)?;
            old.enabled = value.enabled;
            if old != *value {
                return Err(Error::new(
                    "conflict",
                    "Route binding cannot be changed; use a new route ID",
                ));
            }
            tx.execute(
                "UPDATE connector_routes SET enabled=?2 WHERE id=?1",
                params![value.id, value.enabled],
            )
            .map_err(|_| Error::storage())?;
        } else {
            tx.execute(
                "INSERT INTO connector_routes VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![
                    value.id,
                    value.identity_id,
                    value.credential_id,
                    value.conversation_id,
                    value.connector,
                    value.target,
                    value.source,
                    value.enabled,
                    serde_json::to_string(&value.incoming_to).map_err(|_| Error::storage())?
                ],
            )
            .map_err(|_| Error::storage())?;
        }
        tx.commit().map_err(|_| Error::storage())
    }
    /// An interrupted remote mutation has unknown outcome. Never retry it by
    /// default: Telnyx SMS does not provide idempotent message submission.
    pub fn recover_provider_jobs(&self) -> Result<usize> {
        let mut total = 0;
        loop {
            let changed = self.recover_provider_batch()?;
            total += changed;
            if changed == 0 {
                return Ok(total);
            }
        }
    }
    pub fn claim_provider_job(&self) -> Result<Option<ProviderJob>> {
        self.claim_provider_job_scope(None)
    }
    pub fn claim_provider_job_for_routes(&self, routes: &[String]) -> Result<Option<ProviderJob>> {
        if routes.is_empty() {
            return Ok(None);
        }
        self.claim_provider_job_scope(Some(routes))
    }
    fn claim_provider_job_scope(&self, routes: Option<&[String]>) -> Result<Option<ProviderJob>> {
        let selected = routes
            .map(serde_json::to_string)
            .transpose()
            .map_err(|_| Error::storage())?;
        const SCOPED:&str="SELECT r.conversation_id FROM provider_jobs j JOIN connector_routes r ON r.id=j.route_id WHERE j.state='pending' AND (?1 IS NULL OR j.route_id IN(SELECT value FROM json_each(?1))) AND (EXISTS(SELECT 1 FROM task_effect_links t WHERE t.kind='provider_job' AND t.resource_id=j.id) OR NOT EXISTS(SELECT 1 FROM provider_jobs running WHERE running.route_id=j.route_id AND running.state='sending' AND NOT EXISTS(SELECT 1 FROM task_effect_links t WHERE t.kind='provider_job' AND t.resource_id=running.id))) ORDER BY j.sequence LIMIT 1";
        let authority = self.runtime_query_authority(SCOPED, &[&selected])?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row:Option<(String,String,String,String)>=tx.query_row("SELECT j.id,j.route_id,m.body,j.credential_id FROM provider_jobs j JOIN messages m ON j.message_id=m.id WHERE j.state='pending' AND (?1 IS NULL OR j.route_id IN(SELECT value FROM json_each(?1))) AND (EXISTS(SELECT 1 FROM task_effect_links t WHERE t.kind='provider_job' AND t.resource_id=j.id) OR NOT EXISTS(SELECT 1 FROM provider_jobs running WHERE running.route_id=j.route_id AND running.state='sending' AND NOT EXISTS(SELECT 1 FROM task_effect_links t WHERE t.kind='provider_job' AND t.resource_id=running.id))) ORDER BY j.sequence LIMIT 1",[selected],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|_|Error::storage())?;
        let Some((id, rid, body, credential)) = row else {
            return Ok(None);
        };
        let r = route(&tx, &rid)?;
        self.require_runtime_conversations(&tx, &authority, &[r.conversation_id.clone()])?;
        let original: Option<String> = tx
            .query_row(
                "SELECT state_identity FROM provider_dispatch WHERE job_id=?1",
                [&id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| Error::storage())?;
        if original.as_deref() != Some(self.authority.state_identity()) {
            tx.execute(
                "UPDATE provider_jobs SET state='uncertain' WHERE id=?1",
                [&id],
            )
            .map_err(|_| Error::storage())?;
            tx.commit().map_err(|_| Error::storage())?;
            return Ok(None);
        }
        match crate::tasks::authorize_effect_link(&tx, "provider_job", &id, crate::now()) {
            Ok(()) => (),
            Err(e) if matches!(e.code.as_str(), "forbidden" | "unauthorized") => {
                tx.execute(
                    "UPDATE provider_jobs SET state='cancelled' WHERE id=?1",
                    [&id],
                )
                .map_err(|_| Error::storage())?;
                tx.commit().map_err(|_| Error::storage())?;
                return Ok(None);
            }
            Err(e) => return Err(e),
        }
        let allowed = (|| {
            crate::provider_message_endpoints::current(&tx, &r)?;
            let actor = actor_from_connection(&tx, &credential)?;
            actor.require("send")?;
            let conv = authorized_conversation(&tx, &actor, &r.conversation_id)?;
            let delegate = actor_from_connection(&tx, &r.credential_id)?;
            delegate.require("send")?;
            authorized_conversation(&tx, &delegate, &r.conversation_id)?;
            let message: String = tx
                .query_row(
                    "SELECT message_id FROM provider_jobs WHERE id=?1",
                    [&id],
                    |r| r.get(0),
                )
                .map_err(|_| Error::storage())?;
            crate::audience::require_original(
                &tx,
                &actor,
                &r.conversation_id,
                connect_protocol::membership::ContentKind::Message,
                &message,
            )?;
            crate::audience::require_original(
                &tx,
                &delegate,
                &r.conversation_id,
                connect_protocol::membership::ContentKind::Message,
                &message,
            )?;
            Ok::<_, Error>(r.enabled && !conv.closed)
        })();
        if allowed != Ok(true) {
            tx.execute(
                "UPDATE provider_jobs SET state='cancelled' WHERE id=?1",
                [id],
            )
            .map_err(|_| Error::storage())?;
            tx.commit().map_err(|_| Error::storage())?;
            return Ok(None);
        }
        match crate::connector_registry::require_operation(
            &tx,
            &r.connector,
            connect_connector_api::manifest::ConnectorOperation::MessageSend,
        ) {
            Ok(()) => (),
            Err(error) if error.code == "unsupported-connector-operation" => {
                // Ordinary native communication remains journaled. Its fanout
                // intent is explicitly failed without claiming a remote send or
                // selecting/exporting context for a receive-only adapter.
                tx.execute("UPDATE provider_jobs SET state='failed' WHERE id=?1", [&id])
                    .map_err(|_| Error::storage())?;
                tx.commit().map_err(|_| Error::storage())?;
                return Ok(None);
            }
            Err(error) => return Err(error),
        }
        if self.connector(&r.connector)?.sms {
            let source = r.source.as_deref().unwrap_or_default();
            let decision = match crate::tasks::task_sms_decision(
                &tx,
                &id,
                &r.target,
                source,
                &r.conversation_id,
            )? {
                Some(decision) => decision,
                None => {
                    crate::sms_eligibility::eligible(&tx, &r.target, source, &r.conversation_id)?
                }
            };
            match decision {
                Ok(grant) => {
                    tx.execute("UPDATE provider_jobs SET sms_decision='allowed',sms_grant_id=?2 WHERE id=?1",params![id,grant]).map_err(|_|Error::storage())?;
                }
                Err(reason) => {
                    tx.execute(
                        "UPDATE provider_jobs SET state='cancelled',sms_decision=?2 WHERE id=?1",
                        params![id, reason],
                    )
                    .map_err(|_| Error::storage())?;
                    tx.commit().map_err(|_| Error::storage())?;
                    return Ok(None);
                }
            }
        }
        // An uncertain/failed conversational turn blocks subsequent turns until explicitly
        // resolved; otherwise chat context could silently lose the prior input.
        if self.connector(&r.connector)?.conversational && tx.query_row::<bool,_,_>("SELECT EXISTS(SELECT 1 FROM provider_jobs WHERE route_id=?1 AND state IN ('uncertain','failed'))",[&rid],|r|r.get(0)).map_err(|_|Error::storage())? {tx.execute("UPDATE provider_jobs SET state='blocked' WHERE id=?1",[&id]).map_err(|_|Error::storage())?;tx.commit().map_err(|_|Error::storage())?;return Ok(None);}
        let (context, task_origin) = crate::provider_chat_context::begin(
            &tx,
            &credential,
            &r,
            &id,
            self.connector(&r.connector)?.conversational,
            self,
        )?;
        // Remote memory is not authority. A changed membership/sharing revision,
        // identity, membership epoch or credential ceiling starts fresh memory.
        let policy = crate::provider_chat_context::authority_policy(self, &tx, &credential, &r)?;
        let previous_chat_id = if task_origin {
            None
        } else {
            tx.query_row(
                "SELECT chat_id FROM provider_context WHERE route_id=?1 AND authority_policy=?2",
                params![rid, policy],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| Error::storage())?
        };
        tx.execute(
            "UPDATE provider_jobs SET state='sending',authority_policy=?2 WHERE id=?1",
            params![id, policy],
        )
        .map_err(|_| Error::storage())?;
        let claim_token = uuid::Uuid::new_v4().to_string();
        tx.execute("UPDATE provider_dispatch SET runtime_id=?2,claim_token=?3,previous_chat_id=?4 WHERE job_id=?1 AND claim_token IS NULL", params![id,self.runtime_epoch(),claim_token,previous_chat_id]).map_err(|_| Error::storage())?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(Some(ProviderJob {
            id,
            route: r,
            body,
            previous_chat_id,
            context,
            claim_token: Some(claim_token),
        }))
    }
    pub fn finish_provider_job(
        &self,
        job: &ProviderJob,
        state: &str,
        provider_id: Option<&str>,
        reply: Option<&str>,
    ) -> Result<()> {
        if !["accepted", "completed", "failed", "uncertain"].contains(&state) {
            return Err(Error::new("invalid-request", "Invalid job state"));
        }
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
        let current: String = tx
            .query_row(
                "SELECT state FROM provider_jobs WHERE id=?1",
                [&job.id],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        if current != "sending" {
            return Err(Error::new("conflict", "Job is no longer in flight"));
        }
        let route_id: String = tx
            .query_row(
                "SELECT route_id FROM provider_jobs WHERE id=?1",
                [&job.id],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        if route_id != job.route.id {
            return Err(Error::forbidden());
        }
        let r = route(&tx, &job.route.id)?;
        if let Some(body) = reply {
            // Both legacy completions and normalized observations recheck the
            // original job source; a read share cannot revive an old send.
            let (admitted, _) =
                crate::connector_observations::authorized_job(&tx, &job.id, &r.connector)?;
            if admitted.route.id != r.id {
                return Err(Error::forbidden());
            }

            let actor = actor_from_connection(&tx, &r.credential_id)?;
            actor.require("send")?;
            let conversation = authorized_conversation(&tx, &actor, &r.conversation_id)?;
            if !r.enabled || conversation.closed {
                return Err(Error::forbidden());
            }
            if crate::tasks::provider_task_reply(&tx, &actor, &job.id, body, self)?.is_none() {
                self.insert_provider_reply(&tx, &r, &job.id, body)?;
            }
        }
        if state == "completed" && self.connector(&r.connector)?.conversational {
            let pid = provider_id
                .ok_or_else(|| Error::new("invalid-request", "Missing provider chat ID"))?;
            tx.execute("INSERT INTO provider_context(route_id,chat_id,authority_policy) VALUES(?1,?2,(SELECT authority_policy FROM provider_jobs WHERE id=?3)) ON CONFLICT(route_id) DO UPDATE SET chat_id=excluded.chat_id,authority_policy=excluded.authority_policy",params![r.id,pid,job.id]).map_err(|_|Error::storage())?;
        }
        tx.execute(
            "UPDATE provider_jobs SET state=?2,provider_id=?3 WHERE id=?1",
            params![job.id, state, provider_id],
        )
        .map_err(|_| Error::storage())?;
        crate::provider_chat_context::finish(&tx, &job.id, state, provider_id)?;
        // A final callback may precede the API response. Reconcile its saved state.
        if r.connector == "telnyx" && state == "accepted" {
            if let Some(pid) = provider_id {
                for final_state in ["delivered", "failed"] {
                    let event_id = sms_final_key(&tx, &r.id, pid, final_state)?;
                    let seen: bool = tx
                        .query_row(
                            "SELECT EXISTS(SELECT 1 FROM provider_events WHERE id=?1)",
                            [event_id],
                            |r| r.get(0),
                        )
                        .map_err(|_| Error::storage())?;
                    if seen {
                        tx.execute(
                            "UPDATE provider_jobs SET state=?2 WHERE id=?1",
                            params![job.id, final_state],
                        )
                        .map_err(|_| Error::storage())?;
                        break;
                    }
                }
            }
        }
        tx.commit().map_err(|_| Error::storage())
    }
    pub(crate) fn insert_provider_message(
        &self,
        c: &Connection,
        r: &Route,
        body: &str,
    ) -> Result<String> {
        let actor = actor_from_connection(c, &r.credential_id)?;
        actor.require("send")?;
        if !r.enabled {
            return Err(Error::forbidden());
        }
        crate::messages::write(
            c,
            &actor,
            &r.conversation_id,
            connect_protocol::messages::MessageDraft::text(r.incoming_to.clone(), body),
            self,
        )
    }
    pub(crate) fn insert_provider_reply(
        &self,
        c: &Connection,
        r: &Route,
        job: &str,
        body: &str,
    ) -> Result<String> {
        let actor = actor_from_connection(c, &r.credential_id)?;
        actor.require("send")?;
        let (question,owner):(String,String)=c.query_row("SELECT j.message_id,m.sender FROM provider_jobs j JOIN messages m ON m.id=j.message_id WHERE j.id=?1 AND j.route_id=?2",params![job,r.id],|row|Ok((row.get(0)?,row.get(1)?))).map_err(|_|Error::forbidden())?;
        let target = crate::audience::member(c, &owner, &r.conversation_id, true)?;
        let mut draft = connect_protocol::messages::MessageDraft::text(
            connect_protocol::messages::MessageAudience::Participants(vec![target.participant_id]),
            body,
        );
        draft.in_reply_to_msg = Some(question);
        crate::messages::write(c, &actor, &r.conversation_id, draft, self)
    }

    /// Only called after HTTP verifies the raw Telnyx Ed25519 signature. Route
    /// number pairs are explicit; unrecognized senders never acquire an identity.
    pub fn receive_sms(&self, incoming: &crate::SmsObservation<'_>) -> Result<bool> {
        crate::incoming_contacts::sms_receive(self, incoming, None)
    }
    pub fn receive_sms_for_routes(
        &self,
        incoming: &crate::SmsObservation<'_>,
        routes: &[String],
    ) -> Result<bool> {
        crate::incoming_contacts::sms_receive(self, incoming, Some(routes))
    }
    pub fn delivery_sms(
        &self,
        event_id: &str,
        digest: &str,
        provider_id: &str,
        state: &str,
    ) -> Result<bool> {
        self.delivery_sms_scope(event_id, digest, provider_id, state, None, None)
    }
    pub fn delivery_sms_for_routes(
        &self,
        event_id: &str,
        digest: &str,
        provider_id: &str,
        state: &str,
        routes: &[String],
        pair: Option<(&str, &str)>,
    ) -> Result<bool> {
        self.delivery_sms_scope(event_id, digest, provider_id, state, Some(routes), pair)
    }
    fn delivery_sms_scope(
        &self,
        event_id: &str,
        digest: &str,
        provider_id: &str,
        state: &str,
        routes: Option<&[String]>,
        pair: Option<(&str, &str)>,
    ) -> Result<bool> {
        if !["delivered", "failed"].contains(&state) {
            return Err(Error::new("invalid-request", "Invalid delivery state"));
        }
        let scope = routes
            .map(serde_json::to_string)
            .transpose()
            .map_err(|_| Error::storage())?;
        let (source, target) = pair.unwrap_or(("", ""));
        // Retaining an early signed fact changes no Conversation. Do not
        // acquire a writer behind the original transport's read custody. Only
        // already-attributed jobs need the journal/effect mutation authority.
        const TARGETS: &str = "SELECT DISTINCT r.conversation_id FROM connector_routes r WHERE r.connector='telnyx' AND (?2 IS NULL OR r.id IN (SELECT value FROM json_each(?2))) AND EXISTS(SELECT 1 FROM provider_jobs j WHERE j.route_id=r.id AND j.provider_id=?1)";
        let authority = self.runtime_query_authority(TARGETS, &[&provider_id, &scope])?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        self.require_runtime_query(&tx, &authority, TARGETS, &[&provider_id, &scope])?;
        let mut statement = tx.prepare("SELECT DISTINCT r.id FROM connector_routes r WHERE r.connector='telnyx' AND (?2 IS NULL OR r.id IN (SELECT value FROM json_each(?2))) AND (EXISTS(SELECT 1 FROM provider_jobs j WHERE j.route_id=r.id AND j.provider_id=?1) OR (r.source=?3 AND r.target=?4))").map_err(|_| Error::storage())?;
        let ids = statement
            .query_map(params![provider_id, scope, source, target], |r| {
                r.get::<_, String>(0)
            })
            .map_err(|_| Error::storage())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| Error::storage())?;
        drop(statement);
        // A scoped listener cannot infer account/route ownership from an unknown
        // provider ID. A signed number pair can attribute an early final fact.
        if routes.is_some() && ids.len() != 1 {
            return Err(Error::forbidden());
        }
        for id in &ids {
            crate::provider_message_endpoints::current(&tx, &route(&tx, id)?)?;
        }
        // A later callback cannot silently overturn an authenticated terminal
        // fact. Conflicting final states require explicit status reconciliation.
        for id in &ids {
            let opposite = sms_final_key(
                &tx,
                id,
                provider_id,
                if state == "delivered" {
                    "failed"
                } else {
                    "delivered"
                },
            )?;
            let conflicting: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM provider_events WHERE id=?1)",
                    [&opposite],
                    |r| r.get(0),
                )
                .map_err(|_| Error::storage())?;
            if conflicting {
                return Err(Error::new(
                    "conflict",
                    "Conflicting final SMS observation requires reconciliation",
                ));
            }
        }

        if !admit_event(&tx, event_id, digest, self.limits.operation_receipts)? {
            return Ok(false);
        }
        if routes.is_none() {
            admit_event(
                &tx,
                &format!("telnyx-final:{provider_id}:{state}"),
                state,
                self.limits.operation_receipts,
            )?;
        }
        for id in ids {
            let key = sms_final_key(&tx, &id, provider_id, state)?;
            admit_event(&tx, &key, state, self.limits.operation_receipts)?;
            let mut st = tx.prepare("SELECT id FROM provider_jobs WHERE route_id=?1 AND provider_id=?2 AND state IN ('accepted','uncertain','delivered','failed')").map_err(|_| Error::storage())?;
            let jobs = st
                .query_map(params![id, provider_id], |r| r.get::<_, String>(0))
                .map_err(|_| Error::storage())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|_| Error::storage())?;
            drop(st);
            for job in jobs {
                self.apply_connector_receipt_tx(
                    &tx,
                    "telnyx",
                    &sms_final_receipt(&job, provider_id, event_id, state),
                )?;
            }
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok(true)
    }
}
pub(crate) fn sms_final_key(
    c: &Connection,
    route: &str,
    provider: &str,
    state: &str,
) -> Result<String> {
    let bound: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM provider_message_endpoints WHERE route_id=?1)",
            [route],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    Ok(if bound {
        format!(
            "telnyx-final-route:{}:{state}",
            event_digest(format!("{route}\0{provider}").as_bytes())
        )
    } else {
        format!("telnyx-final:{provider}:{state}")
    })
}
pub(crate) fn sms_final_receipt(
    job: &str,
    provider: &str,
    event: &str,
    state: &str,
) -> connect_connector_api::lifecycle::MessageReceipt {
    use connect_connector_api::lifecycle::*;
    MessageReceipt {
        observation: Observation {
            mutation_id: job.into(),
            external_id: Some(provider.into()),
            event_id: event.into(),
            state: State::Message(if state == "delivered" {
                MessageState::Delivered
            } else {
                MessageState::Failed
            }),
            verification: Verification::Reconciliation,
        },
        reply: None,
        reply_sources: vec![],
    }
}
pub(crate) fn admit_event(c: &Connection, id: &str, digest: &str, max: u64) -> Result<bool> {
    if id.is_empty() || id.len() > 256 || digest.len() > 128 {
        return Err(Error::new("invalid-request", "Invalid event ID or digest"));
    }
    let saved: Option<String> = c
        .query_row(
            "SELECT digest FROM provider_events WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| Error::storage())?;
    if let Some(saved) = saved {
        if saved != digest {
            return Err(Error::new(
                "conflict",
                "Provider event ID has different content",
            ));
        }
        return Ok(false);
    }
    let count: u64 = c
        .query_row("SELECT COUNT(*) FROM provider_events", [], |r| r.get(0))
        .map_err(|_| Error::storage())?;
    if count >= max {
        return Err(Error::new(
            "capacity-exceeded",
            "Provider event capacity exceeded",
        ));
    }
    c.execute(
        "INSERT INTO provider_events VALUES(?1,?2)",
        params![id, digest],
    )
    .map_err(|_| Error::storage())?;
    Ok(true)
}
pub fn event_digest(raw: &[u8]) -> String {
    hex::encode(Sha256::digest(raw))
}
