//! One current conversation model for canonical dispatch and ordinary facades.
use crate::{
    conversations::{authorized_conversation, capacity, insert_event},
    identity::actor_from_connection,
    Actor, AuthorityGuardSet, AuthorityMode, AuthorityScope, AuthorityWait, PeerOperationClaim,
    Store,
};
use connect_protocol::{conversations::*, transitions::OperationAction, Error, Operation, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
const MAX_PAGE_BYTES: usize = 384 * 1024;
fn invalid() -> Error {
    Error::new("invalid-request", "Invalid conversation request")
}
fn conflict() -> Error {
    Error::new("conflict", "Conversation state or revision conflicts")
}
pub(crate) fn normalize_create(
    c: &Connection,
    actor: &Actor,
    request: &mut CreateRequest,
) -> Result<()> {
    request.normalize()?;
    let tenant: String = c
        .query_row(
            "SELECT tenant_id FROM identities WHERE id=?1 AND revoked=0",
            [&actor.identity_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| Error::storage())?
        .ok_or_else(Error::forbidden)?;
    if request.tenant_id.as_ref().is_some_and(|id| id != &tenant) {
        return Err(Error::forbidden());
    }
    request.tenant_id = Some(tenant);
    if !request
        .initial_participants
        .iter()
        .any(|p| p.identity_id == actor.identity_id)
    {
        request.initial_participants.push(InitialParticipant {
            identity_id: actor.identity_id.clone(),
            role: "participant".into(),
        });
    }
    request.initial_participants.sort();
    Ok(())
}
pub(crate) fn create_resources(
    c: &Connection,
    actor: &Actor,
    request: &CreateRequest,
    store: &Store,
) -> Result<Vec<String>> {
    let mut request = request.clone();
    normalize_create(c, actor, &mut request)?;
    if request.initial_participants.len() > store.limits.participants {
        return Err(invalid());
    }
    let mut ids = vec![];
    for p in request.initial_participants {
        let exists:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM identities WHERE id=?1 AND tenant_id=?2 AND revoked=0)",params![p.identity_id,request.tenant_id],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !exists || !actor.permits_recipient(&p.identity_id) {
            return Err(Error::forbidden());
        }
        ids.push(p.identity_id);
    }
    Ok(ids)
}
/// Returns a server-selected result ID plus all current/frozen source scopes.
/// A concurrent retry may resolve a different candidate to the committed ID.
pub(crate) fn create_plan(
    c: &Connection,
    actor: &Actor,
    id: &str,
    request: &CreateRequest,
    candidate: &str,
    store: &Store,
) -> Result<(String, Vec<AuthorityScope>)> {
    actor.require("create")?;
    connect_protocol::transitions::id(id)?;
    connect_protocol::transitions::id(candidate)?;
    let resources = create_resources(c, actor, request, store)?;
    let tenant: String = c
        .query_row(
            "SELECT tenant_id FROM identities WHERE id=?1",
            [&actor.identity_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let saved:Option<Option<String>>=c.query_row("SELECT conversation_id FROM operations WHERE owner_id=?1 AND tenant_id=?2 AND operation_id=?3",params![actor.identity_id,tenant,id],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
    let target = saved
        .as_ref()
        .and_then(Clone::clone)
        .unwrap_or_else(|| candidate.into());
    let mut scopes = vec![
        AuthorityScope::identity(&actor.identity_id, AuthorityMode::Write)?,
        AuthorityScope::conversation(&target, AuthorityMode::Write)?,
    ];
    for id in resources {
        scopes.push(AuthorityScope::identity(&id, AuthorityMode::Read)?);
    }
    if saved.is_some() {
        scopes.extend(crate::transitions::operation_source_scopes(
            c,
            actor,
            &PeerOperationClaim {
                action: OperationAction::Get {
                    operation_id: id.into(),
                },
                conversation_id: None,
                session_id: None,
                connection_id: None,
            },
            false,
        )?);
    } else if c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM conversations WHERE id=?1)",
            [&target],
            |r| r.get::<_, bool>(0),
        )
        .map_err(|_| Error::storage())?
    {
        return Err(Error::forbidden());
    }
    Ok((target, AuthorityScope::merged(&scopes)?))
}
pub(crate) fn view(c: &Connection, actor: &Actor, cid: &str) -> Result<Conversation> {
    authorized_conversation(c, actor, cid)?;
    let row=c.query_row("SELECT tenant_id,policy,idle_close_secs,metadata_json,opened_at,revision,closed,closed_at,reason_code,reason FROM conversations WHERE id=?1",[cid],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<u64>>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,i64>(4)?,r.get::<_,u64>(5)?,r.get::<_,bool>(6)?,r.get::<_,Option<i64>>(7)?,r.get::<_,Option<u64>>(8)?,r.get::<_,Option<String>>(9)?))).map_err(|_|Error::storage())?;
    let (tenant, policy, idle, metadata, opened, revision, closed, closed_at, reason_code, reason) =
        row;
    let mut statement=c.prepare("SELECT p.participant_id,p.identity_id,i.kind,p.role,i.display_name FROM participants p JOIN identities i ON i.id=p.identity_id AND i.tenant_id=p.tenant_id WHERE p.conversation_id=?1 AND p.active=1 AND i.revoked=0 ORDER BY p.participant_id LIMIT 129").map_err(|_|Error::storage())?;
    let mut participants = vec![];
    let mut count = 0;
    for row in statement
        .query_map([cid], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        })
        .map_err(|_| Error::storage())?
    {
        let (pid, id, kind, role, display) = row.map_err(|_| Error::storage())?;
        count += 1;
        if count > 128 {
            return Err(Error::new(
                "capacity-exceeded",
                "Conversation participant bound exceeded",
            ));
        }
        if actor.permits_recipient(&id) {
            participants.push(Participant {
                participant_id: pid,
                identity_id: id,
                kind: serde_json::from_value(serde_json::json!(kind))
                    .map_err(|_| Error::storage())?,
                role,
                display_name: display,
            });
        }
    }
    Ok(Conversation {
        id: cid.into(),
        tenant_id: tenant,
        policy: serde_json::from_value(serde_json::json!(policy)).map_err(|_| Error::storage())?,
        idle_close_secs: idle,
        metadata: metadata
            .map(|m| serde_json::from_str(&m).map_err(|_| Error::storage()))
            .transpose()?,
        participants,
        opened_at: opened,
        revision,
        closed,
        closed_at,
        reason_code,
        reason,
        receipt: None,
    })
}
pub(crate) fn create(
    c: &Connection,
    actor: &Actor,
    id: &str,
    request: &CreateRequest,
    resources: &[String],
    store: &Store,
) -> Result<Conversation> {
    let mut request = request.clone();
    normalize_create(c, actor, &mut request)?;
    if !capacity(c, resources.len() as u64, 0, store)? {
        return Err(Error::new(
            "capacity-exceeded",
            "Durable event capacity exceeded",
        ));
    }
    c.execute("INSERT INTO conversations(id,tenant_id,policy,idle_close_secs,metadata_json,opened_at) VALUES(?1,?2,?3,?4,?5,?6)",params![id,request.tenant_id,request.policy.as_str(),request.idle_close_secs,request.metadata.as_ref().map(crate::provisioning::canonical_json).transpose()?,crate::now()]).map_err(|_|Error::storage())?;
    for p in request.initial_participants {
        if !resources.contains(&p.identity_id) {
            return Err(Error::forbidden());
        }
        c.execute("INSERT INTO participants(conversation_id,identity_id,tenant_id,kind,role) SELECT ?1,id,tenant_id,kind,?3 FROM identities WHERE id=?2",params![id,p.identity_id,p.role]).map_err(|_|Error::storage())?;
        insert_event(c, &p.identity_id, id, "conversation_created", None)?;
    }
    view(c, actor, id)
}
pub(crate) fn close(
    c: &Connection,
    actor: &Actor,
    cid: &str,
    options: &CloseOptions,
    store: &Store,
) -> Result<Conversation> {
    options.validate()?;
    let existing = view(c, actor, cid)?;
    if options
        .expected_revision
        .is_some_and(|v| v != existing.revision)
    {
        return Err(conflict());
    }
    if existing.closed {
        if options
            .reason_code
            .is_some_and(|v| Some(v) != existing.reason_code)
            || options
                .reason
                .as_ref()
                .is_some_and(|v| Some(v) != existing.reason.as_ref())
        {
            return Err(conflict());
        }
        return Ok(existing);
    }
    crate::sessions::ensure_conversation_closable(c, cid)?;
    let recipients = authorized_conversation(c, actor, cid)?.participants;
    if !capacity(c, recipients.len() as u64, 0, store)? {
        return Err(Error::new(
            "capacity-exceeded",
            "Durable event capacity exceeded",
        ));
    }
    c.execute(
        "UPDATE conversations SET closed=1,reason_code=?2,reason=?3 WHERE id=?1",
        params![
            cid,
            options.reason_code.unwrap_or(0),
            options.reason.as_deref().unwrap_or("explicit closure")
        ],
    )
    .map_err(|_| Error::storage())?;
    for id in recipients {
        insert_event(c, &id, cid, "conversation_closed", None)?;
    }
    view(c, actor, cid)
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Position {
    purpose: String,
    identity: String,
    credential: String,
    tenant: String,
    scope: String,
    filter: String,
    last: i64,
    high: i64,
    expires: i64,
}
fn hashes(actor: &Actor, query: &ListQuery) -> Result<(String, String)> {
    let mut ops = actor.scope.operations.clone();
    ops.sort();
    ops.dedup();
    let mut recipients = actor.scope.recipients.clone();
    recipients.sort();
    recipients.dedup();
    Ok((
        hex::encode(Sha256::digest(
            serde_jcs::to_vec(&(ops, recipients)).map_err(|_| Error::storage())?,
        )),
        hex::encode(Sha256::digest(
            serde_jcs::to_vec(&query.filter).map_err(|_| Error::storage())?,
        )),
    ))
}
fn authorize_filter(c: &Connection, actor: &Actor, query: &ListQuery) -> Result<String> {
    query.validate()?;
    actor.require("conversation.list")?;
    let tenant: String = c
        .query_row(
            "SELECT tenant_id FROM identities WHERE id=?1 AND revoked=0",
            [&actor.identity_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if query
        .filter
        .tenant_id
        .as_ref()
        .is_some_and(|t| t != &tenant)
    {
        return Err(Error::forbidden());
    }
    if let Some(id) = &query.filter.identity_id {
        let live:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM identities WHERE id=?1 AND tenant_id=?2 AND revoked=0)",params![id,tenant],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !live || !actor.permits_recipient(id) {
            return Err(Error::forbidden());
        }
    }
    if let Some(pid) = &query.filter.participant_id {
        let id:Option<String>=c.query_row("SELECT p.identity_id FROM participants p JOIN identities i ON i.id=p.identity_id AND i.tenant_id=p.tenant_id WHERE p.participant_id=?1 AND p.tenant_id=?2 AND p.active=1 AND i.revoked=0",params![pid,tenant],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
        if !id.as_ref().is_some_and(|id| actor.permits_recipient(id)) {
            return Err(Error::forbidden());
        }
    }
    Ok(tenant)
}
pub(crate) fn list_plan(
    c: &Connection,
    actor: &Actor,
    query: &ListQuery,
    store: &Store,
) -> Result<(ListPage, Vec<AuthorityScope>)> {
    let tenant = authorize_filter(c, actor, query)?;
    let (scope, filter) = hashes(actor, query)?;
    let (last, high, expires) = if let Some(cursor) = &query.cursor {
        let p: Position = crate::cursor::open(c, cursor)?;
        if p.purpose != "conversation-list/1"
            || p.identity != actor.identity_id
            || p.credential != actor.credential_id
            || p.tenant != tenant
            || p.scope != scope
            || p.filter != filter
            || p.expires <= crate::now()
            || p.last < 0
            || p.last > p.high
        {
            return Err(Error::new(
                "invalid-cursor",
                "Conversation cursor is no longer current",
            ));
        }
        (p.last, p.high, p.expires)
    } else {
        (
            0,
            c.query_row(
                "SELECT COALESCE(MAX(rowid),0) FROM conversations",
                [],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?,
            (crate::now() + 30).min(actor.expires_at),
        )
    };
    let limit = query.limit.unwrap_or(50).min(store.limits.page_size);
    if limit == 0 {
        return Err(invalid());
    }
    let since = query
        .filter
        .since
        .map(|v| v.timestamp() + i64::from(v.timestamp_subsec_nanos() > 0));
    let until = query.filter.until.map(|v| v.timestamp());
    let mut statement=c.prepare("SELECT v.rowid,v.id FROM conversations v JOIN participants own ON own.conversation_id=v.id AND own.identity_id=?1 AND own.active=1 WHERE v.tenant_id=?2 AND v.rowid>?3 AND v.rowid<=?4 AND (?5='all' OR (?5='closed')=v.closed) AND (?6 IS NULL OR v.opened_at>=?6) AND (?7 IS NULL OR v.opened_at<=?7) AND (?8 IS NULL OR EXISTS(SELECT 1 FROM participants p JOIN identities i ON i.id=p.identity_id WHERE p.conversation_id=v.id AND p.identity_id=?8 AND p.active=1 AND i.revoked=0)) AND (?9 IS NULL OR EXISTS(SELECT 1 FROM participants p WHERE p.conversation_id=v.id AND p.participant_id=?9 AND p.active=1)) ORDER BY v.rowid LIMIT ?10").map_err(|_|Error::storage())?;
    let state = match query.filter.state {
        ConversationState::Open => "open",
        ConversationState::Closed => "closed",
        ConversationState::All => "all",
    };
    let rows = statement
        .query_map(
            params![
                actor.identity_id,
                tenant,
                last,
                high,
                state,
                since,
                until,
                query.filter.identity_id,
                query.filter.participant_id,
                limit + 1
            ],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
        )
        .map_err(|_| Error::storage())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| Error::storage())?;
    let mut scopes = vec![AuthorityScope::identity(
        &actor.identity_id,
        AuthorityMode::Read,
    )?];
    let mut conversations = vec![];
    let mut bytes = 0;
    let mut frames = 0;
    let mut position = last;
    let mut more = false;
    for (seq, id) in rows {
        let value = view(c, actor, &id)?;
        let size = serde_json::to_vec(&value)
            .map_err(|_| Error::storage())?
            .len();
        let next_scopes =
            crate::authority_admission::conversation_scopes(c, &id, AuthorityMode::Read)?;
        let mut merged = scopes.clone();
        merged.extend(next_scopes);
        let merged = AuthorityScope::merged(&merged);
        let cost = if value.closed { 2 } else { 1 };
        if conversations.len() >= limit as usize
            || bytes + size > MAX_PAGE_BYTES
            || frames + cost > 126
            || merged.is_err()
        {
            more = true;
            break;
        }
        scopes = merged?;
        bytes += size;
        frames += cost;
        position = seq;
        conversations.push(value);
    }
    if more && conversations.is_empty() {
        return Err(Error::new(
            "capacity-exceeded",
            "Conversation page cannot fit its bounded authority or response budget",
        ));
    }
    let next_cursor = more
        .then(|| {
            crate::cursor::seal(
                c,
                &Position {
                    purpose: "conversation-list/1".into(),
                    identity: actor.identity_id.clone(),
                    credential: actor.credential_id.clone(),
                    tenant,
                    scope,
                    filter,
                    last: position,
                    high,
                    expires,
                },
            )
        })
        .transpose()?;
    Ok((
        ListPage {
            conversations,
            next_cursor,
        },
        scopes,
    ))
}
impl Store {
    pub(crate) fn create_authority(
        &self,
        credential: &str,
        id: &str,
        request: &CreateRequest,
        held: Option<&Arc<AuthorityGuardSet>>,
    ) -> Result<(String, Arc<AuthorityGuardSet>)> {
        let candidate = format!("conv_{}", uuid::Uuid::new_v4().simple());
        let guard = self.stable_authority_with(
            held,
            &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                let actor = actor_from_connection(&c, credential)?;
                Ok(create_plan(&c, &actor, id, request, &candidate, self)?.1)
            },
        )?;
        let c = self.connection()?;
        let actor = actor_from_connection(&c, credential)?;
        let (id, scopes) = create_plan(&c, &actor, id, request, &candidate, self)?;
        if !self.authority_covers(&guard, &scopes) {
            return Err(Error::forbidden());
        }
        Ok((id, guard))
    }
    pub fn list_conversations(&self, credential: &str, query: ListQuery) -> Result<ListPage> {
        self.list_conversations_held(credential, query, None, None)
    }
    pub(crate) fn list_conversations_held(
        &self,
        credential: &str,
        query: ListQuery,
        held: Option<&Arc<AuthorityGuardSet>>,
        peer: Option<&crate::PeerAuthorityGuard>,
    ) -> Result<ListPage> {
        let guard = self.stable_authority_with(
            held,
            &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                let actor = actor_from_connection(&c, credential)?;
                Ok(list_plan(&c, &actor, &query, self)?.1)
            },
        )?;
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        let actor = actor_from_connection(&tx, credential)?;
        if let Some(peer) = peer {
            self.require_peer_conversation(
                &tx,
                peer,
                credential,
                &Operation::Conversations {
                    query: query.clone(),
                },
            )?;
        }
        let (page, scopes) = list_plan(&tx, &actor, &query, self)?;
        if !self.authority_covers(&guard, &scopes) {
            return Err(Error::forbidden());
        }
        actor_from_connection(&tx, credential)?;
        if let Some(peer) = peer {
            self.require_peer_source(&tx, peer, credential)?;
        }
        Ok(page)
    }
    /// Trusted bounded local policy maintenance, never a body-selected actor.
    pub fn expire_idle_conversations(&self, limit: u32) -> Result<u32> {
        self.require_runtime_owner()?;
        if limit == 0 || limit > 32 {
            return Err(invalid());
        }
        let mut closed = 0;
        for _ in 0..limit {
            let selected = {
                let c = self.connection()?;
                eligible(&c)?
            };
            let Some(cid) = selected else {
                break;
            };
            let guard = self.runtime_conversation_authority(|c| {
                Ok(if eligible(c)?.as_deref() == Some(&cid) {
                    vec![cid.clone()]
                } else {
                    vec![]
                })
            })?;
            let mut c = self.connection()?;
            let tx = c
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|_| Error::storage())?;
            self.require_runtime_owner()?;
            if eligible(&tx)?.as_deref() != Some(&cid) {
                continue;
            }
            let scopes =
                crate::authority_admission::conversation_scopes(&tx, &cid, AuthorityMode::Write)?;
            if !self.authority_covers(&guard, &scopes) {
                continue;
            }
            crate::sessions::ensure_conversation_closable(&tx, &cid)?;
            let mut recipients=tx.prepare("SELECT identity_id FROM participants WHERE conversation_id=?1 AND active=1 ORDER BY identity_id").map_err(|_|Error::storage())?;
            let ids = recipients
                .query_map([&cid], |r| r.get::<_, String>(0))
                .map_err(|_| Error::storage())?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|_| Error::storage())?;
            drop(recipients);
            if !capacity(&tx, ids.len() as u64, 0, self)? {
                return Err(Error::new(
                    "capacity-exceeded",
                    "Conversation policy journal capacity exceeded",
                ));
            }
            tx.execute(
                "UPDATE conversations SET closed=1,reason_code=0,reason='idle timeout' WHERE id=?1",
                [&cid],
            )
            .map_err(|_| Error::storage())?;
            for id in ids {
                insert_event(&tx, &id, &cid, "conversation_closed", None)?;
            }
            self.require_runtime_owner()?;
            tx.commit().map_err(|_| Error::storage())?;
            closed += 1;
        }
        Ok(closed)
    }
}
fn eligible(c: &Connection) -> Result<Option<String>> {
    c.query_row("SELECT v.id FROM conversations v WHERE v.closed=0 AND v.policy='ephemeral' AND v.idle_close_secs IS NOT NULL AND MAX(v.opened_at,COALESCE((SELECT MAX(accepted_at) FROM messages WHERE conversation_id=v.id),v.opened_at),COALESCE((SELECT MAX(updated_at) FROM sessions WHERE conversation_id=v.id),v.opened_at))<=?1-v.idle_close_secs AND NOT EXISTS(SELECT 1 FROM sessions WHERE conversation_id=v.id AND state NOT IN ('ended','rejected','cancelled','failed')) AND NOT EXISTS(SELECT 1 FROM voice_calls WHERE conversation_id=v.id AND state NOT IN ('ended','cancelled','failed','forbidden')) AND NOT EXISTS(SELECT 1 FROM sip_legs WHERE conversation_id=v.id AND state NOT IN ('ended','cancelled','failed','forbidden')) AND NOT EXISTS(SELECT 1 FROM operations WHERE conversation_id=v.id AND ledger_version=1 AND state NOT IN ('completed','cancelled','failed')) ORDER BY v.opened_at,v.id LIMIT 1",[crate::now()],|r|r.get(0)).optional().map_err(|_|Error::storage())
}
