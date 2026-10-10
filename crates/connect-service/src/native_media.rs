//! Native task media is an exact Session attachment, not general UCTP authority.
use connect_protocol::PROFILE;
use connect_store::Store;
use rvoip_core::AuthenticatedPrincipal;
use rvoip_uctp::{
    envelope::UctpEnvelope,
    payloads::{connection, session, stream},
    state::{ResourceBindingError, UctpScopePolicy},
    types::MessageType,
};

/// Connect's bounded native media entry point. It never authorizes raw
/// conversation, identity or data commands. Current resources are checked again
/// for every envelope and by the physical peer's authorization supervisor.
pub const NATIVE_MEDIA_SCOPE: &str = "connect:native-media";

/// Keep stock UCTP authorization for every other command family. Owner peers
/// retain their existing behavior; Native grants receive only this media scope.
pub fn native_media_scope_policy() -> UctpScopePolicy {
    let mut policy = UctpScopePolicy::secure_defaults();
    for kind in [
        MessageType::SessionInvite,
        MessageType::ConnectionOffer,
        MessageType::ConnectionReady,
        MessageType::ConnectionUpdate,
        MessageType::ConnectionQuality,
        MessageType::ConnectionEnd,
        MessageType::StreamSubscribe,
        MessageType::StreamUnsubscribe,
    ] {
        policy = policy.with_required_scope(kind, NATIVE_MEDIA_SCOPE);
    }
    policy
}

pub(crate) fn authorize(
    store: &Store,
    namespace: &str,
    principal: &AuthenticatedPrincipal,
    envelope: &UctpEnvelope,
) -> Result<(), ResourceBindingError> {
    if principal.issuer.as_deref() != Some("connect-local")
        || principal.tenant.as_deref() != Some(namespace)
        || principal.is_expired()
    {
        return denied();
    }
    let authority = store
        .revalidate_native_task_principal(&principal.subject)
        .map_err(super::session_bindings::resource_error)?;
    if let Some(profile) = envelope.payload.get("profile").and_then(|v| v.as_str()) {
        // Only the registered application handler can dispatch this branch. It
        // validates request type/profile and admits only the task facade.
        return if profile == PROFILE { Ok(()) } else { denied() };
    }
    if envelope.msg_type == MessageType::AuthRefresh {
        return Ok(()); // Stock validation also enforces physical-peer ownership.
    }
    if matches!(envelope.msg_type, MessageType::Ack | MessageType::Error)
        && envelope.in_reply_to.is_some()
        && envelope.sid.is_none()
    {
        return Ok(()); // Only this peer's correlation map can consume the reply.
    }
    if !authority.permits_media_attachment() {
        return denied();
    }
    let sid = envelope.sid.as_deref().ok_or_else(forbidden)?;
    let session = store
        .authorize_native_task_session_binding(&principal.subject, sid)
        .map_err(super::session_bindings::resource_error)?;
    if envelope.msg_type == MessageType::SessionInvite {
        store
            .vapi_session_route(&principal.subject, sid)
            .map_err(|error| {
                if error.code == "unsupported-task-media-route" {
                    ResourceBindingError {
                        code: 501,
                        reason: "connect-task-provider-context-unqualified".into(),
                    }
                } else {
                    super::session_bindings::resource_error(error)
                }
            })?;
    }
    if session.media != ["audio"]
        || envelope
            .cid
            .as_deref()
            .is_some_and(|id| id != session.conversation_id)
    {
        return denied();
    }
    // UCTP auth.session names the transport participant with the authenticated
    // subject. The backing task owner remains the durable Connect identity.
    let participant = principal.subject.as_str();
    let counterpart = if authority.identity_id() == session.initiator_id {
        &session.participant_id
    } else {
        &session.initiator_id
    };
    if envelope.msg_type != MessageType::SessionInvite && envelope.connid.is_none() {
        return denied();
    }
    match envelope.msg_type {
        MessageType::SessionInvite => {
            let offer: session::SessionInvite =
                envelope.decode_payload().map_err(|_| forbidden())?;
            if envelope.cid.as_deref() != Some(session.conversation_id.as_str())
                || envelope.connid.is_some()
                || offer.from != participant
                || offer.to != [counterpart.clone()]
                || offer.medium != "voice"
                || offer.intent != "connect-session"
                || offer.capabilities_offer != serde_json::json!({"audio":["g.711-mu"]})
            {
                return denied();
            }
        }
        MessageType::ConnectionOffer => {
            let offer: connection::ConnectionOffer =
                envelope.decode_payload().map_err(|_| forbidden())?;
            if offer.by_participant != participant
                || offer.substrate != "quic"
                || !offer.substrate_setup.is_null()
                || offer.capabilities != serde_json::json!({})
                || offer.streams_offered.len() != 1
                || offer.streams_offered.iter().any(|s| {
                    s.kind != "audio"
                        || !matches!(s.direction.as_str(), "sendrecv" | "sendonly" | "recvonly")
                        || s.codec_preferences != ["g.711-mu"]
                })
            {
                return denied();
            }
        }
        MessageType::StreamSubscribe => {
            let request: stream::StreamSubscribe =
                envelope.decode_payload().map_err(|_| forbidden())?;
            if request.by_participant != participant
                || request.subscriptions.is_empty()
                || request.subscriptions.len() > 16
                || request.subscriptions.iter().any(|s| {
                    s.from_participant
                        .as_ref()
                        .is_some_and(|p| p != counterpart)
                        || s.kinds.iter().any(|kind| kind != "audio")
                })
            {
                return denied();
            }
            // Exact stream/connection ownership is checked by the stock bound
            // subscription handler against this canonical Session registry.
        }
        MessageType::StreamUnsubscribe => {
            let request: stream::StreamUnsubscribe =
                envelope.decode_payload().map_err(|_| forbidden())?;
            if request.strm_ids.len() > 16 {
                return denied();
            }
        }
        MessageType::ConnectionUpdate => {
            let update: connection::ConnectionUpdate =
                envelope.decode_payload().map_err(|_| forbidden())?;
            if !matches!(
                update.action.as_str(),
                "hold" | "resume" | "mute" | "unmute"
            ) || update
                .codec_preferences
                .iter()
                .any(|codec| codec != "g.711-mu")
                || update.details != serde_json::json!({})
            {
                return denied();
            }
        }
        MessageType::ConnectionReady
        | MessageType::ConnectionEnd
        | MessageType::ConnectionQuality => {}
        _ => return denied(),
    }
    Ok(())
}

fn forbidden() -> ResourceBindingError {
    ResourceBindingError::forbidden("connect-task-media-denied")
}
fn denied() -> Result<(), ResourceBindingError> {
    Err(forbidden())
}
