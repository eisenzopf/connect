//! One current-membership and source-audience decision for every projection.
use crate::{Actor, Store};
use connect_protocol::{
    membership::{ContentKind, ContentRef, MemberView},
    Error, Result,
};
use rusqlite::{params, Connection, OptionalExtension};

pub(crate) fn read_scope(kind: ContentKind) -> &'static str {
    match kind {
        ContentKind::Message => "history",
        ContentKind::Artifact => "artifact_get",
        ContentKind::Session => "session_get",
        ContentKind::Call => "call_status",
        ContentKind::SipLeg => "sip_status",
        ContentKind::ContextDelivery => "context_view",
        _ => "record_get",
    }
}

/// Apply the current recipient ceiling to every source of a compound event,
/// before pagination or omitted counts. The caller uses an unaliased view.
pub(crate) fn journal_scope(recipients_parameter: u32) -> String {
    format!("NOT EXISTS(SELECT 1 FROM journal_sources restricted WHERE restricted.sequence=visible_journal.sequence AND restricted.owner_id!=?1 AND NOT EXISTS(SELECT 1 FROM json_each(?{recipients_parameter}) allowed WHERE allowed.value IN(restricted.owner_id,'*'))) AND (json_extract(data_json,'$.receipt.participant_id') IS NULL OR EXISTS(SELECT 1 FROM participants subject WHERE subject.participant_id=json_extract(data_json,'$.receipt.participant_id') AND (subject.identity_id=?1 OR EXISTS(SELECT 1 FROM json_each(?{recipients_parameter}) allowed WHERE allowed.value IN(subject.identity_id,'*')))))")
}

pub(crate) fn member(
    c: &Connection,
    identity: &str,
    conversation: &str,
    active: bool,
) -> Result<MemberView> {
    let row:Option<(String,String,String,u64,bool,Option<String>)>=c.query_row(
        "SELECT p.participant_id,p.kind,p.role,p.membership_epoch,p.active,p.removal_reason FROM participants p JOIN identities i ON i.id=p.identity_id AND i.tenant_id=p.tenant_id JOIN conversations v ON v.id=p.conversation_id AND v.tenant_id=p.tenant_id WHERE p.identity_id=?1 AND p.conversation_id=?2 AND i.revoked=0 AND (?3=0 OR p.active=1)",
        params![identity,conversation,active],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional().map_err(|_|Error::storage())?;
    let (id, kind, role, epoch, active, removal_reason) = row.ok_or_else(Error::forbidden)?;
    Ok(MemberView {
        participant_id: id,
        conversation_id: conversation.into(),
        identity_id: identity.into(),
        kind: serde_json::from_value(serde_json::Value::String(kind))
            .map_err(|_| Error::storage())?,
        role,
        membership_epoch: epoch,
        active,
        removal_reason,
    })
}
pub(crate) fn source_owner(
    c: &Connection,
    conversation: &str,
    kind: ContentKind,
    id: &str,
) -> Result<Option<String>> {
    c.query_row(
        "SELECT owner_id FROM audience_resources WHERE id=?1 AND conversation_id=?2 AND kind=?3",
        params![id, conversation, kind.as_str()],
        |r| r.get(0),
    )
    .optional()
    .map_err(|_| Error::storage())
}

pub(crate) fn visible_to_identity(
    c: &Connection,
    identity: &str,
    conversation: &str,
    kind: ContentKind,
    id: &str,
) -> Result<bool> {
    identity_audience(c, identity, conversation, kind, id, true)
}
// Boolean membership distributes over the audience UNIONs. Probe the exact
// current member's indexed rows instead of building deduplicated temporary
// tables. This is a fresh query, not a cached audience or authority grant.
fn identity_audience(
    c: &Connection,
    identity: &str,
    conversation: &str,
    kind: ContentKind,
    id: &str,
    history: bool,
) -> Result<bool> {
    c.prepare_cached(
        "SELECT EXISTS(
         SELECT 1 FROM participants p
         JOIN identities i ON i.id=p.identity_id AND i.tenant_id=p.tenant_id AND i.revoked=0
         JOIN conversations v ON v.id=p.conversation_id AND v.tenant_id=p.tenant_id
         WHERE p.conversation_id=?3 AND p.identity_id=?4 AND p.active=1 AND (
          EXISTS(SELECT 1 FROM resource_audiences a WHERE a.kind=?1 AND a.id=?2 AND a.conversation_id=?3 AND a.participant_id=p.participant_id AND a.membership_epoch=p.membership_epoch)
          OR (?1='message' AND EXISTS(SELECT 1 FROM message_audiences a WHERE a.message_id=?2 AND a.conversation_id=?3 AND a.participant_id=p.participant_id AND a.membership_epoch=p.membership_epoch))
          OR (?1='artifact' AND EXISTS(SELECT 1 FROM artifact_audiences a WHERE a.artifact_id=?2 AND a.conversation_id=?3 AND a.participant_id=p.participant_id AND a.membership_epoch=p.membership_epoch))
          OR (?5 AND EXISTS(SELECT 1 FROM history_share_items x JOIN history_shares s ON s.id=x.sharing_id AND s.active=1 WHERE x.kind=?1 AND x.source_id=?2 AND x.conversation_id=?3 AND x.participant_id=p.participant_id AND x.membership_epoch=p.membership_epoch))
         ))",
    )
    .and_then(|mut query| {
        query.query_row(params![kind.as_str(), id, conversation, identity, history], |r| r.get(0))
    })
    .map_err(|_| Error::storage())
}

pub(crate) fn visible(
    c: &Connection,
    actor: &Actor,
    conversation: &str,
    kind: ContentKind,
    id: &str,
) -> Result<bool> {
    let Some(owner) = source_owner(c, conversation, kind, id)? else {
        return Ok(false);
    };
    if !actor.permits_recipient(&owner) {
        return Ok(false);
    }
    let same_tenant:bool = c.query_row("SELECT EXISTS(SELECT 1 FROM identities i JOIN conversations v ON v.tenant_id=i.tenant_id WHERE i.id=?1 AND v.id=?2)",params![owner,conversation],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !same_tenant {
        return Ok(false);
    }

    visible_to_identity(c, &actor.identity_id, conversation, kind, id)
}
pub(crate) fn require(
    c: &Connection,
    actor: &Actor,
    conversation: &str,
    kind: ContentKind,
    id: &str,
) -> Result<()> {
    if visible(c, actor, conversation, kind, id)? {
        Ok(())
    } else {
        Err(Error::forbidden())
    }
}

/// A history grant permits reading, never continuation of an old mutation or
/// live presence. New effects require an original current resource epoch.
pub(crate) fn require_original(
    c: &Connection,
    actor: &Actor,
    conversation: &str,
    kind: ContentKind,
    id: &str,
) -> Result<()> {
    require(c, actor, conversation, kind, id)?;
    let original = original_to_identity(c, &actor.identity_id, conversation, kind, id)?;
    if original {
        Ok(())
    } else {
        Err(Error::forbidden())
    }
}

pub(crate) fn original_to_identity(
    c: &Connection,
    identity: &str,
    conversation: &str,
    kind: ContentKind,
    id: &str,
) -> Result<bool> {
    member(c, identity, conversation, true)?;
    identity_audience(c, identity, conversation, kind, id, false)
}
pub(crate) fn source_members(
    c: &Connection,
    conversation: &str,
    kind: ContentKind,
    id: &str,
) -> Result<Vec<String>> {
    let mut st=c.prepare("SELECT DISTINCT p.identity_id FROM content_audiences a JOIN participants p ON p.participant_id=a.participant_id AND p.conversation_id=a.conversation_id AND p.membership_epoch=a.membership_epoch AND p.active=1 JOIN identities i ON i.id=p.identity_id AND i.tenant_id=p.tenant_id AND i.revoked=0 WHERE a.kind=?1 AND a.id=?2 AND a.conversation_id=?3 ORDER BY p.identity_id").map_err(|_|Error::storage())?;
    let rows = st
        .query_map(params![kind.as_str(), id, conversation], |r| r.get(0))
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    Ok(rows)
}
pub(crate) fn event_visible(
    c: &Connection,
    actor: &Actor,
    conversation: &str,
    message: Option<&str>,
    session: Option<&str>,
    artifact: Option<&str>,
    call: Option<&str>,
) -> Result<bool> {
    for (kind, id) in [
        (ContentKind::Message, message),
        (ContentKind::Session, session),
        (ContentKind::Artifact, artifact),
        (ContentKind::Call, call),
    ] {
        if let Some(id) = id {
            if !visible(c, actor, conversation, kind, id)? {
                return Ok(false);
            }
        }
    }
    Ok(true)
}
pub(crate) fn context_kind(kind: &str) -> ContentKind {
    if matches!(kind, "record_ready" | "record_history_shared") {
        ContentKind::Record
    } else if kind.starts_with("artifact_") {
        ContentKind::Artifact
    } else if kind.starts_with("session_") {
        ContentKind::Session
    } else {
        ContentKind::Message
    }
}
pub(crate) fn outcome(
    c: &Connection,
    actor: &Actor,
    value: &connect_protocol::Outcome,
) -> Result<()> {
    use connect_protocol::Outcome;
    match value {
        Outcome::ReceiptAcknowledged(v) => {
            require_original(
                c,
                actor,
                &v.conversation_id,
                ContentKind::Message,
                &v.message_id,
            )?;
            let m = member(c, &actor.identity_id, &v.conversation_id, true)?;
            if m.participant_id != v.participant_id || m.membership_epoch != v.membership_epoch {
                return Err(Error::forbidden());
            };
            Ok(())
        }
        Outcome::ReceiptPreferences(v) => {
            let m = member(c, &actor.identity_id, &v.conversation_id, true)?;
            if m.participant_id != v.participant_id || m.membership_epoch != v.membership_epoch {
                return Err(Error::forbidden());
            };
            Ok(())
        }
        Outcome::Accepted {
            conversation_id,
            message_id,
            ..
        } => require(c, actor, conversation_id, ContentKind::Message, message_id),
        Outcome::ArtifactLink(link) => {
            require(
                c,
                actor,
                &link.conversation_id,
                ContentKind::Artifact,
                &link.artifact_id,
            )?;
            if link.expires_at <= crate::now() {
                return Err(Error::forbidden());
            }
            if let Some(id) = &link.message_id {
                require(c, actor, &link.conversation_id, ContentKind::Message, id)?;
            }
            Ok(())
        }
        Outcome::Artifact(a) => require(c, actor, &a.conversation_id, ContentKind::Artifact, &a.id),
        Outcome::ArtifactChunk(a) => require(
            c,
            actor,
            &a.artifact.conversation_id,
            ContentKind::Artifact,
            &a.artifact.id,
        ),
        Outcome::ConnectionPrepared(v) => require_original(
            c,
            actor,
            &v.conversation_id,
            ContentKind::Session,
            &v.session_id,
        ),
        Outcome::SessionControl(s) => require_original(
            c,
            actor,
            &s.session.conversation_id,
            ContentKind::Session,
            &s.session.id,
        ),
        Outcome::SessionDecision(s) => require_original(
            c,
            actor,
            &s.conversation_id,
            ContentKind::Session,
            &s.session_id,
        ),
        Outcome::Session(s) => require(c, actor, &s.conversation_id, ContentKind::Session, &s.id),
        Outcome::Call(v) => require(c, actor, &v.conversation_id, ContentKind::Call, &v.id),
        Outcome::SipLeg(v) => require(c, actor, &v.conversation_id, ContentKind::SipLeg, &v.id),
        Outcome::Conversation(v) => {
            member(c, &actor.identity_id, &v.id, true)?;
            Ok(())
        }
        Outcome::RequestStatus {
            outcome: Some(v), ..
        } => outcome(c, actor, v),
        _ => Ok(()),
    }
}

impl Store {
    /// Read-admission hook for service and record projections. This returns no
    /// resource metadata; forbidden and unknown resources are indistinguishable.
    pub fn authorize_content(
        &self,
        credential: &str,
        conversation: &str,
        source: &ContentRef,
    ) -> Result<()> {
        let c = self.connection()?;
        let actor = crate::identity::actor_from_connection(&c, credential)?;
        actor.require(read_scope(source.kind))?;
        require(&c, &actor, conversation, source.kind, &source.id)
    }
}

/// Resolve an explicit audience once. Membership labels never grant access.
pub(crate) fn select_members(
    c: &Connection,
    actor: &Actor,
    cid: &str,
    to: &connect_protocol::messages::MessageAudience,
) -> Result<Vec<MemberView>> {
    let mut to = to.clone();
    to.normalize()?;
    let conversation = crate::conversations::authorized_conversation(c, actor, cid)?;
    let mut members=match &to {
        connect_protocol::messages::MessageAudience::All(_)=>conversation.participants.iter().map(|id|member(c,id,cid,true)).collect::<Result<Vec<_>>>()?,
        connect_protocol::messages::MessageAudience::Participants(ids)=>ids.iter().map(|id|{
            let identity:Option<String>=c.query_row("SELECT identity_id FROM participants WHERE participant_id=?1 AND conversation_id=?2",params![id,cid],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
            member(c,&identity.ok_or_else(Error::forbidden)?,cid,true)
        }).collect::<Result<Vec<_>>>()?,
    };
    members.push(member(c, &actor.identity_id, cid, true)?);
    members.sort_by(|a, b| a.participant_id.cmp(&b.participant_id));
    members.dedup_by(|a, b| a.participant_id == b.participant_id);
    if members
        .iter()
        .any(|p| !actor.permits_recipient(&p.identity_id))
    {
        return Err(Error::forbidden());
    }
    Ok(members)
}

#[cfg(test)]
mod tests;
