//! Current resource authority is checked before a receipt or effect is used.
use super::invalid;
use crate::{conversations::authorized_conversation, Actor, Store};
use connect_protocol::{Error, Operation, Result};
use rusqlite::Connection;
pub(crate) fn authorize(
    c: &Connection,
    actor: &Actor,
    operation: &Operation,
    store: &Store,
) -> Result<Vec<String>> {
    if let Operation::ConnectionPrepare { request } = operation {
        return crate::connection_control::resources(c, actor, request);
    }
    if let Operation::SessionControl { request } = operation {
        return crate::session_control::resources(c, actor, request);
    }
    if let Operation::SessionDecision { request } = operation {
        return crate::session_decisions::resources(c, actor, request);
    }
    if let Operation::Membership { request } = operation {
        return crate::membership::control::resources(c, actor, request);
    }
    use connect_protocol::membership::ContentKind;
    if let Operation::MessageReceipt {
        conversation_id,
        message_id,
        ..
    } = operation
    {
        return crate::recipient_receipts::acknowledge_authority(
            c,
            actor,
            conversation_id,
            message_id,
        );
    }
    if let Operation::ReceiptPreferencesSet {
        conversation_id, ..
    } = operation
    {
        crate::audience::member(c, &actor.identity_id, conversation_id, true)?;
        return Ok(vec![actor.identity_id.clone()]);
    }
    if let Operation::Send {
        conversation_id,
        message,
        ..
    } = operation
    {
        return crate::messages::recipients(c, actor, conversation_id, message)
            .map(|members| members.into_iter().map(|p| p.identity_id).collect());
    }
    if let Operation::ArtifactBegin {
        conversation_id,
        to,
        ..
    } = operation
    {
        if authorized_conversation(c, actor, conversation_id)?.closed {
            return Err(Error::new(
                "conversation-closed",
                "Conversation no longer accepts artifacts",
            ));
        }
        return crate::audience::select_members(c, actor, conversation_id, to)
            .map(|members| members.into_iter().map(|p| p.identity_id).collect());
    }
    let selected = match operation {
        Operation::SessionAccept {
            conversation_id,
            session_id,
            ..
        }
        | Operation::SessionReject {
            conversation_id,
            session_id,
            ..
        }
        | Operation::SessionCancel {
            conversation_id,
            session_id,
            ..
        }
        | Operation::SessionEnd {
            conversation_id,
            session_id,
            ..
        } => Some((conversation_id, session_id, ContentKind::Session)),
        Operation::ArtifactAppend {
            conversation_id,
            artifact_id,
            ..
        }
        | Operation::ArtifactLinkAppend {
            conversation_id,
            artifact_id,
            ..
        }
        | Operation::ArtifactFinish {
            conversation_id,
            artifact_id,
            ..
        }
        | Operation::ArtifactDelete {
            conversation_id,
            artifact_id,
            ..
        } => Some((conversation_id, artifact_id, ContentKind::Artifact)),
        Operation::CallEnd {
            conversation_id,
            call_id,
            ..
        } => Some((conversation_id, call_id, ContentKind::Call)),
        Operation::SipEnd {
            conversation_id,
            sip_leg_id,
            ..
        } => Some((conversation_id, sip_leg_id, ContentKind::SipLeg)),
        _ => None,
    };
    if let Some((conversation, id, kind)) = selected {
        authorized_conversation(c, actor, conversation)?;
        crate::audience::require_original(c, actor, conversation, kind, id)?;
        return crate::audience::source_members(c, conversation, kind, id);
    }
    Ok(match operation {
        Operation::ContactPairingStart {
            peer_identity_id, ..
        } => {
            crate::contacts::authorize_peer(c, actor, peer_identity_id)?;
            vec![actor.identity_id.clone(), peer_identity_id.clone()]
        }
        Operation::ContactPairingComplete { pairing_id, .. }
        | Operation::ContactPairingRevoke { pairing_id, .. } => crate::contacts::authorize_pairing(
            c,
            actor,
            pairing_id,
            matches!(operation, Operation::ContactPairingComplete { .. }),
        )?,
        Operation::Create { request, .. } => {
            crate::conversation_control::create_resources(c, actor, request, store)?
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
