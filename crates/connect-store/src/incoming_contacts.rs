//! Private account-scoped incoming custody. Adapters authenticate and validate
//! their exact source/candidate policy before entering this transactional layer.
use crate::{
    identity::actor_from_connection, Actor, AuthorityMode, AuthorityScope, AuthorityWait, Store,
};
use connect_protocol::{incoming_contacts::*, Error, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
mod sms;
pub(crate) use sms::receive as sms_receive;
pub use sms::SmsObservation;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Candidate {
    pub view: IncomingContactCandidate,
    pub identity_id: String,
    pub source: Value,
}
impl Candidate {
    pub(crate) fn seal(mut self) -> Result<Self> {
        self.view.id.clear();
        self.view.id = format!("icc_{}", digest(encoded(&self)?.as_bytes()));
        Ok(self)
    }
}
/// These fields come from protected configured adapter composition, never from
/// the provider's caller-ID or principal-looking payload fields.
pub(crate) struct Source<'a> {
    pub kind: IncomingContactKind,
    pub provider: &'a str,
    pub account_reference: &'a str,
    pub credential_id: &'a str,
    pub binding: Value,
}
pub(crate) struct Capture<'a> {
    pub source: Source<'a>,
    pub provider_id: &'a str,
    pub event_id: &'a str,
    pub event_digest: &'a str,
    pub payload: Value,
    pub candidates: Vec<Candidate>,
    pub reason: IncomingContactReason,
}
fn invalid() -> Error {
    Error::new("invalid-request", "Invalid incoming contact custody")
}
fn conflict() -> Error {
    Error::new(
        "conflict",
        "Incoming contact requires attributed reconciliation",
    )
}
fn digest(raw: &[u8]) -> String {
    hex::encode(Sha256::digest(raw))
}
pub(crate) fn encoded<T: Serialize>(value: &T) -> Result<String> {
    crate::provisioning::canonical_json(&serde_json::to_value(value).map_err(|_| Error::storage())?)
}
fn bounded(v: &str, max: usize) -> bool {
    !v.is_empty() && v.len() <= max && !v.chars().any(char::is_control)
}
fn tenant(c: &Connection, owner: &str) -> Result<String> {
    c.query_row(
        "SELECT tenant_id FROM identities WHERE id=?1 AND revoked=0",
        [owner],
        |r| r.get(0),
    )
    .map_err(|_| Error::forbidden())
}
pub(crate) fn admin(c: &Connection, key: &str) -> Result<(Actor, String)> {
    let actor = actor_from_connection(c, key)?;
    actor.require("admin")?;
    let tenant = tenant(c, &actor.identity_id)?;
    Ok((actor, tenant))
}
pub(crate) fn visible(
    c: &Connection,
    actor: &Actor,
    tenant: &str,
    id: &str,
) -> Result<Vec<Candidate>> {
    let raw: String = c.query_row("SELECT h.candidates_json FROM incoming_contact_holds h JOIN incoming_contact_resources r ON r.id=h.resource_id WHERE h.id=?1 AND r.owner_id=?2 AND r.tenant_id=?3", params![id,actor.identity_id,tenant], |r| r.get(0)).optional().map_err(|_| Error::storage())?.ok_or_else(Error::forbidden)?;
    let candidates: Vec<Candidate> = serde_json::from_str(&raw).map_err(|_| Error::storage())?;
    if candidates
        .iter()
        .any(|p| !actor.permits_recipient(&p.identity_id))
    {
        return Err(Error::forbidden());
    }
    Ok(candidates)
}
pub(crate) fn view(
    c: &Connection,
    actor: &Actor,
    tenant: &str,
    id: &str,
) -> Result<IncomingContact> {
    let candidates = visible(c, actor, tenant, id)?;
    let (kind,provider,reason,created_at,resolved,rejected):(String,String,String,i64,bool,bool)=c.query_row("SELECT r.kind,r.provider,h.reason,h.created_at,EXISTS(SELECT 1 FROM incoming_contact_deliveries d WHERE d.hold_id=h.id),EXISTS(SELECT 1 FROM incoming_contact_rejections x WHERE x.hold_id=h.id) FROM incoming_contact_holds h JOIN incoming_contact_resources r ON r.id=h.resource_id WHERE h.id=?1", [id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).map_err(|_| Error::storage())?;
    let decision: Option<IncomingContactDecision> = c.query_row("SELECT d.operation_id,d.actor_identity_id,d.actor_credential_id,d.candidate_id,d.message_id,json_extract(v.source_json,'$.request.session_id'),d.created_at FROM incoming_contact_deliveries d LEFT JOIN provider_session_voice v ON v.id=d.provider_call_id WHERE d.hold_id=?1 UNION ALL SELECT x.operation_id,x.actor_identity_id,x.actor_credential_id,NULL,NULL,NULL,x.created_at FROM incoming_contact_rejections x WHERE x.hold_id=?1",[id],|r|Ok(IncomingContactDecision{operation_id:r.get(0)?,actor_identity_id:r.get(1)?,actor_credential_id:r.get(2)?,candidate_id:r.get(3)?,message_id:r.get(4)?,session_id:r.get(5)?,created_at:r.get(6)?})).optional().map_err(|_|Error::storage())?;
    Ok(IncomingContact {
        id: id.into(),
        kind: serde_json::from_value(Value::String(kind)).map_err(|_| Error::storage())?,
        provider,
        reason: serde_json::from_value(Value::String(reason)).map_err(|_| Error::storage())?,
        state: if resolved {
            IncomingContactState::Resolved
        } else if rejected {
            IncomingContactState::Rejected
        } else {
            IncomingContactState::Held
        },
        revision: u64::from(resolved || rejected),
        decision,
        candidates: candidates.into_iter().map(|p| p.view).collect(),
        created_at,
    })
}
pub(crate) fn record(
    c: &Connection,
    actor: &Actor,
    operation: &str,
    kind: &str,
    intent: &str,
    hold: &str,
    store: &Store,
) -> Result<()> {
    crate::operations::reject_control_id(c, actor, operation)?;
    let tenant = tenant(c, &actor.identity_id)?;
    let count: u64 = c
        .query_row(
            "SELECT (SELECT count(*) FROM operations)+(SELECT count(*) FROM task_requests)",
            [],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if count >= store.limits().operation_receipts {
        return Err(Error::new(
            "capacity-exceeded",
            "Incoming decision receipt budget exhausted",
        ));
    }
    c.execute("INSERT INTO operations(record_id,tenant_id,owner_id,operation_id,digest,command_type,intent_json,state,result_json,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,'completed',?8,?9,?9)",params![format!("opr_{}",uuid::Uuid::new_v4()),tenant,actor.identity_id,operation,digest(intent.as_bytes()),kind,intent,encoded(&view(c,actor,&tenant,hold)?)?,crate::now()]).map_err(|_|conflict())?;
    Ok(())
}
/// Caller owns the surrounding transaction and the actual source authority.
/// No message, Session, member or public journal event is created here.
pub(crate) fn capture(c: &Connection, capture: &Capture<'_>, store: &Store) -> Result<String> {
    let runtime = store.require_runtime_owner()?;
    let source = &capture.source;
    let actor = actor_from_connection(c, source.credential_id)?;
    let tenant = tenant(c, &actor.identity_id)?;
    for id in [source.provider, source.account_reference] {
        connect_protocol::transitions::id(id)?;
    }
    if !bounded(capture.provider_id, 256)
        || !bounded(capture.event_id, 256)
        || !bounded(capture.event_digest, 128)
        || capture.candidates.is_empty()
        || capture.candidates.len() > 64
    {
        return Err(invalid());
    }
    let namespace = digest(
        encoded(&(
            source.kind,
            source.provider,
            source.account_reference,
            &tenant,
            &actor.identity_id,
        ))?
        .as_bytes(),
    );
    if let Some((old,hold))=c.query_row("SELECT digest,hold_id FROM incoming_contact_receipts WHERE namespace=?1 AND event_id=?2",params![namespace,capture.event_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional().map_err(|_|Error::storage())? {
        if old != capture.event_digest { return Err(conflict()); }
        return Ok(hold);
    }
    let mut candidates = capture.candidates.clone();
    for candidate in &mut candidates {
        for id in [
            &candidate.view.route_id,
            &candidate.view.conversation_id,
            &candidate.view.endpoint_id,
            &candidate.view.participant_id,
            &candidate.identity_id,
        ] {
            connect_protocol::transitions::id(id)?;
        }
        let member = crate::audience::member(
            c,
            &candidate.identity_id,
            &candidate.view.conversation_id,
            true,
        )?;
        let same:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM conversations WHERE id=?1 AND tenant_id=?2 AND closed=0)",params![candidate.view.conversation_id,tenant],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !same
            || !actor.permits_recipient(&candidate.identity_id)
            || member.participant_id != candidate.view.participant_id
            || member.membership_epoch != candidate.view.membership_epoch
        {
            return Err(Error::forbidden());
        }
        *candidate = candidate.clone().seal()?;
        if encoded(candidate)?.len() > 32768 {
            return Err(invalid());
        }
    }
    candidates.sort_by(|a, b| a.view.id.cmp(&b.view.id));
    if candidates.windows(2).any(|p| p[0].view.id == p[1].view.id) {
        return Err(invalid());
    }
    let payload = encoded(&capture.payload)?;
    let candidates = encoded(&candidates)?;
    let binding = encoded(&source.binding)?;
    if payload.len() > 65536 || candidates.len() > 262144 || binding.len() > 32768 {
        return Err(invalid());
    }
    let content = digest(payload.as_bytes());
    let resource = format!(
        "icr_{}",
        digest(encoded(&(&namespace, capture.provider_id))?.as_bytes())
    );
    let old: Option<String> = c
        .query_row(
            "SELECT payload_digest FROM incoming_contact_resources WHERE id=?1",
            [&resource],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| Error::storage())?;
    let reason = if old.as_ref().is_some_and(|p| p != &content) {
        "resource_conflict"
    } else {
        match capture.reason {
            IncomingContactReason::AmbiguousRoute => "ambiguous_route",
            IncomingContactReason::HistoricalOverlap => "historical_overlap",
            IncomingContactReason::Routed => "routed",
            IncomingContactReason::Ineligible => "ineligible",
            IncomingContactReason::ResourceConflict => return Err(invalid()),
        }
    };
    let hold = format!(
        "ich_{}",
        digest(encoded(&(&resource, &content))?.as_bytes())
    );
    let exists: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM incoming_contact_holds WHERE id=?1)",
            [&hold],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if !exists && reason == "ambiguous_route" && capture.candidates.len() < 2 {
        return Err(invalid());
    }
    let count:u64=c.query_row("SELECT (SELECT count(*) FROM incoming_contact_resources)+(SELECT count(*) FROM incoming_contact_holds)+(SELECT count(*) FROM incoming_contact_receipts)",[],|r|r.get(0)).map_err(|_|Error::storage())?;
    let needed = 1 + u64::from(old.is_none()) + u64::from(!exists);
    if count.saturating_add(needed) > store.limits().operation_receipts {
        return Err(Error::new(
            "capacity-exceeded",
            "Incoming contact custody budget exhausted",
        ));
    }
    if old.is_none() {
        c.execute(
            "INSERT INTO incoming_contact_resources VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![
                resource,
                namespace,
                source.kind.as_str(),
                source.provider,
                tenant,
                actor.identity_id,
                capture.provider_id,
                content,
                binding,
                crate::now()
            ],
        )
        .map_err(|_| Error::storage())?;
    }
    if !exists {
        c.execute("INSERT INTO incoming_contact_holds(id,resource_id,payload_digest,payload_json,candidates_json,reason,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![hold,resource,content,payload,candidates,reason,crate::now()]).map_err(|_|Error::storage())?;
        c.execute("INSERT INTO incoming_contact_audit(hold_id,event_type,actor_identity_id,actor_credential_id,created_at) VALUES(?1,'held',?2,?3,?4)",params![hold,actor.identity_id,source.credential_id,crate::now()]).map_err(|_|Error::storage())?;
    }
    c.execute(
        "INSERT INTO incoming_contact_receipts VALUES(?1,?2,?3,?4)",
        params![namespace, capture.event_id, capture.event_digest, hold],
    )
    .map_err(|_| Error::storage())?;
    if !runtime.current() || actor.expires_at <= crate::now() {
        return Err(Error::unauthorized());
    }
    Ok(hold)
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Position {
    purpose: String,
    owner: String,
    tenant: String,
    scope: String,
    sequence: i64,
}
impl Store {
    pub fn incoming_contact(&self, credential: &str, id: &str) -> Result<IncomingContact> {
        connect_protocol::transitions::id(id)?;
        let owner = self.credential_owner(credential)?;
        let _held = self.acquire_authority(
            &[AuthorityScope::identity(&owner, AuthorityMode::Read)?],
            &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
        )?;
        let c = self.connection()?;
        let (actor, tenant) = admin(&c, credential)?;
        view(&c, &actor, &tenant, id)
    }
    pub fn incoming_contacts(
        &self,
        credential: &str,
        after: Option<&str>,
        limit: u32,
    ) -> Result<IncomingContactPage> {
        if !(1..=32).contains(&limit) {
            return Err(invalid());
        }
        let owner = self.credential_owner(credential)?;
        let _held = self.acquire_authority(
            &[AuthorityScope::identity(&owner, AuthorityMode::Read)?],
            &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
        )?;
        let c = self.connection()?;
        let (actor, tenant) = admin(&c, credential)?;
        let scope = digest(encoded(&actor.scope)?.as_bytes());
        let sequence = if let Some(token) = after {
            let p: Position = crate::cursor::open(&c, token)?;
            if p.purpose != "incoming_contacts"
                || p.owner != actor.identity_id
                || p.tenant != tenant
                || p.scope != scope
                || p.sequence < 0
            {
                return Err(invalid());
            }
            p.sequence
        } else {
            0
        };
        let recipients = encoded(&actor.scope.recipients)?;
        let mut q=c.prepare("SELECT h.sequence,h.id FROM incoming_contact_holds h JOIN incoming_contact_resources r ON r.id=h.resource_id WHERE r.owner_id=?1 AND r.tenant_id=?2 AND h.sequence>?3 AND NOT EXISTS(SELECT 1 FROM json_each(h.candidates_json) p WHERE NOT EXISTS(SELECT 1 FROM json_each(?4) allowed WHERE allowed.value IN(json_extract(p.value,'$.identity_id'),'*'))) ORDER BY h.sequence LIMIT ?5").map_err(|_|Error::storage())?;
        let rows = q
            .query_map(
                params![actor.identity_id, tenant, sequence, recipients, limit + 1],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
            )
            .map_err(|_| Error::storage())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| Error::storage())?;
        let mut contacts = vec![];
        let mut next = None;
        let mut last = sequence;
        let mut bytes = 0;
        for (position, id) in rows {
            let contact = view(&c, &actor, &tenant, &id)?;
            let size = serde_json::to_vec(&contact)
                .map_err(|_| Error::storage())?
                .len();
            if contacts.len() >= limit as usize || bytes + size > connect_protocol::MAX_PAGE_BYTES {
                next = Some(crate::cursor::seal(
                    &c,
                    &Position {
                        purpose: "incoming_contacts".into(),
                        owner: actor.identity_id.clone(),
                        tenant: tenant.clone(),
                        scope: scope.clone(),
                        sequence: last,
                    },
                )?);
                break;
            }
            bytes += size;
            last = position;
            contacts.push(contact);
        }
        Ok(IncomingContactPage { contacts, next })
    }
    /// Explicit rejection never delivers content or modifies a Conversation.
    /// The same logical ID is current-authority replay, not another decision.
    pub fn reject_incoming_contact(
        &self,
        credential: &str,
        operation_id: &str,
        id: &str,
        expected_revision: u64,
        reason: &str,
    ) -> Result<IncomingContact> {
        for key in [operation_id, id] {
            connect_protocol::transitions::id(key)?;
        }
        if expected_revision > 1 || !bounded(reason, 256) {
            return Err(invalid());
        }
        let owner = self.credential_owner(credential)?;
        let _held = self.acquire_authority(
            &[AuthorityScope::identity(&owner, AuthorityMode::Read)?],
            &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let (actor, tenant) = admin(&tx, credential)?;
        let current = view(&tx, &actor, &tenant, id)?;
        crate::operations::reject_control_id(&tx, &actor, operation_id)?;
        let intent = encoded(&(id, expected_revision, reason))?;
        let intent_digest = digest(intent.as_bytes());
        let old:Option<(String,String,bool)>=tx.query_row("SELECT digest,command_type,tombstoned FROM operations WHERE tenant_id=?1 AND owner_id=?2 AND operation_id=?3",params![tenant,actor.identity_id,operation_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_|Error::storage())?;
        if let Some((old, kind, tombstoned)) = old {
            if old != intent_digest || kind != "incoming-contact.reject" {
                return Err(conflict());
            }
            if tombstoned {
                return Err(Error::new("gone", "Incoming decision receipt has expired"));
            }
            return Ok(current);
        }
        if current.state != IncomingContactState::Held || current.revision != expected_revision {
            return Err(conflict());
        }
        let count: u64 = tx
            .query_row(
                "SELECT (SELECT count(*) FROM operations)+(SELECT count(*) FROM task_requests)",
                [],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        if count >= self.limits().operation_receipts {
            return Err(Error::new(
                "capacity-exceeded",
                "Incoming decision receipt budget exhausted",
            ));
        }
        tx.execute(
            "INSERT INTO incoming_contact_rejections VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                id,
                actor.identity_id,
                credential,
                operation_id,
                reason,
                crate::now()
            ],
        )
        .map_err(|_| conflict())?;
        tx.execute("INSERT INTO incoming_contact_audit(hold_id,event_type,actor_identity_id,actor_credential_id,operation_id,created_at) VALUES(?1,'rejected',?2,?3,?4,?5)",params![id,actor.identity_id,credential,operation_id,crate::now()]).map_err(|_|Error::storage())?;
        let result = view(&tx, &actor, &tenant, id)?;
        tx.execute("INSERT INTO operations(record_id,tenant_id,owner_id,operation_id,digest,command_type,intent_json,state,result_json,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,'incoming-contact.reject',?6,'completed',?7,?8,?8)",params![format!("opr_{}",uuid::Uuid::new_v4()),tenant,actor.identity_id,operation_id,intent_digest,intent,encoded(&result)?,crate::now()]).map_err(|_|conflict())?;
        if actor.expires_at <= crate::now() {
            return Err(Error::unauthorized());
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok(result)
    }
}
#[cfg(test)]
mod tests;
