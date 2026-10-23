//! Historical canonical Session outcomes, read from their authoritative journal.
use super::*;

/// Keep the outcome at its observed revision. Current Session state is not the
/// body of an earlier event; current source authorization is checked separately.
pub(super) fn canonical_session(
    c: &Connection,
    viewer: &str,
    conversation: &str,
    sequence: u64,
) -> Result<(String, String, String)> {
    let (id, revision, raw, owner): (String, u64, String, String) = c
        .query_row(
            "SELECT j.source_id,j.resource_revision,j.data_json,s.initiator_id FROM visible_journal j JOIN sessions s ON s.id=j.source_id AND s.conversation_id=j.conversation_id JOIN session_control sc ON sc.session_id=s.id AND sc.conversation_id=s.conversation_id WHERE j.sequence=?1 AND j.recipient=?2 AND j.conversation_id=?3 AND j.kind='session_control' AND j.source_kind='session'",
            params![sequence, viewer, conversation],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .map_err(|_| Error::forbidden())?;
    let event: serde_json::Value = serde_json::from_str(&raw).map_err(|_| Error::storage())?;
    let session: connect_protocol::session_control::SessionView =
        serde_json::from_value(event["session"].clone()).map_err(|_| Error::storage())?;
    if session.id != id || session.conversation_id != conversation || session.revision != revision {
        return Err(Error::storage());
    }
    Ok((
        id,
        owner,
        serde_json::to_string(&session).map_err(|_| Error::storage())?,
    ))
}

/// Current visibility is independent of the saved metadata at a journal position.
/// Do not replace an upload/invite snapshot with a later ready/end projection.
pub(super) fn saved_metadata(
    c: &Connection,
    viewer: &str,
    conversation: &str,
    sequence: u64,
    kind: connect_protocol::membership::ContentKind,
) -> Result<(String, String, String)> {
    use connect_protocol::membership::ContentKind;
    let column = match kind {
        ContentKind::Session => "session_json",
        ContentKind::Artifact => "artifact_json",
        _ => return Err(Error::forbidden()),
    };
    let (id, raw): (String, String) = c.query_row(
        &format!("SELECT source_id,{column} FROM visible_journal WHERE sequence=?1 AND recipient=?2 AND conversation_id=?3 AND source_kind=?4 AND {column} IS NOT NULL"),
        params![sequence, viewer, conversation, kind.as_str()], |r|Ok((r.get(0)?,r.get(1)?)),
    ).map_err(|_|Error::forbidden())?;
    let (source_id, source_conversation, owner, body) = match kind {
        ContentKind::Session => {
            let saved: connect_protocol::Session =
                serde_json::from_str(&raw).map_err(|_| Error::storage())?;
            crate::sessions::get(c, conversation, &id)?;
            (
                saved.id.clone(),
                saved.conversation_id.clone(),
                saved.initiator_id.clone(),
                serde_json::to_string(&saved).map_err(|_| Error::storage())?,
            )
        }
        ContentKind::Artifact => {
            let saved: connect_protocol::Artifact =
                serde_json::from_str(&raw).map_err(|_| Error::storage())?;
            crate::artifacts::metadata(c, conversation, &id)?;
            (
                saved.id.clone(),
                saved.conversation_id.clone(),
                saved.owner_id.clone(),
                serde_json::to_string(&saved).map_err(|_| Error::storage())?,
            )
        }
        _ => return Err(Error::forbidden()),
    };
    if source_id != id || source_conversation != conversation {
        return Err(Error::storage());
    }
    Ok((id, owner, body))
}
