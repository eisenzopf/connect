//! Existing mutation implementations; network dispatch remains outside SQLite.
use super::invalid;
use crate::conversations::{authorized_conversation, capacity, insert_event};
use crate::{Actor, Store};
use connect_protocol::{Conversation, Error, Operation, Outcome, Result};
use rusqlite::{params, Connection};
pub(super) fn apply(
    c: &Connection,
    actor: &Actor,
    operation: &Operation,
    resources: &[String],
    store: &Store,
) -> Result<Outcome> {
    let credential_id = actor.credential_id.as_str();
    let request_id = operation
        .request_id()
        .ok_or_else(|| invalid("Logical request ID is required"))?;
    match operation {
        Operation::ContactPairingStart {
            peer_identity_id,
            expires_at,
            ..
        } => crate::contacts::start(c, actor, peer_identity_id, *expires_at),
        Operation::ContactPairingComplete {
            pairing_id,
            challenge,
            ..
        } => crate::contacts::complete(c, actor, pairing_id, challenge),
        Operation::ContactPairingRevoke { pairing_id, .. } => {
            crate::contacts::revoke(c, actor, pairing_id)
        }
        Operation::CallStart {
            conversation_id,
            route_id,
            destination,
            ..
        } => crate::voice::start(
            c,
            actor,
            credential_id,
            conversation_id,
            route_id,
            destination,
            store,
        ),
        Operation::CallEnd {
            conversation_id,
            call_id,
            ..
        } => crate::voice::end(
            c,
            credential_id,
            conversation_id,
            call_id,
            request_id,
            store,
        ),
        Operation::SessionInvite {
            conversation_id,
            participant_id,
            media,
            ..
        } => crate::sessions::invite(
            c,
            actor,
            conversation_id,
            participant_id,
            media,
            resources,
            store,
        )
        .map(Outcome::Session),
        Operation::SessionAccept {
            conversation_id,
            session_id,
            ..
        } => crate::sessions::transition(
            c,
            actor,
            conversation_id,
            session_id,
            crate::sessions::Transition::Accept,
            resources,
            store,
        )
        .map(Outcome::Session),
        Operation::SessionReject {
            conversation_id,
            session_id,
            reason,
            ..
        } => crate::sessions::transition(
            c,
            actor,
            conversation_id,
            session_id,
            crate::sessions::Transition::Reject(reason.clone()),
            resources,
            store,
        )
        .map(Outcome::Session),
        Operation::SessionCancel {
            conversation_id,
            session_id,
            ..
        } => crate::sessions::transition(
            c,
            actor,
            conversation_id,
            session_id,
            crate::sessions::Transition::Cancel,
            resources,
            store,
        )
        .map(Outcome::Session),
        Operation::SessionEnd {
            conversation_id,
            session_id,
            reason,
            ..
        } => crate::sessions::transition(
            c,
            actor,
            conversation_id,
            session_id,
            crate::sessions::Transition::End(reason.clone()),
            resources,
            store,
        )
        .map(Outcome::Session),
        Operation::SipDial {
            conversation_id,
            session_id,
            destination,
            ..
        } => crate::sip_legs::dial(
            c,
            actor,
            credential_id,
            conversation_id,
            session_id,
            destination,
        )
        .map(Outcome::SipLeg),
        Operation::SipEnd {
            conversation_id,
            sip_leg_id,
            ..
        } => crate::sip_legs::end(c, actor, conversation_id, sip_leg_id, request_id)
            .map(Outcome::SipLeg),
        Operation::AvailabilitySet {
            state,
            preferred_route,
            expires_at,
            ..
        } => crate::availability::set(c, actor, state, preferred_route.as_deref(), *expires_at)
            .map(Outcome::Availability),
        Operation::ArtifactBegin {
            conversation_id,
            name,
            media_type,
            size,
            sha256,
            expires_at,
            ..
        } => crate::artifacts::begin(
            c,
            actor,
            crate::artifacts::BeginRequest {
                conversation_id,
                name,
                media_type,
                size: *size,
                sha256,
                expires_at: *expires_at,
                recipients: resources,
            },
            store,
        )
        .map(Outcome::Artifact),
        Operation::ArtifactAppend {
            conversation_id,
            artifact_id,
            offset,
            data,
            ..
        } => crate::artifacts::append(c, actor, conversation_id, artifact_id, *offset, data, store)
            .map(Outcome::Artifact),
        Operation::ArtifactFinish {
            conversation_id,
            artifact_id,
            ..
        } => crate::artifacts::finish(c, actor, conversation_id, artifact_id, resources, store)
            .map(Outcome::Artifact),
        Operation::ArtifactDelete {
            conversation_id,
            artifact_id,
            ..
        } => crate::artifacts::delete(c, actor, conversation_id, artifact_id, resources, store)
            .map(Outcome::Artifact),
        Operation::Create { .. } => {
            if capacity(c, resources.len() as u64, 0, store)? {
                let id = uuid::Uuid::new_v4().to_string();
                c.execute("INSERT INTO conversations(id) VALUES(?1)", [&id])
                    .map_err(|_| Error::storage())?;
                for member in resources {
                    c.execute(
                        "INSERT INTO participants(conversation_id,identity_id) VALUES(?1,?2)",
                        params![id, member],
                    )
                    .map_err(|_| Error::storage())?;
                    insert_event(c, member, &id, "conversation_created", None)?;
                }
                Ok(Outcome::Conversation(Conversation {
                    id,
                    participants: resources.to_vec(),
                    closed: false,
                }))
            } else {
                Err(Error::new(
                    "capacity-exceeded",
                    "Durable event capacity exceeded",
                ))
            }
        }
        Operation::Send {
            conversation_id,
            body,
            ..
        } => {
            let conversation = authorized_conversation(c, actor, conversation_id)?;
            if conversation.closed {
                Err(Error::new(
                    "conversation-closed",
                    "Conversation no longer accepts messages",
                ))
            } else if body.is_empty() || body.len() > store.limits.message_bytes {
                Err(invalid(
                    "Message must be nonempty and within the byte limit",
                ))
            } else if !capacity(c, resources.len() as u64, body.len() as u64, store)? {
                Err(Error::new(
                    "capacity-exceeded",
                    "Durable message or event capacity exceeded",
                ))
            } else {
                let id = uuid::Uuid::new_v4().to_string();
                c.execute(
                    "INSERT INTO messages(id,conversation_id,sender,body) VALUES(?1,?2,?3,?4)",
                    params![id, conversation_id, actor.identity_id, body],
                )
                .map_err(|_| Error::storage())?;
                for member in resources {
                    insert_event(c, member, conversation_id, "message", Some(&id))?;
                }
                crate::providers::enqueue(
                    c,
                    &id,
                    conversation_id,
                    &actor.identity_id,
                    credential_id,
                    store.limits.operation_receipts,
                )?;
                Ok(Outcome::Accepted {
                    request_id: request_id.into(),
                    message_id: id,
                    conversation_id: conversation_id.clone(),
                })
            }
        }
        Operation::Close {
            conversation_id, ..
        } => {
            let conversation = authorized_conversation(c, actor, conversation_id)?;
            if conversation.closed {
                Ok(Outcome::Conversation(conversation))
            } else if let Err(error) =
                crate::sessions::ensure_conversation_closable(c, conversation_id)
            {
                Err(error)
            } else if !capacity(c, resources.len() as u64, 0, store)? {
                Err(Error::new(
                    "capacity-exceeded",
                    "Durable event capacity exceeded",
                ))
            } else {
                c.execute(
                    "UPDATE conversations SET closed=1 WHERE id=?1",
                    [conversation_id],
                )
                .map_err(|_| Error::storage())?;
                for member in resources {
                    insert_event(c, member, conversation_id, "conversation_closed", None)?;
                }
                Ok(Outcome::Conversation(Conversation {
                    closed: true,
                    ..conversation
                }))
            }
        }
        _ => Err(invalid("Unsupported mutation")),
    }
}
