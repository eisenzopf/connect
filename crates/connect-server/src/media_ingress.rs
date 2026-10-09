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
    let admitted = match route.session_id.as_deref() {
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
            SessionMedium::Voice,
            [
                ParticipantId::from_string(&admitted.caller_identity_id),
                ParticipantId::from_string(&admitted.session.participant_id),
            ],
        )
        .is_err()
    {
        end_admission(store, &admitted.session.id, "media-projection-failed");
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
        end_admission(store, &admitted.session.id, "media-admission-failed");
        reject(orchestrator, connection_id, RejectReason::NotAcceptable).await;
        return None;
    }
    Some(InboundBinding {
        credential_id: admitted.credential_id,
        session_id: admitted.session.id,
        owns_session: route.session_id.is_none(),
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
