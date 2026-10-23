//! Existing mutation implementations; network dispatch remains outside SQLite.
use super::invalid;
use crate::{Actor, Store};
use connect_protocol::{Operation, Outcome, Result};
use rusqlite::Connection;
pub(super) fn apply(
    c: &Connection,
    actor: &Actor,
    operation: &Operation,
    resources: &[String],
    store: &Store,
    create_id: Option<&str>,
) -> Result<Outcome> {
    let credential_id = actor.credential_id.as_str();
    let request_id = operation
        .request_id()
        .ok_or_else(|| invalid("Logical request ID is required"))?;
    match operation {
        Operation::ConnectionPrepare { request } => {
            crate::connection_control::apply(c, actor, request, store)
        }
        Operation::SessionControl { request } => {
            crate::session_control::apply(c, actor, request, store)
        }
        Operation::SessionDecision { request } => {
            crate::session_decisions::apply(c, actor, request, resources, store)
                .map(Outcome::SessionDecision)
        }
        Operation::Membership { request } => {
            crate::membership::control::apply(c, actor, request, store).map(Outcome::Membership)
        }
        Operation::MessageReceipt {
            conversation_id,
            message_id,
            state,
            ..
        } => crate::recipient_receipts::acknowledge(
            c,
            actor,
            conversation_id,
            message_id,
            *state,
            store,
        ),
        Operation::ReceiptPreferencesSet {
            conversation_id,
            delivery,
            read,
            ..
        } => {
            crate::recipient_receipts::set_preferences(c, actor, conversation_id, *delivery, *read)
        }
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
            crate::sessions::Transition::Cancel(None),
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
        }
        | Operation::ArtifactLinkAppend {
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
        Operation::Create { request, .. } => crate::conversation_control::create(
            c,
            actor,
            create_id.ok_or_else(connect_protocol::Error::forbidden)?,
            request,
            resources,
            store,
        )
        .map(Outcome::Conversation),
        Operation::Send {
            conversation_id,
            message,
            ..
        } => {
            let id = crate::messages::write(c, actor, conversation_id, message.clone(), store)?;
            Ok(Outcome::Accepted {
                receipt: None,
                request_id: request_id.into(),
                message_id: id,
                conversation_id: conversation_id.clone(),
            })
        }
        Operation::Close {
            conversation_id,
            options,
            ..
        } => crate::conversation_control::close(c, actor, conversation_id, options, store)
            .map(Outcome::Conversation),
        _ => Err(invalid("Unsupported mutation")),
    }
}
