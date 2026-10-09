//! Transactional connector boundary. Network effects never run inside SQLite.
use crate::{
    conversations::{authorized_conversation, capacity, insert_event},
    identity::actor_from_connection,
    Store,
};
use connect_protocol::{Error, ProviderDelivery, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
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
}
#[derive(Clone, Debug)]
pub struct ProviderJob {
    pub id: String,
    pub route: Route,
    pub body: String,
    pub previous_chat_id: Option<String>,
    /// Exact serialized context selection journaled before provider dispatch.
    pub context: Option<String>,
}
fn route(c: &Connection, id: &str) -> Result<Route> {
    c.query_row("SELECT id,identity_id,credential_id,conversation_id,connector,target,source,enabled FROM connector_routes WHERE id=?1", [id], |r| Ok(Route { id:r.get(0)?,identity_id:r.get(1)?,credential_id:r.get(2)?,conversation_id:r.get(3)?,connector:r.get(4)?,target:r.get(5)?,source:r.get(6)?,enabled:r.get(7)? })).map_err(|_|Error::forbidden())
}
pub(crate) fn enqueue(
    c: &Connection,
    message: &str,
    conversation: &str,
    sender: &str,
    credential: &str,
    max: u64,
) -> Result<()> {
    let count: u64 = c
        .query_row("SELECT COUNT(*) FROM provider_jobs", [], |r| r.get(0))
        .map_err(|_| Error::storage())?;
    let routes:u64=c.query_row("SELECT COUNT(*) FROM connector_routes WHERE enabled=1 AND conversation_id=?1 AND identity_id!=?2", params![conversation,sender], |r|r.get(0)).map_err(|_|Error::storage())?;
    if count.saturating_add(routes) > max {
        return Err(Error::new(
            "capacity-exceeded",
            "Provider job capacity exceeded",
        ));
    }
    let mut st=c.prepare("SELECT id FROM connector_routes WHERE enabled=1 AND conversation_id=?1 AND identity_id!=?2").map_err(|_|Error::storage())?;
    let ids = st
        .query_map(params![conversation, sender], |r| r.get::<_, String>(0))
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    for id in ids {
        c.execute(
            "INSERT INTO provider_jobs(id,route_id,message_id,credential_id) VALUES(?1,?2,?3,?4)",
            params![uuid::Uuid::new_v4().to_string(), id, message, credential],
        )
        .map_err(|_| Error::storage())?;
    }
    Ok(())
}
pub(crate) fn status(
    c: &Connection,
    conversation: &str,
    limit: u32,
) -> Result<Vec<ProviderDelivery>> {
    let mut st=c.prepare("SELECT j.id,j.message_id,r.connector,j.state,j.provider_id FROM provider_jobs j JOIN connector_routes r ON j.route_id=r.id WHERE r.conversation_id=?1 ORDER BY j.sequence DESC LIMIT ?2").map_err(|_|Error::storage())?;
    let rows = st
        .query_map(params![conversation, limit], |r| {
            Ok(ProviderDelivery {
                job_id: r.get(0)?,
                message_id: r.get(1)?,
                connector: r.get(2)?,
                state: r.get(3)?,
                provider_id: r.get(4)?,
            })
        })
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    Ok(rows)
}
impl Store {
    /// Local operator command, while the installation is stopped. Bindings are
    /// immutable; only enable/disable is mutable to avoid rerouting queued work.
    pub fn configure_route(&self, operator: &str, value: &Route) -> Result<()> {
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        actor_from_connection(&tx, operator)?.require("admin")?;
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
        if value.connector == "telnyx" {
            let valid = |s: &str| {
                s.starts_with('+')
                    && (8..=16).contains(&s.len())
                    && s[1..].chars().all(|c| c.is_ascii_digit())
            };
            if !valid(&value.target) || !value.source.as_deref().is_some_and(valid) {
                return Err(Error::new(
                    "invalid-request",
                    "SMS routes require E.164 target and source",
                ));
            }
            let duplicate:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM connector_routes WHERE connector='telnyx' AND target=?1 AND source=?2 AND id!=?3)",params![value.target,value.source,value.id],|r|r.get(0)).map_err(|_|Error::storage())?;
            if duplicate {
                return Err(Error::new(
                    "conflict",
                    "SMS number pair already belongs to another conversation",
                ));
            }
        } else if (value.connector == "vapi"
            && (uuid::Uuid::parse_str(&value.target).is_err() || value.source.is_some()))
            || value.connector.is_empty()
            || !value
                .connector
                .chars()
                .all(|c| c.is_ascii_lowercase() || c == '-')
            || value.target.is_empty()
        {
            return Err(Error::new(
                "invalid-request",
                "Unsupported connector or assistant ID",
            ));
        }
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
                "INSERT INTO connector_routes VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    value.id,
                    value.identity_id,
                    value.credential_id,
                    value.conversation_id,
                    value.connector,
                    value.target,
                    value.source,
                    value.enabled
                ],
            )
            .map_err(|_| Error::storage())?;
        }
        tx.commit().map_err(|_| Error::storage())
    }
    /// An interrupted remote mutation has unknown outcome. Never retry it by
    /// default: Telnyx SMS does not provide idempotent message submission.
    pub fn recover_provider_jobs(&self) -> Result<usize> {
        let mut connection = self.connection()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        tx.execute(
            "UPDATE context_deliveries SET state='uncertain',updated_at=?1
             WHERE state='pending' AND id IN (SELECT 'provider-job:' || id FROM provider_jobs WHERE state='sending')",
            [crate::now()],
        ).map_err(|_| Error::storage())?;
        let count = tx
            .execute(
                "UPDATE provider_jobs SET state='uncertain' WHERE state='sending'",
                [],
            )
            .map_err(|_| Error::storage())?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(count)
    }
    pub fn claim_provider_job(&self) -> Result<Option<ProviderJob>> {
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row:Option<(String,String,String,String)>=tx.query_row("SELECT j.id,j.route_id,m.body,j.credential_id FROM provider_jobs j JOIN messages m ON j.message_id=m.id WHERE j.state='pending' ORDER BY j.sequence LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|_|Error::storage())?;
        let Some((id, rid, body, credential)) = row else {
            return Ok(None);
        };
        let r = route(&tx, &rid)?;
        let allowed = (|| {
            let actor = actor_from_connection(&tx, &credential)?;
            actor.require("send")?;
            let conv = authorized_conversation(&tx, &actor, &r.conversation_id)?;
            let delegate = actor_from_connection(&tx, &r.credential_id)?;
            delegate.require("send")?;
            authorized_conversation(&tx, &delegate, &r.conversation_id)?;
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
        // An uncertain/failed Vapi turn blocks subsequent turns until explicitly
        // resolved; otherwise chat context could silently lose the prior input.
        if r.connector=="vapi" && tx.query_row::<bool,_,_>("SELECT EXISTS(SELECT 1 FROM provider_jobs WHERE route_id=?1 AND state IN ('uncertain','failed'))",[&rid],|r|r.get(0)).map_err(|_|Error::storage())? {tx.execute("UPDATE provider_jobs SET state='blocked' WHERE id=?1",[&id]).map_err(|_|Error::storage())?;tx.commit().map_err(|_|Error::storage())?;return Ok(None);}
        let previous_chat_id = tx
            .query_row(
                "SELECT chat_id FROM provider_context WHERE route_id=?1",
                [&rid],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| Error::storage())?;
        let context = crate::provider_chat_context::begin(&tx, &credential, &r, &id)?;
        tx.execute(
            "UPDATE provider_jobs SET state='sending' WHERE id=?1",
            [&id],
        )
        .map_err(|_| Error::storage())?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(Some(ProviderJob {
            id,
            route: r,
            body,
            previous_chat_id,
            context,
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
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
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
        let r = route(&tx, &job.route.id)?;
        if let Some(body) = reply {
            self.insert_provider_message(&tx, &r, body)?;
        }
        if state == "completed" && r.connector == "vapi" {
            let pid = provider_id
                .ok_or_else(|| Error::new("invalid-request", "Missing provider chat ID"))?;
            tx.execute("INSERT INTO provider_context VALUES(?1,?2) ON CONFLICT(route_id) DO UPDATE SET chat_id=excluded.chat_id",params![r.id,pid]).map_err(|_|Error::storage())?;
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
                    let event_id = format!("telnyx-final:{pid}:{final_state}");
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
    fn insert_provider_message(&self, c: &Connection, r: &Route, body: &str) -> Result<String> {
        let actor = actor_from_connection(c, &r.credential_id)?;
        actor.require("send")?;
        let conv = authorized_conversation(c, &actor, &r.conversation_id)?;
        if !r.enabled || conv.closed {
            return Err(Error::forbidden());
        }
        if body.is_empty() || body.len() > self.limits.message_bytes {
            return Err(Error::new(
                "invalid-request",
                "Provider message exceeds body bounds",
            ));
        }
        if !capacity(c, conv.participants.len() as u64, body.len() as u64, self)? {
            return Err(Error::new(
                "capacity-exceeded",
                "Durable provider message capacity exceeded",
            ));
        }
        let id = uuid::Uuid::new_v4().to_string();
        c.execute(
            "INSERT INTO messages(id,conversation_id,sender,body) VALUES(?1,?2,?3,?4)",
            params![id, r.conversation_id, r.identity_id, body],
        )
        .map_err(|_| Error::storage())?;
        for member in &conv.participants {
            insert_event(c, member, &r.conversation_id, "message", Some(&id))?;
        }
        enqueue(
            c,
            &id,
            &r.conversation_id,
            &r.identity_id,
            &r.credential_id,
            self.limits.operation_receipts,
        )?;
        Ok(id)
    }
    /// Only called after HTTP verifies the raw Telnyx Ed25519 signature. Route
    /// number pairs are explicit; unrecognized senders never acquire an identity.
    pub fn receive_sms(
        &self,
        event_id: &str,
        digest: &str,
        from: &str,
        to: &str,
        body: &str,
    ) -> Result<bool> {
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        if !admit_event(&tx, event_id, digest, self.limits.operation_receipts)? {
            return Ok(false);
        }
        let rid:String=tx.query_row("SELECT id FROM connector_routes WHERE connector='telnyx' AND target=?1 AND source=?2 AND enabled=1",params![from,to],|r|r.get(0)).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)?;
        self.insert_provider_message(&tx, &route(&tx, &rid)?, body)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(true)
    }
    pub fn delivery_sms(
        &self,
        event_id: &str,
        digest: &str,
        provider_id: &str,
        state: &str,
    ) -> Result<bool> {
        if !["delivered", "failed"].contains(&state) {
            return Err(Error::new("invalid-request", "Invalid delivery state"));
        }
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        if !admit_event(&tx, event_id, digest, self.limits.operation_receipts)? {
            return Ok(false);
        }
        // Persist early final statuses under a second deterministic key so a
        // later send acknowledgement can reconcile without repeating the send.
        admit_event(
            &tx,
            &format!("telnyx-final:{provider_id}:{state}"),
            state,
            self.limits.operation_receipts,
        )?;
        tx.execute("UPDATE provider_jobs SET state=?2 WHERE provider_id=?1 AND state IN ('accepted','uncertain') AND route_id IN (SELECT id FROM connector_routes WHERE connector='telnyx')",params![provider_id,state]).map_err(|_|Error::storage())?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(true)
    }
}
fn admit_event(c: &Connection, id: &str, digest: &str, max: u64) -> Result<bool> {
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
