//! Atomic native message history and frozen audience; no engine work here.
use super::*;
use connect_protocol::{
    membership::ContentKind,
    messages::{BodyEncoding, MessageAudience, MessageDraft},
};
pub(super) fn accept(
    c: &Connection,
    store: &Store,
    actor: &Actor,
    wire: &Intent,
    record: &str,
    source: &crate::NativeStreamProjection,
) -> Result<()> {
    let message = connect_protocol::native_data::message(&wire.payload)?;
    let members = crate::peer_authority::native_data::recipients(c, actor, wire, source)?;
    let draft = MessageDraft {
        to: MessageAudience::Participants(
            members.iter().map(|p| p.participant_id.clone()).collect(),
        ),
        content_type: message.content_type,
        body: wire.payload["body"].as_str().ok_or_else(invalid)?.into(),
        body_encoding: if wire.payload["body_encoding"] == "base64" {
            BodyEncoding::Base64
        } else {
            BodyEncoding::Utf8
        },
        label: message.label,
        attachments: Vec::new(),
        in_reply_to_msg: None,
        msg_id: Some(message.message_id.clone()),
    };
    crate::messages::write_native(
        c,
        actor,
        &wire.conversation_id,
        draft,
        members.clone(),
        store,
    )?;
    c.execute("INSERT INTO native_data_messages(operation_record_id,message_id,source_stream_revision) VALUES(?1,?2,?3)",params![record,message.message_id,source.stream_revision]).map_err(|_|Error::storage())?;
    for member in members {
        c.execute("INSERT INTO native_data_recipients(operation_record_id,participant_id,identity_id,membership_epoch,member_json) VALUES(?1,?2,?3,?4,?5)",params![record,member.participant_id,member.identity_id,member.membership_epoch,serde_json::to_string(&member).map_err(|_|Error::storage())?]).map_err(|_|Error::storage())?;
    }
    Ok(())
}
pub(super) fn read(c: &Connection, actor: &Actor, row: &rows::Saved) -> Result<()> {
    let message: String = c
        .query_row(
            "SELECT message_id FROM native_data_messages WHERE operation_record_id=?1",
            [&row.record.record_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    crate::audience::require_original(
        c,
        actor,
        row.record
            .conversation_id
            .as_deref()
            .ok_or_else(Error::storage)?,
        ContentKind::Message,
        &message,
    )?;
    let mut q = c
        .prepare("SELECT identity_id FROM native_data_recipients WHERE operation_record_id=?1")
        .map_err(|_| Error::storage())?;
    for member in q
        .query_map([&row.record.record_id], |r| r.get::<_, String>(0))
        .map_err(|_| Error::storage())?
    {
        if !actor.permits_recipient(&member.map_err(|_| Error::storage())?) {
            return Err(Error::forbidden());
        }
    }
    Ok(())
}
