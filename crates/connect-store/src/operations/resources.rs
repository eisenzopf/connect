//! Current resource authority is checked before a receipt or effect is used.
use super::invalid;
use crate::{conversations::authorized_conversation, Actor, Store};
use connect_protocol::{Error, Operation, Result};
use rusqlite::Connection;
pub(super) fn authorize(
    c: &Connection,
    actor: &Actor,
    operation: &Operation,
    store: &Store,
) -> Result<Vec<String>> {
    Ok(match operation {
        Operation::Create { participants, .. } => {
            let mut members = participants.clone();
            members.push(actor.identity_id.clone());
            members.sort();
            members.dedup();
            if members.len() > store.limits.participants {
                return Err(invalid("Participant limit exceeded"));
            }
            for id in &members {
                if !actor.permits_recipient(id) {
                    return Err(Error::forbidden());
                }
                let exists: bool = c
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM identities WHERE id=?1)",
                        [id],
                        |r| r.get(0),
                    )
                    .map_err(|_| Error::storage())?;
                if !exists {
                    return Err(Error::forbidden());
                }
            }
            members
        }
        Operation::Send {
            conversation_id, ..
        }
        | Operation::CallStart {
            conversation_id, ..
        }
        | Operation::CallEnd {
            conversation_id, ..
        }
        | Operation::SessionAccept {
            conversation_id, ..
        }
        | Operation::SessionReject {
            conversation_id, ..
        }
        | Operation::SessionCancel {
            conversation_id, ..
        }
        | Operation::SessionEnd {
            conversation_id, ..
        }
        | Operation::SipDial {
            conversation_id, ..
        }
        | Operation::SipEnd {
            conversation_id, ..
        }
        | Operation::ArtifactAppend {
            conversation_id, ..
        }
        | Operation::ArtifactFinish {
            conversation_id, ..
        }
        | Operation::ArtifactDelete {
            conversation_id, ..
        }
        | Operation::Close {
            conversation_id, ..
        } => authorized_conversation(c, actor, conversation_id)?.participants,
        Operation::SessionInvite {
            conversation_id, ..
        }
        | Operation::ArtifactBegin {
            conversation_id, ..
        } => {
            let conversation = authorized_conversation(c, actor, conversation_id)?;
            if conversation.closed {
                return Err(Error::new(
                    "conversation-closed",
                    "Conversation no longer accepts new sessions or artifacts",
                ));
            }
            conversation.participants
        }
        Operation::AvailabilitySet { .. } => vec![actor.identity_id.clone()],
        _ => return Err(invalid("Unsupported operation")),
    })
}
