//! Durable voice commands. Only a claimed start may create a provider call.
use crate::{
    conversations::{authorized_conversation, capacity},
    identity::actor_from_connection,
    Actor, Store,
};
use connect_protocol::{Call, Error, Outcome, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VoiceRoute {
    pub id: String,
    pub conversation_id: String,
    pub identity_id: String,
    pub assistant_id: String,
    pub phone_number_id: String,
    pub destinations: Vec<String>,
    pub enabled: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceAction {
    Start,
    End,
    Poll,
}
/// Contains a private provider control capability; deliberately does not implement Debug.
pub struct VoiceJob {
    pub call: Call,
    pub route: VoiceRoute,
    pub action: VoiceAction,
    pub control_url: Option<String>,
}
type VoiceClaim = (String, String, String, u8, Option<String>, Option<String>);

fn invalid() -> Error {
    Error::new("invalid-request", "Invalid voice route or destination")
}
fn route(c: &Connection, id: &str) -> Result<VoiceRoute> {
    let (conversation_id, identity_id, assistant_id, phone_number_id, destinations, enabled): (String,String,String,String,String,bool) = c.query_row("SELECT conversation_id,identity_id,assistant_id,phone_number_id,destinations,enabled FROM voice_routes WHERE id=?1", [id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional().map_err(|_| Error::storage())?.ok_or_else(Error::forbidden)?;
    Ok(VoiceRoute {
        id: id.into(),
        conversation_id,
        identity_id,
        assistant_id,
        phone_number_id,
        destinations: serde_json::from_str(&destinations).map_err(|_| Error::storage())?,
        enabled,
    })
}
pub(crate) fn get(c: &Connection, conversation: &str, id: &str) -> Result<Call> {
    c.query_row("SELECT id,conversation_id,route_id,destination,state,provider_id,ended_reason,direction,end_requested FROM voice_calls WHERE id=?1 AND conversation_id=?2", params![id,conversation], |r| Ok(Call { id:r.get(0)?,conversation_id:r.get(1)?,route_id:r.get(2)?,destination:r.get(3)?,state:r.get(4)?,provider_id:r.get(5)?,ended_reason:r.get(6)?,direction:r.get(7)?,end_requested:r.get::<_,u8>(8)?>0 })).optional().map_err(|_| Error::storage())?.ok_or_else(Error::forbidden)
}
fn event(c: &Connection, call: &Call, store: &Store) -> Result<()> {
    let mut stmt = c
        .prepare(
            "SELECT identity_id FROM participants WHERE conversation_id=?1 ORDER BY identity_id",
        )
        .map_err(|_| Error::storage())?;
    let members = stmt
        .query_map([&call.conversation_id], |r| r.get::<_, String>(0))
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    if !capacity(c, members.len() as u64, 0, store)? {
        return Err(Error::new(
            "capacity-exceeded",
            "Durable call event capacity exceeded",
        ));
    }
    let json = serde_json::to_string(call).map_err(|_| Error::storage())?;
    for member in members {
        c.execute("INSERT INTO journal(id,recipient,conversation_id,kind,call_json) VALUES(?1,?2,?3,'call_state',?4)", params![uuid::Uuid::new_v4().to_string(),member,call.conversation_id,json]).map_err(|_| Error::storage())?;
    }
    Ok(())
}
pub(crate) fn start(
    c: &Connection,
    actor: &Actor,
    credential: &str,
    conversation: &str,
    route_id: &str,
    destination: &str,
    store: &Store,
) -> Result<Outcome> {
    let conversation_value = authorized_conversation(c, actor, conversation)?;
    if conversation_value.closed {
        return Err(Error::new(
            "conversation-closed",
            "Conversation no longer accepts calls",
        ));
    }
    let r = route(c, route_id)?;
    if !r.enabled
        || r.conversation_id != conversation
        || !r.destinations.iter().any(|d| d == destination)
    {
        return Err(Error::forbidden());
    }
    let count: u64 = c
        .query_row("SELECT COUNT(*) FROM voice_calls", [], |r| r.get(0))
        .map_err(|_| Error::storage())?;
    if count >= store.limits.operation_receipts {
        return Err(Error::new(
            "capacity-exceeded",
            "Durable call capacity exceeded",
        ));
    }
    let call = Call {
        end_requested: false,
        direction: "outbound".into(),
        id: uuid::Uuid::new_v4().to_string(),
        conversation_id: conversation.into(),
        route_id: route_id.into(),
        destination: destination.into(),
        state: "pending".into(),
        provider_id: None,
        ended_reason: None,
    };
    // Capacity checked before insertion, to allow a durable rejection receipt.
    if !capacity(c, conversation_value.participants.len() as u64, 0, store)? {
        return Err(Error::new(
            "capacity-exceeded",
            "Durable call event capacity exceeded",
        ));
    }
    c.execute("INSERT INTO voice_calls(id,conversation_id,route_id,credential_id,destination) VALUES(?1,?2,?3,?4,?5)", params![call.id,conversation,route_id,credential,destination]).map_err(|_| Error::storage())?;
    event(c, &call, store)?;
    Ok(Outcome::Call(call))
}
pub(crate) fn end(
    c: &Connection,
    credential: &str,
    conversation: &str,
    id: &str,
    store: &Store,
) -> Result<Outcome> {
    let mut call = get(c, conversation, id)?;
    if ["ended", "failed", "cancelled", "forbidden"].contains(&call.state.as_str()) {
        return Ok(Outcome::Call(call));
    }
    if call.state == "uncertain" && call.provider_id.is_none() {
        return Err(Error::new(
            "provider-uncertain",
            "Provider creation is uncertain; reconcile it before ending",
        ));
    }
    let members: u64 = c
        .query_row(
            "SELECT COUNT(*) FROM participants WHERE conversation_id=?1",
            [conversation],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if !capacity(c, members, 0, store)? {
        return Err(Error::new(
            "capacity-exceeded",
            "Durable call event capacity exceeded",
        ));
    }
    if call.state == "pending" {
        call.state = "cancelled".into();
    }
    c.execute(
        "UPDATE voice_calls SET end_requested=CASE WHEN ?2='cancelled' THEN 0 ELSE 1 END,end_credential_id=?3,state=?2,next_poll=0 WHERE id=?1",
        params![id, call.state, credential],
    )
    .map_err(|_| Error::storage())?;
    call = get(c, conversation, id)?;
    event(c, &call, store)?;
    Ok(Outcome::Call(call))
}
impl Store {
    /// Immutable operator binding. Enable/disable is the only in-place change.
    pub fn configure_voice_route(&self, operator: &str, value: &VoiceRoute) -> Result<()> {
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let actor = actor_from_connection(&tx, operator)?;
        actor.require("admin")?;
        let allowed:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM conversations c JOIN participants p ON p.conversation_id=c.id WHERE c.id=?1 AND c.closed=0 AND p.identity_id=?2)",params![value.conversation_id,value.identity_id],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !allowed {
            return Err(Error::forbidden());
        }
        let members = {
            let mut st = tx
                .prepare("SELECT identity_id FROM participants WHERE conversation_id=?1")
                .map_err(|_| Error::storage())?;
            let members = st
                .query_map([&value.conversation_id], |r| r.get::<_, String>(0))
                .map_err(|_| Error::storage())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|_| Error::storage())?;
            members
        };
        if members.iter().any(|p| !actor.permits_recipient(p)) {
            return Err(Error::forbidden());
        }
        let e164 = |d: &str| {
            d.starts_with('+')
                && (8..=16).contains(&d.len())
                && d[1..].chars().all(|c| c.is_ascii_digit())
        };
        if value.id.is_empty()
            || value.id.len() > 128
            || !value
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            || uuid::Uuid::parse_str(&value.assistant_id).is_err()
            || uuid::Uuid::parse_str(&value.phone_number_id).is_err()
            || value.destinations.is_empty()
            || value.destinations.len() > 100
            || !value.destinations.iter().all(|d| e164(d))
        {
            return Err(invalid());
        }
        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM voice_routes WHERE id=?1)",
                [&value.id],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        if exists {
            let mut old = route(&tx, &value.id)?;
            old.enabled = value.enabled;
            if old != *value {
                return Err(Error::new("conflict", "Voice route bindings are immutable"));
            }
            tx.execute(
                "UPDATE voice_routes SET enabled=?2 WHERE id=?1",
                params![value.id, value.enabled],
            )
            .map_err(|_| Error::storage())?;
        } else {
            tx.execute(
                "INSERT INTO voice_routes VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![
                    value.id,
                    value.conversation_id,
                    value.identity_id,
                    value.assistant_id,
                    value.phone_number_id,
                    serde_json::to_string(&value.destinations).map_err(|_| Error::storage())?,
                    value.enabled
                ],
            )
            .map_err(|_| Error::storage())?;
        }
        tx.commit().map_err(|_| Error::storage())
    }
    /// Crashed starts are never retried: provider creation may have succeeded.
    /// Interrupted end requests can be reconciled by read-only status polling.
    pub fn recover_voice_calls(&self) -> Result<()> {
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let rows = {
            let mut stmt=tx.prepare("SELECT id,conversation_id FROM voice_calls WHERE state IN ('starting','ending')").map_err(|_| Error::storage())?;
            let rows = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .map_err(|_| Error::storage())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|_| Error::storage())?;
            rows
        };
        for (id, conv) in rows {
            tx.execute(
                "UPDATE voice_calls SET state='uncertain',end_requested=CASE WHEN end_requested>0 THEN 2 ELSE 0 END,next_poll=0 WHERE id=?1",
                [&id],
            )
            .map_err(|_| Error::storage())?;
            event(&tx, &get(&tx, &conv, &id)?, self)?;
        }
        tx.commit().map_err(|_| Error::storage())
    }
    pub fn claim_voice_job(&self) -> Result<Option<VoiceJob>> {
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row:Option<VoiceClaim>=tx.query_row("SELECT id,conversation_id,credential_id,end_requested,control_url,end_credential_id FROM voice_calls WHERE state='pending' OR (provider_id IS NOT NULL AND state NOT IN ('ended','failed','cancelled','forbidden','starting','ending') AND next_poll<=?1) ORDER BY end_requested DESC,sequence LIMIT 1",[crate::now()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional().map_err(|_| Error::storage())?;
        let Some((id, conv, credential, end_requested, control_url, end_credential)) = row else {
            return Ok(None);
        };
        let mut call = get(&tx, &conv, &id)?;
        let r = route(&tx, &call.route_id)?;
        let action = if call.state == "pending" {
            let authorized = actor_from_connection(&tx, &credential).and_then(|a| {
                a.require("call_start")?;
                authorized_conversation(&tx, &a, &conv)
            });
            if !r.enabled || !matches!(authorized,Ok(ref c) if !c.closed) {
                call.state = "forbidden".into();
                tx.execute(
                    "UPDATE voice_calls SET state='forbidden' WHERE id=?1",
                    [&id],
                )
                .map_err(|_| Error::storage())?;
                event(&tx, &call, self)?;
                tx.commit().map_err(|_| Error::storage())?;
                return Ok(None);
            }
            call.state = "starting".into();
            VoiceAction::Start
        } else if end_requested == 1 {
            let authorized = end_credential
                .as_deref()
                .ok_or_else(Error::unauthorized)
                .and_then(|key| actor_from_connection(&tx, key))
                .and_then(|a| {
                    a.require("call_end")?;
                    authorized_conversation(&tx, &a, &conv)
                });
            if authorized.is_err() {
                tx.execute("UPDATE voice_calls SET end_requested=0,end_credential_id=NULL,next_poll=0 WHERE id=?1",[&id]).map_err(|_|Error::storage())?;
                event(&tx, &get(&tx, &conv, &id)?, self)?;
                tx.commit().map_err(|_| Error::storage())?;
                return Ok(None);
            }
            tx.execute("UPDATE voice_calls SET end_requested=2 WHERE id=?1", [&id])
                .map_err(|_| Error::storage())?;
            call.state = "ending".into();
            VoiceAction::End
        } else {
            VoiceAction::Poll
        };
        tx.execute(
            "UPDATE voice_calls SET state=?2,next_poll=?3 WHERE id=?1",
            params![id, call.state, crate::now() + 2],
        )
        .map_err(|_| Error::storage())?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(Some(VoiceJob {
            call,
            route: r,
            action,
            control_url,
        }))
    }
    pub fn finish_voice_job(
        &self,
        job: &VoiceJob,
        state: &str,
        provider_id: Option<&str>,
        control_url: Option<&str>,
        reason: Option<&str>,
    ) -> Result<()> {
        if ![
            "queued",
            "ringing",
            "in-progress",
            "ended",
            "failed",
            "uncertain",
        ]
        .contains(&state)
            || reason.is_some_and(|r| r.len() > 256)
        {
            return Err(invalid());
        }
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let old = get(&tx, &job.call.conversation_id, &job.call.id)?;
        tx.execute("UPDATE voice_calls SET state=?2,provider_id=COALESCE(?3,provider_id),control_url=COALESCE(?4,control_url),ended_reason=?5,end_requested=CASE WHEN ?2 IN ('ended','failed') THEN 0 ELSE end_requested END WHERE id=?1",params![old.id,state,provider_id,control_url,reason]).map_err(|_| Error::storage())?;
        let saved = get(&tx, &old.conversation_id, &old.id)?;
        if saved != old {
            event(&tx, &saved, self)?;
        }
        tx.commit().map_err(|_| Error::storage())
    }
}

/// Explicit inbound routing is separate from outbound dial authority. Caller ID is
/// a routing hint, never authentication or a grant to execute an agent action.
#[derive(Clone)]
pub struct VoiceIngress {
    pub route: VoiceRoute,
    pub phone_number_id: String,
    pub since: i64,
    /// Last completely committed discovery window, in Unix milliseconds.
    pub scanned_until: i64,
    /// Fixed upper bound chosen before contacting the provider.
    pub scan_until: i64,
}
pub struct InboundCall {
    pub provider_id: String,
    pub assistant_id: String,
    pub phone_number_id: String,
    pub caller: String,
    pub created_at: i64,
    pub state: String,
    pub control_url: Option<String>,
    pub ended_reason: Option<String>,
}
impl Store {
    pub fn configure_voice_ingress(
        &self,
        operator: &str,
        route_id: &str,
        phone: &str,
    ) -> Result<()> {
        if uuid::Uuid::parse_str(phone).is_err() {
            return Err(invalid());
        }
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let actor = actor_from_connection(&tx, operator)?;
        actor.require("admin")?;
        let r = route(&tx, route_id)?;
        ingress_authority(&tx, &actor, &r)?;
        let existing: Option<(String, String)> = tx
            .query_row(
                "SELECT phone_number_id,credential_id FROM voice_ingress WHERE route_id=?1",
                [route_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|_| Error::storage())?;
        if let Some((p, k)) = existing {
            if p != phone || k != operator {
                return Err(Error::new("conflict", "Inbound bindings are immutable"));
            }
        } else {
            tx.execute(
                "INSERT INTO voice_ingress(route_id,phone_number_id,credential_id,since) VALUES(?1,?2,?3,?4)",
                params![route_id, phone, operator, crate::now()],
            )
            .map_err(|_| Error::storage())?;
        }
        tx.commit().map_err(|_| Error::storage())
    }
    pub fn voice_ingresses(&self) -> Result<Vec<VoiceIngress>> {
        let c = self.connection()?;
        let mut stmt = c.prepare("SELECT i.route_id,i.phone_number_id,i.since,i.scanned_until,i.credential_id FROM voice_ingress i JOIN voice_routes r ON r.id=i.route_id WHERE r.enabled=1 ORDER BY i.route_id").map_err(|_| Error::storage())?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })
            .map_err(|_| Error::storage())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| Error::storage())?;
        let mut bindings = Vec::new();
        for (id, phone_number_id, since, scanned_until, key) in rows {
            let r = route(&c, &id)?;
            let authorized =
                actor_from_connection(&c, &key).and_then(|a| ingress_authority(&c, &a, &r));
            if authorized.is_ok() {
                bindings.push(VoiceIngress {
                    route: r,
                    phone_number_id,
                    since,
                    scanned_until,
                    scan_until: chrono::Utc::now().timestamp_millis() - 2000,
                });
            }
        }
        Ok(bindings)
    }
    /// Advance only after every retrieved record was committed or denied by policy.
    pub fn complete_voice_scan(&self, binding: &VoiceIngress) -> Result<()> {
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let r = route(&tx, &binding.route.id)?;
        let key: String = tx
            .query_row(
                "SELECT credential_id FROM voice_ingress WHERE route_id=?1",
                [&r.id],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        let actor = actor_from_connection(&tx, &key)?;
        ingress_authority(&tx, &actor, &r)?;
        if !r.enabled
            || binding.scan_until > chrono::Utc::now().timestamp_millis()
            || binding.scan_until < binding.scanned_until
        {
            return Err(invalid());
        }
        tx.execute(
            "UPDATE voice_ingress SET scanned_until=MAX(scanned_until,?2) WHERE route_id=?1",
            params![r.id, binding.scan_until],
        )
        .map_err(|_| Error::storage())?;
        tx.commit().map_err(|_| Error::storage())
    }
    pub fn import_voice_call(&self, route_id: &str, incoming: &InboundCall) -> Result<bool> {
        if uuid::Uuid::parse_str(&incoming.provider_id).is_err()
            || !["queued", "ringing", "in-progress", "ended", "uncertain"]
                .contains(&incoming.state.as_str())
            || incoming
                .ended_reason
                .as_ref()
                .is_some_and(|r| r.len() > 256)
        {
            return Err(invalid());
        }
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let r = route(&tx, route_id)?;
        let (phone, key, since): (String, String, i64) = tx
            .query_row(
                "SELECT phone_number_id,credential_id,since FROM voice_ingress WHERE route_id=?1",
                [route_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(|_| Error::forbidden())?;
        let actor = actor_from_connection(&tx, &key)?;
        actor.require("admin")?;
        ingress_authority(&tx, &actor, &r)?;
        if !r.enabled
            || phone != incoming.phone_number_id
            || r.assistant_id != incoming.assistant_id
            || incoming.created_at < since
            || !r.destinations.contains(&incoming.caller)
        {
            return Err(Error::forbidden());
        }
        let existing: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM voice_calls WHERE provider_id=?1)",
                [&incoming.provider_id],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        if existing {
            return Ok(false);
        } // Existing calls are updated by their owned lifecycle poll.
        let count: u64 = tx
            .query_row("SELECT COUNT(*) FROM voice_calls", [], |r| r.get(0))
            .map_err(|_| Error::storage())?;
        if count >= self.limits.operation_receipts {
            return Err(Error::new(
                "capacity-exceeded",
                "Durable call capacity exceeded",
            ));
        }
        let id = uuid::Uuid::new_v4().to_string();
        tx.execute("INSERT INTO voice_calls(id,conversation_id,route_id,credential_id,destination,state,provider_id,control_url,ended_reason,direction) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,'inbound')",params![id,r.conversation_id,route_id,key,incoming.caller,incoming.state,incoming.provider_id,incoming.control_url,incoming.ended_reason]).map_err(|_| Error::storage())?;
        event(&tx, &get(&tx, &r.conversation_id, &id)?, self)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(true)
    }
}

fn ingress_authority(c: &Connection, actor: &Actor, r: &VoiceRoute) -> Result<()> {
    actor.require("admin")?;
    let open:bool = c.query_row("SELECT EXISTS(SELECT 1 FROM conversations c JOIN participants p ON p.conversation_id=c.id WHERE c.id=?1 AND c.closed=0 AND p.identity_id=?2)",params![r.conversation_id,r.identity_id],|r|r.get(0)).map_err(|_|Error::storage())?;
    let mut st = c
        .prepare("SELECT identity_id FROM participants WHERE conversation_id=?1")
        .map_err(|_| Error::storage())?;
    let members = st
        .query_map([&r.conversation_id], |r| r.get::<_, String>(0))
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    if !open || members.iter().any(|m| !actor.permits_recipient(m)) {
        return Err(Error::forbidden());
    }
    Ok(())
}
