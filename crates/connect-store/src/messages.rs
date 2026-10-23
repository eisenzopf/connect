//! A single current-authority writer for addressed messages and provider speech.
use crate::{
    audience,
    conversations::{authorized_conversation, capacity, insert_event},
    Actor, Store,
};
use connect_protocol::{
    membership::{ContentKind, MemberView},
    messages::*,
    Error, Message, Result,
};
use rusqlite::{params, Connection, OptionalExtension};

pub(crate) fn recipients(
    c: &Connection,
    actor: &Actor,
    cid: &str,
    draft: &MessageDraft,
) -> Result<Vec<MemberView>> {
    let mut normalized = draft.clone();
    normalized.normalize()?;
    let draft = &normalized;
    actor.require("send")?;
    let members = audience::select_members(c, actor, cid, &draft.to)?;
    if let Some(parent) = &draft.in_reply_to_msg {
        audience::require(c, actor, cid, ContentKind::Message, parent)?;
        for member in &members {
            if !audience::visible_to_identity(
                c,
                &member.identity_id,
                cid,
                ContentKind::Message,
                parent,
            )? {
                return Err(Error::forbidden());
            }
        }
    }
    for attachment in &draft.attachments {
        actor.require("artifact_get")?;
        audience::require(c, actor, cid, ContentKind::Artifact, &attachment.id)?;
        let artifact = crate::artifacts::metadata(c, cid, &attachment.id)?;
        if artifact.state != "ready"
            || artifact.expires_at.is_some_and(|e| e <= crate::now())
            || artifact.media_type != attachment.content_type
            || artifact.size != attachment.size_bytes
        {
            return Err(Error::forbidden());
        }
        for member in &members {
            if !audience::visible_to_identity(
                c,
                &member.identity_id,
                cid,
                ContentKind::Artifact,
                &attachment.id,
            )? {
                return Err(Error::forbidden());
            }
        }
    }
    Ok(members)
}

pub(crate) fn write(
    c: &Connection,
    actor: &Actor,
    cid: &str,
    mut draft: MessageDraft,
    store: &Store,
) -> Result<String> {
    draft.normalize()?;
    let members = recipients(c, actor, cid, &draft)?;
    write_members(c, actor, cid, draft, members, store, true)
}
/// Native admission freezes its accepted Session audience before this call.
/// It shares the message journal and ID namespace, while its selected native
/// route must never enqueue ambient conversation providers.
pub(crate) fn write_native(
    c: &Connection,
    actor: &Actor,
    cid: &str,
    draft: MessageDraft,
    mut members: Vec<MemberView>,
    store: &Store,
) -> Result<String> {
    members.push(audience::member(c, &actor.identity_id, cid, true)?);
    members.sort_by(|a, b| a.participant_id.cmp(&b.participant_id));
    members.dedup_by(|a, b| a.participant_id == b.participant_id);
    write_members(c, actor, cid, draft, members, store, false)
}
fn write_members(
    c: &Connection,
    actor: &Actor,
    cid: &str,
    draft: MessageDraft,
    members: Vec<MemberView>,
    store: &Store,
    providers: bool,
) -> Result<String> {
    if providers && draft.body.is_empty() && draft.attachments.is_empty() {
        return Err(Error::new("invalid-request", "Message requires content"));
    }
    // Bound stored/wire bytes before decoding binary content.
    if draft.body.len()
        > store
            .limits
            .message_bytes
            .saturating_mul(4)
            .saturating_div(3)
            .saturating_add(4)
        || draft.bytes()?.len() > store.limits.message_bytes
    {
        return Err(Error::new(
            "invalid-request",
            "Message exceeds its byte limit",
        ));
    }
    let conversation = authorized_conversation(c, actor, cid)?;
    if conversation.closed {
        return Err(Error::new(
            "conversation-closed",
            "Conversation no longer accepts messages",
        ));
    }
    if !capacity(c, members.len() as u64, draft.body.len() as u64, store)? {
        return Err(Error::new(
            "capacity-exceeded",
            "Durable message or event capacity exceeded",
        ));
    }
    let id = draft
        .msg_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let sender = audience::member(c, &actor.identity_id, cid, true)?;
    c.execute("INSERT INTO messages(id,conversation_id,sender,body,audience_mode,sender_participant_id,content_type,body_encoding,label,in_reply_to_msg_id,accepted_at) VALUES(?1,?2,?3,?4,'explicit',?5,?6,?7,?8,?9,?10)",params![id,cid,actor.identity_id,draft.body,sender.participant_id,draft.content_type,draft.body_encoding.as_str(),draft.label,draft.in_reply_to_msg,crate::now()]).map_err(|e|if matches!(e,rusqlite::Error::SqliteFailure(ref v,_) if v.code==rusqlite::ErrorCode::ConstraintViolation){Error::new("conflict","Message identity is unavailable")}else{Error::storage()})?;
    for member in &members {
        c.execute(
            "INSERT INTO message_audiences VALUES(?1,?2,?3,?4,?5)",
            params![
                id,
                cid,
                member.participant_id,
                member.membership_epoch,
                if member.identity_id == actor.identity_id {
                    "sender"
                } else {
                    "recipient"
                }
            ],
        )
        .map_err(|_| Error::storage())?;
        insert_event(c, &member.identity_id, cid, "message", Some(&id))?;
    }
    for attachment in &draft.attachments {
        c.execute(
            "INSERT INTO message_attachments VALUES(?1,?2,?3,?4,?5)",
            params![
                id,
                attachment.id,
                cid,
                attachment.content_type,
                attachment.size_bytes
            ],
        )
        .map_err(|_| Error::storage())?;
    }
    if providers {
        crate::providers::enqueue(c, &id, cid, &actor.identity_id, &actor.credential_id, store)?;
    }
    crate::identity::actor_from_connection(c, &actor.credential_id)?;
    Ok(id)
}

pub(crate) fn get(c: &Connection, cid: &str, id: &str) -> Result<Message> {
    let mut message=c.query_row("SELECT id,conversation_id,sender,body,sender_participant_id,content_type,body_encoding,label,in_reply_to_msg_id,accepted_at FROM messages WHERE id=?1 AND conversation_id=?2",params![id,cid],|r|Ok(Message{id:r.get(0)?,conversation_id:r.get(1)?,sender:r.get(2)?,body:r.get(3)?,sender_participant_id:r.get(4)?,content_type:r.get(5)?,body_encoding:if r.get::<_,String>(6)?=="base64"{BodyEncoding::Base64}else{BodyEncoding::Utf8},label:r.get(7)?,in_reply_to_msg:r.get(8)?,accepted_at:r.get(9)?,audience:vec![],attachments:vec![]})).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)?;
    let mut st=c.prepare("SELECT participant_id,membership_epoch FROM message_audiences WHERE message_id=?1 ORDER BY participant_id").map_err(|_|Error::storage())?;
    message.audience = st
        .query_map([id], |r| {
            Ok(MessageRecipient {
                participant_id: r.get(0)?,
                membership_epoch: r.get(1)?,
            })
        })
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    let mut st=c.prepare("SELECT artifact_id,content_type,size_bytes FROM message_attachments WHERE message_id=?1 ORDER BY artifact_id").map_err(|_|Error::storage())?;
    message.attachments = st
        .query_map([id], |r| {
            Ok(AttachmentRef {
                id: r.get(0)?,
                content_type: r.get(1)?,
                size_bytes: r.get(2)?,
            })
        })
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    Ok(message)
}
pub(crate) fn project(
    c: &Connection,
    actor: &Actor,
    cid: &str,
    id: &str,
    peer: Option<&str>,
    delegate: Option<&Actor>,
) -> Result<Message> {
    audience::require(c, actor, cid, ContentKind::Message, id)?;
    let mut message = get(c, cid, id)?;
    let allowed = |kind, id: &str| -> Result<bool> {
        Ok(audience::visible(c, actor, cid, kind, id)?
            && match peer {
                Some(peer) => audience::visible_to_identity(c, peer, cid, kind, id)?,
                None => true,
            }
            && match delegate {
                Some(d) => audience::visible(c, d, cid, kind, id)?,
                None => true,
            })
    };
    if let Some(parent) = &message.in_reply_to_msg {
        if !allowed(ContentKind::Message, parent)? {
            message.in_reply_to_msg = None;
        }
    }
    let mut attachments = vec![];
    for a in message.attachments {
        if allowed(ContentKind::Artifact, &a.id)? {
            attachments.push(a);
        }
    }
    message.attachments = attachments;
    Ok(message)
}
pub(crate) fn context_body(message: &Message) -> Result<String> {
    if message.body_encoding == BodyEncoding::Utf8
        && message.content_type == "text/plain"
        && message.attachments.is_empty()
        && message.in_reply_to_msg.is_none()
    {
        Ok(message.body.clone())
    } else {
        serde_json::to_string(message).map_err(|_| Error::storage())
    }
}

impl Store {
    /// Canonical sender claims are checked under the same transaction as acceptance.
    pub fn send_message_as(
        &self,
        credential: &str,
        operation_id: &str,
        cid: &str,
        from: Option<&str>,
        message: MessageDraft,
    ) -> Result<connect_protocol::Outcome> {
        self.send_message_as_with_peer(credential, operation_id, cid, from, message, None)
    }
    pub fn send_message_as_with_peer(
        &self,
        credential: &str,
        operation_id: &str,
        cid: &str,
        from: Option<&str>,
        message: MessageDraft,
        peer: Option<&crate::PeerAuthorityGuard>,
    ) -> Result<connect_protocol::Outcome> {
        let mut connection = self.connection()?;
        let tx = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        if let Some(peer) = peer {
            self.require_peer_authority(&tx, peer, credential, cid, "send")?;
        }
        let actor = crate::identity::actor_from_connection(&tx, credential)?;
        let sender = audience::member(&tx, &actor.identity_id, cid, true)?;
        if from.is_some_and(|id| id != sender.participant_id) {
            return Err(Error::forbidden());
        }
        let operation = connect_protocol::Operation::Send {
            request_id: operation_id.into(),
            conversation_id: cid.into(),
            message,
        };
        let result = crate::operations::execute(
            &tx,
            &actor,
            &operation,
            &crate::DiscoveryRuntime::default(),
            self,
            None,
        )?
        .outcome;
        if let Some(peer) = peer {
            self.require_peer_authority(&tx, peer, credential, cid, "send")?;
        }
        tx.commit().map_err(|_| Error::storage())?;
        result
    }
    pub fn message_history(
        &self,
        credential: &str,
        cid: &str,
        query: &HistoryQuery,
    ) -> Result<connect_protocol::Outcome> {
        self.message_history_with_peer(credential, cid, query, None)
    }
    pub fn message_history_with_peer(
        &self,
        credential: &str,
        cid: &str,
        query: &HistoryQuery,
        peer: Option<&crate::PeerAuthorityGuard>,
    ) -> Result<connect_protocol::Outcome> {
        let mut connection = self.connection()?;
        let tx = connection.transaction().map_err(|_| Error::storage())?;
        if let Some(peer) = peer {
            self.require_peer_authority(&tx, peer, credential, cid, "history")?;
        }
        let actor = crate::identity::actor_from_connection(&tx, credential)?;
        actor.require("history")?;
        let result = crate::conversation_queries::history_query(
            &tx,
            &actor,
            cid,
            query,
            self.limits.page_size,
        );
        if let Some(peer) = peer {
            self.require_peer_authority(&tx, peer, credential, cid, "history")?;
        }
        result
    }
}
