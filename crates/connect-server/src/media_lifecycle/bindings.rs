//! Current-authority binding, pair ownership and durable connection cleanup.
use super::MediaState;
use connect_store::{SipConnectionEnd, Store};
use rvoip_core::{adapter::EndReason, ids::ConnectionId, Orchestrator};
use rvoip_vapi::VapiAdapter;
use std::sync::Arc;

pub(super) async fn finish_connection(
    orchestrator: &Arc<Orchestrator>,
    store: &Arc<Store>,
    state: &mut MediaState,
    connection_id: &ConnectionId,
    end: SipConnectionEnd,
) {
    state.credentials.remove(connection_id);
    let durable_session = state.sessions.remove(connection_id);
    let routed = state.routed_connections.remove(connection_id);
    let sip_leg = state.sip_legs.remove(connection_id);
    for waiting in state.waiting_by_session.values_mut() {
        waiting.retain(|candidate| candidate != connection_id);
    }
    state
        .waiting_by_session
        .retain(|_, waiting| !waiting.is_empty());
    let paired = state.paired_connections.remove(connection_id);
    if let Some(peer) = paired.as_ref() {
        state.paired_connections.remove(peer);
    }
    for id in std::iter::once(connection_id).chain(paired.as_ref()) {
        if let Some(bridge) = state.video_bridges.remove(id) {
            bridge.shutdown().await;
        }
    }
    state.vapi.end_connection(connection_id).await;
    let persisted = if routed {
        durable_session.as_deref().map_or(Ok(None), |session_id| {
            store
                .end_routed_inbound_session(session_id, "media-leg-ended")
                .map(Some)
        })
    } else {
        store.unbind_session_connection(connection_id.as_str())
    };
    if let Err(error) = persisted {
        tracing::warn!(code = error.code, "could not persist media termination");
    }
    if sip_leg.is_some() {
        if let Err(error) = store.observe_sip_connection_end(
            connection_id.as_str(),
            end,
            if end == SipConnectionEnd::Failed {
                "sip-connection-failed"
            } else {
                "sip-connection-ended"
            },
        ) {
            tracing::warn!(code = error.code, "could not persist SIP leg termination");
        }
    }
    if let Some(peer) = paired {
        let _ = orchestrator
            .end_connection(peer, EndReason::BridgeTorn)
            .await;
    }
}

pub(super) async fn bind_or_end_routed_connection(
    orchestrator: &Arc<Orchestrator>,
    store: &Arc<Store>,
    vapi: Option<&Arc<VapiAdapter>>,
    state: &mut MediaState,
    connection_id: &ConnectionId,
) {
    let routed = state.routed_connections.contains(connection_id);
    if let Err(reason) =
        bind_and_pair_connection(orchestrator, store, vapi, state, connection_id, routed).await
    {
        tracing::warn!(%connection_id, reason, "media connection could not become ready");
        if routed {
            if let Some(session_id) = state.sessions.get(connection_id) {
                if let Err(error) = store.end_routed_inbound_session(session_id, reason) {
                    tracing::error!(
                        code = error.code,
                        %session_id,
                        "could not terminate failed routed media session"
                    );
                }
            }
        }
        let _ = orchestrator
            .end_connection(
                connection_id.clone(),
                EndReason::Failed {
                    detail: reason.into(),
                },
            )
            .await;
    }
}

pub(super) async fn bind_and_pair_connection(
    orchestrator: &Arc<Orchestrator>,
    store: &Arc<Store>,
    vapi: Option<&Arc<VapiAdapter>>,
    state: &mut MediaState,
    connection_id: &ConnectionId,
    routed: bool,
) -> std::result::Result<(), &'static str> {
    let (Some(credential), Some(durable_session)) = (
        state.credentials.get(connection_id),
        state.sessions.get(connection_id),
    ) else {
        return Ok(());
    };
    let session = store
        .bind_session_connection(credential, durable_session, connection_id.as_str())
        .map_err(|error| {
            tracing::warn!(code = error.code, "could not persist media readiness");
            "media-binding-denied"
        })?;
    let outcome = state
        .vapi
        .bind(
            orchestrator,
            store,
            vapi,
            crate::vapi_media::VapiBinding {
                credential,
                durable_session,
                connection_id,
                routed,
            },
        )
        .await?;
    if outcome == crate::vapi_media::BindOutcome::NoRoute {
        pair_connection(
            orchestrator,
            store,
            state,
            connection_id,
            session.media.iter().any(|kind| kind == "video"),
        )
        .await?;
    }
    Ok(())
}

async fn pair_connection(
    orchestrator: &Arc<Orchestrator>,
    store: &Store,
    state: &mut MediaState,
    connection_id: &ConnectionId,
    video: bool,
) -> std::result::Result<(), &'static str> {
    if state.paired_connections.contains_key(connection_id) {
        return Ok(());
    }
    let Some(session_id) = state.sessions.get(connection_id).cloned() else {
        return Ok(());
    };
    let transport = orchestrator
        .connection_transport(connection_id)
        .map_err(|_| "media-connection-unavailable")?;
    let waiting = state
        .waiting_by_session
        .entry(session_id.clone())
        .or_default();
    let peer_index = waiting.iter().position(|candidate| {
        candidate != connection_id
            && (if video {
                // The durable authority boundary admits only Session participants;
                // require distinct authenticated identities for a video pairing.
                let identities = [candidate, connection_id].map(|id| {
                    orchestrator
                        .connection_principal(id)
                        .ok()
                        .and_then(|principal| match principal.assurance {
                            rvoip_core::IdentityAssurance::TaskScoped { identity, .. }
                            | rvoip_core::IdentityAssurance::UserAuthorized { identity, .. } => {
                                Some(identity)
                            }
                            _ => None,
                        })
                });
                matches!(&identities, [Some(a), Some(b)] if a != b)
            } else {
                orchestrator
                    .connection_transport(candidate)
                    .is_ok_and(|candidate_transport| candidate_transport != transport)
            })
    });
    let Some(peer_index) = peer_index else {
        if !waiting.contains(connection_id) {
            waiting.push(connection_id.clone());
        }
        return Ok(());
    };
    let peer = waiting.remove(peer_index);
    if waiting.is_empty() {
        state.waiting_by_session.remove(&session_id);
    }
    for id in [&peer, connection_id] {
        let credential = state.credentials.get(id).ok_or("media-binding-denied")?;
        let authorized = if credential.starts_with(connect_protocol::tasks::TASK_PRINCIPAL_PREFIX) {
            store.authorize_native_task_session_binding(credential, &session_id)
        } else {
            store.authorize_session_binding(credential, &session_id)
        };
        if authorized.is_err() {
            return Err("media-binding-denied");
        }
    }
    if video
        && transport == rvoip_core::connection::Transport::Quic
        && orchestrator
            .connection_transport(&peer)
            .is_ok_and(|peer| peer == transport)
    {
        // Native AV forwarding remains explicit authorized UCTP fanout. Own
        // only its paired lifecycle so either terminal leg closes its peer.
        state
            .paired_connections
            .insert(peer.clone(), connection_id.clone());
        state.paired_connections.insert(connection_id.clone(), peer);
        return Ok(());
    }
    match orchestrator
        .bridge_connections(peer.clone(), connection_id.clone())
        .await
    {
        Ok(_) => {
            if video {
                match orchestrator
                    .bridge_encoded_video(peer.clone(), connection_id.clone())
                    .await
                {
                    Ok(Some(bridge)) => {
                        state.video_bridges.insert(connection_id.clone(), bridge);
                    }
                    _ => {
                        let _ = orchestrator
                            .end_connection(peer, EndReason::BridgeTorn)
                            .await;
                        return Err("video-stream-bridge-failed");
                    }
                }
            }
            state
                .paired_connections
                .insert(peer.clone(), connection_id.clone());
            state.paired_connections.insert(connection_id.clone(), peer);
            Ok(())
        }
        Err(error) => {
            tracing::warn!(%error, "could not bridge cross-transport Session peers");
            state
                .waiting_by_session
                .entry(session_id)
                .or_default()
                .push(peer);
            Err("cross-transport-bridge-failed")
        }
    }
}
