//! Explicit policy boundary for unauthenticated SIP and WebRTC callers.
use connect_store::Store;
use rvoip_core::{
    adapter::RejectReason,
    commands::InboundAction,
    connection::Transport,
    ids::{ConnectionId, ConversationId, ParticipantId, SessionId, TenantId},
    session::SessionMedium,
    Orchestrator,
};
use std::sync::Arc;

#[derive(Clone, Default)]
pub(crate) struct InboundMediaRoutes {
    pub sip: Option<crate::config::InboundMediaRoute>,
    pub webrtc: Option<crate::config::InboundMediaRoute>,
}

pub(crate) struct InboundBinding {
    pub credential_id: String,
    pub session_id: String,
    pub owns_session: bool,
}

/// Admit a transport connection only through its configured durable route.
/// SIP headers, SDP, caller ID and WebRTC offer fields never select authority.
pub(crate) async fn admit(
    orchestrator: &Arc<Orchestrator>,
    store: &Arc<Store>,
    routes: &InboundMediaRoutes,
    connection_id: ConnectionId,
    namespace: &str,
) -> Option<InboundBinding> {
    let transport = match orchestrator.connection_transport(&connection_id) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(%connection_id, %error, "could not identify inbound media transport");
            return None;
        }
    };
    let route = match transport {
        Transport::Sip => routes.sip.as_ref(),
        Transport::WebRtc => routes.webrtc.as_ref(),
        _ => return None,
    };
    let Some(route) = route else {
        reject(orchestrator, connection_id, RejectReason::Forbidden).await;
        return None;
    };
    let selected = if transport == Transport::WebRtc {
        match selected_browser_session(orchestrator, &connection_id) {
            Ok(value) => value,
            Err(_) => {
                reject(orchestrator, connection_id, RejectReason::Forbidden).await;
                return None;
            }
        }
    } else {
        None
    };
    if transport == Transport::WebRtc
        && selected.is_none()
        && configured_video_session(store, route).is_none()
        && !orchestrator
            .connection_principal(&connection_id)
            .is_ok_and(|principal| {
                !principal.is_expired() && principal.subject == route.credential_id
            })
    {
        reject(orchestrator, connection_id, RejectReason::Forbidden).await;
        return None;
    }
    let owns_session = selected.is_none() && route.session_id.is_none();
    let admitted = if let Some((cid, sid)) = selected.as_ref() {
        let principal = match orchestrator.connection_principal(&connection_id) {
            Ok(value) if !value.is_expired() => value,
            _ => {
                reject(orchestrator, connection_id, RejectReason::Forbidden).await;
                return None;
            }
        };
        store.authorize_selected_browser_video(&principal.subject, &browser_route(route), cid, sid)
    } else {
        match route.session_id.as_deref() {
            Some(session_id)
                if transport == Transport::WebRtc
                    && configured_video_session(store, route).is_some() =>
            {
                admit_video_participant(orchestrator, store, &connection_id, route, session_id)
            }
            Some(session_id) => store.admit_configured_session(
                &route.credential_id,
                &route.conversation_id,
                &route.assistant_identity_id,
                session_id,
            ),
            None => store.admit_routed_inbound_session(
                &route.credential_id,
                &route.conversation_id,
                &route.assistant_identity_id,
            ),
        }
    };
    let admitted = match admitted {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(
                code = error.code,
                "configured inbound media route was denied"
            );
            reject(orchestrator, connection_id, RejectReason::Forbidden).await;
            return None;
        }
    };
    let canonical_conversation = ConversationId::from_string(format!(
        "connect:{namespace}:{}",
        admitted.session.conversation_id
    ));
    let canonical_session =
        SessionId::from_string(format!("connect:{namespace}:{}", admitted.session.id));
    if orchestrator
        .project_external_session(
            canonical_conversation,
            canonical_session.clone(),
            TenantId::from_string(namespace),
            if admitted.session.media.iter().any(|kind| kind == "video") {
                SessionMedium::VoiceVideo
            } else {
                SessionMedium::Voice
            },
            [
                ParticipantId::from_string(&admitted.session.initiator_id),
                ParticipantId::from_string(&admitted.session.participant_id),
            ],
        )
        .is_err()
    {
        if owns_session {
            end_admission(store, &admitted.session.id, "media-projection-failed");
        }
        reject(orchestrator, connection_id, RejectReason::NotAcceptable).await;
        return None;
    }
    if let Err(error) = orchestrator
        .route_inbound_connection(
            connection_id.clone(),
            InboundAction::Accept {
                session_id: canonical_session,
                participant_id: ParticipantId::from_string(admitted.caller_identity_id),
            },
        )
        .await
    {
        tracing::warn!(%connection_id, %error, "could not accept routed inbound media connection");
        if owns_session {
            end_admission(store, &admitted.session.id, "media-admission-failed");
        }
        reject(orchestrator, connection_id, RejectReason::NotAcceptable).await;
        return None;
    }
    Some(InboundBinding {
        credential_id: admitted.credential_id,
        session_id: admitted.session.id,
        owns_session,
    })
}

pub(crate) fn browser_route(
    route: &crate::config::InboundMediaRoute,
) -> connect_store::BrowserRoute {
    connect_store::BrowserRoute {
        credential_id: route.credential_id.clone(),
        binding: connect_protocol::BrowserBinding {
            conversation_id: route.conversation_id.clone(),
            participant_id: route.assistant_identity_id.clone(),
            session_id: route.session_id.clone(),
            signaling_url: String::new(),
        },
    }
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserSessionSelection {
    conversation_id: String,
    session_id: String,
}
fn selected_browser_session(
    orchestrator: &Orchestrator,
    connection_id: &ConnectionId,
) -> connect_protocol::Result<Option<(String, String)>> {
    let principal = orchestrator
        .connection_principal(connection_id)
        .map_err(|_| connect_protocol::Error::forbidden())?;
    let context = orchestrator
        .take_inbound_context(connection_id, &principal)
        .map_err(|_| connect_protocol::Error::forbidden())?;
    let Some(context) = context else {
        return Ok(None);
    };
    let Some(hint) = context.routing_hint() else {
        return Ok(None);
    };
    let Some(selection) = hint
        .expose_secret()
        .strip_prefix("webrtc-session-selection-v1:")
    else {
        return Ok(None);
    };
    let selection: BrowserSessionSelection =
        serde_json::from_str(selection).map_err(|_| connect_protocol::Error::forbidden())?;
    Ok(Some((selection.conversation_id, selection.session_id)))
}

/// An accepted administrator-selected video Session opts into authenticated
/// participant joining. Its durable IDs remain fixed; SDP supplies no authority.
pub(crate) fn configured_video_session(
    store: &Store,
    route: &crate::config::InboundMediaRoute,
) -> Option<connect_protocol::Session> {
    let session = store
        .authorize_session_binding(&route.credential_id, route.session_id.as_deref()?)
        .ok()?;
    (session.conversation_id == route.conversation_id
        && session.participant_id == route.assistant_identity_id
        && session.media == ["audio", "video"])
    .then_some(session)
}

fn admit_video_participant(
    orchestrator: &Orchestrator,
    store: &Store,
    connection_id: &ConnectionId,
    route: &crate::config::InboundMediaRoute,
    session_id: &str,
) -> connect_protocol::Result<connect_store::RoutedInboundSession> {
    let principal = orchestrator
        .connection_principal(connection_id)
        .map_err(|_| connect_protocol::Error::forbidden())?;
    if principal.is_expired() {
        return Err(connect_protocol::Error::forbidden());
    }
    let actor = store.revalidate(&principal.subject)?;
    let session = store.authorize_session_binding(&actor.credential_id, session_id)?;
    if session.conversation_id != route.conversation_id
        || session.participant_id != route.assistant_identity_id
        || session.media != ["audio", "video"]
    {
        return Err(connect_protocol::Error::forbidden());
    }
    Ok(connect_store::RoutedInboundSession {
        session,
        credential_id: actor.credential_id,
        caller_identity_id: actor.identity_id,
    })
}

fn end_admission(store: &Store, session_id: &str, reason: &str) {
    if let Err(error) = store.end_routed_inbound_session(session_id, reason) {
        tracing::error!(
            code = error.code,
            %session_id,
            "could not terminate failed inbound media admission"
        );
    }
}

async fn reject(
    orchestrator: &Arc<Orchestrator>,
    connection_id: ConnectionId,
    reason: RejectReason,
) {
    let _ = orchestrator
        .route_inbound_connection(connection_id, InboundAction::Reject { reason })
        .await;
}
