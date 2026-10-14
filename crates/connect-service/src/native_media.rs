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
            .map_err(super::session_bindings::resource_error)?;
    }
    let video = session.media == ["audio", "video"];
    if (!video && session.media != ["audio"])
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
                || offer.medium != if video { "voice-video" } else { "voice" }
                || offer.intent != "connect-session"
                || offer.capabilities_offer
                    != if video {
                        serde_json::json!({"audio":["g.711-mu"],"video":["vp8"]})
                    } else {
                        serde_json::json!({"audio":["g.711-mu"]})
                    }
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
                || !valid_stream_offer(&offer.streams_offered, video)
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
                        || s.kinds
                            .iter()
                            .any(|kind| kind != "audio" && !(video && kind == "video"))
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
                .any(|codec| codec != "g.711-mu" && !(video && codec == "vp8"))
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

/// Exact approved native profile, checked before the UCTP handler allocates streams.
fn valid_stream_offer(streams: &[connection::StreamOffer], video: bool) -> bool {
    let expected = if video { 2 } else { 1 };
    streams.len() == expected
        && streams.iter().all(|stream| !stream.id.is_empty())
        && (streams.len() == 1 || streams[0].id != streams[1].id)
        && streams
            .iter()
            .filter(|stream| stream.kind == "audio")
            .count()
            == 1
        && streams
            .iter()
            .filter(|stream| stream.kind == "video")
            .count()
            == usize::from(video)
        && streams.iter().all(|stream| {
            (if video {
                stream.direction == "sendrecv"
            } else {
                matches!(
                    stream.direction.as_str(),
                    "sendrecv" | "sendonly" | "recvonly"
                )
            }) && stream.codec_preferences
                == [if stream.kind == "audio" {
                    "g.711-mu"
                } else {
                    "vp8"
                }]
        })
}

/// Regular backing credentials retain generic audio behavior but accepted video
/// Sessions use the same exact profile boundary as scoped native task grants.
pub(crate) fn authorize_owner_video(
    store: &Store,
    principal: &AuthenticatedPrincipal,
    envelope: &UctpEnvelope,
) -> Result<(), ResourceBindingError> {
    if !matches!(
        envelope.msg_type,
        MessageType::SessionInvite | MessageType::ConnectionOffer
    ) {
        return Ok(());
    }
    let Some(sid) = envelope.sid.as_deref() else {
        return Ok(());
    };
    let session = store
        .authorize_session_binding(&principal.subject, sid)
        .map_err(super::session_bindings::resource_error)?;
    if !session.media.iter().any(|kind| kind == "video") {
        return Ok(());
    }
    if session.media != ["audio", "video"]
        || envelope
            .cid
            .as_deref()
            .is_some_and(|cid| cid != session.conversation_id)
    {
        return denied();
    }
    store
        .vapi_session_route(&principal.subject, sid)
        .map_err(super::session_bindings::resource_error)?;
    match envelope.msg_type {
        MessageType::SessionInvite => {
            let offer: session::SessionInvite =
                envelope.decode_payload().map_err(|_| forbidden())?;
            let actor = store
                .revalidate(&principal.subject)
                .map_err(super::session_bindings::resource_error)?;
            let counterpart = if actor.identity_id == session.initiator_id {
                &session.participant_id
            } else {
                &session.initiator_id
            };
            if offer.from != principal.subject
                || offer.to != [counterpart.clone()]
                || envelope.cid.as_deref() != Some(session.conversation_id.as_str())
                || offer.medium != "voice-video"
                || offer.intent != "connect-session"
                || offer.capabilities_offer
                    != serde_json::json!({"audio":["g.711-mu"],"video":["vp8"]})
            {
                return denied();
            }
        }
        MessageType::ConnectionOffer => {
            let offer: connection::ConnectionOffer =
                envelope.decode_payload().map_err(|_| forbidden())?;
            if offer.by_participant != principal.subject
                || offer.substrate != "quic"
                || !offer.substrate_setup.is_null()
                || offer.capabilities != serde_json::json!({})
                || !valid_stream_offer(&offer.streams_offered, true)
            {
                return denied();
            }
        }
        _ => {}
    }
    Ok(())
}
