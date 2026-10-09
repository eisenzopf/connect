use anyhow::Result;
use connect_store::Store;
use rvoip_core::{
    adapter::EndReason,
    commands::InboundAction,
    events::Event,
    ids::{ConnectionId, ParticipantId},
    Orchestrator,
};
use rvoip_vapi::VapiAdapter;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use tokio_util::sync::CancellationToken;

pub(crate) struct MediaLifecycle {
    stop: CancellationToken,
    commands: tokio::sync::mpsc::Sender<Command>,
    task: tokio::task::JoinHandle<()>,
}

#[derive(Clone)]
pub(crate) struct MediaRegistrar {
    commands: tokio::sync::mpsc::Sender<Command>,
}

enum Command {
    RegisterOutbound {
        connection_id: ConnectionId,
        credential_id: String,
        session_id: String,
        sip_leg_id: String,
        acknowledged: tokio::sync::oneshot::Sender<()>,
    },
    ActivateOutbound {
        connection_id: ConnectionId,
        completed: tokio::sync::oneshot::Sender<std::result::Result<(), &'static str>>,
    },
}

#[derive(Default)]
struct MediaState {
    credentials: HashMap<ConnectionId, String>,
    sessions: HashMap<ConnectionId, String>,
    routed_connections: HashSet<ConnectionId>,
    sip_legs: HashMap<ConnectionId, String>,
    waiting_by_session: HashMap<String, Vec<ConnectionId>>,
    paired_connections: HashMap<ConnectionId, ConnectionId>,
    vapi: crate::vapi_media::VapiMedia,
}

impl MediaLifecycle {
    pub(crate) fn start(
        orchestrator: Arc<Orchestrator>,
        store: Arc<Store>,
        vapi: Option<Arc<VapiAdapter>>,
        inbound_routes: crate::media_ingress::InboundMediaRoutes,
    ) -> Result<Self> {
        let namespace = store.installation_id()?;
        let prefix = format!("connect:{namespace}:");
        let mut events = orchestrator.subscribe_events();
        let stop = CancellationToken::new();
        let cancelled = stop.clone();
        let (commands, mut command_receiver) = tokio::sync::mpsc::channel(32);
        let task = tokio::spawn(async move {
            let mut state = MediaState::default();
            loop {
                let event = tokio::select! {
                    _ = cancelled.cancelled() => break,
                    command = command_receiver.recv() => {
                        let Some(command) = command else { break; };
                        match command {
                            Command::RegisterOutbound { connection_id, credential_id, session_id, sip_leg_id, acknowledged } => {
                                state.credentials.insert(connection_id.clone(), credential_id);
                                state.sessions.insert(connection_id.clone(), session_id);
                                state.sip_legs.insert(connection_id, sip_leg_id);
                                let _ = acknowledged.send(());
                            }
                            Command::ActivateOutbound { connection_id, completed } => {
                                let result = bind_and_pair_connection(
                                    &orchestrator,
                                    &store,
                                    vapi.as_ref(),
                                    &mut state,
                                    &connection_id,
                                    false,
                                )
                                .await;
                                if let Err(reason) = result {
                                    tracing::warn!(%connection_id, reason, "outbound media connection could not become ready");
                                    let _ = orchestrator
                                        .end_connection(
                                            connection_id,
                                            EndReason::Failed { detail: reason.into() },
                                        )
                                        .await;
                                }
                                let _ = completed.send(result);
                            }
                        }
                        continue;
                    },
                    event = events.recv() => event,
                };
                match event {
                    Ok(Event::ConnectionInbound { connection_id, .. }) => {
                        if let Some(binding) = crate::media_ingress::admit(
                            &orchestrator,
                            &store,
                            &inbound_routes,
                            connection_id.clone(),
                            &namespace,
                        )
                        .await
                        {
                            state
                                .credentials
                                .insert(connection_id.clone(), binding.credential_id);
                            state
                                .sessions
                                .insert(connection_id.clone(), binding.session_id);
                            if binding.owns_session {
                                state.routed_connections.insert(connection_id);
                            }
                        }
                    }
                    Ok(Event::ConnectionPrincipalAuthenticated {
                        connection_id,
                        session_id,
                        participant_id,
                        principal,
                        ..
                    }) => {
                        let routed = state.routed_connections.contains(&connection_id);
                        if !routed && orchestrator.session_of(&connection_id).is_none() {
                            if let Some(session_id) = session_id.clone() {
                                if let Err(error) = orchestrator
                                    .route_inbound_connection(
                                        connection_id.clone(),
                                        InboundAction::Accept {
                                            session_id,
                                            participant_id: ParticipantId::from_string(
                                                participant_id,
                                            ),
                                        },
                                    )
                                    .await
                                {
                                    tracing::warn!(%error, "could not admit authenticated UCTP media leg");
                                    continue;
                                }
                            }
                        }
                        if let Some(durable_session) = session_id
                            .as_ref()
                            .and_then(|session| session.as_str().strip_prefix(&prefix))
                        {
                            state
                                .sessions
                                .insert(connection_id.clone(), durable_session.to_owned());
                        }
                        state
                            .credentials
                            .insert(connection_id.clone(), principal.subject);
                        bind_or_end_routed_connection(
                            &orchestrator,
                            &store,
                            vapi.as_ref(),
                            &mut state,
                            &connection_id,
                        )
                        .await;
                    }
                    Ok(Event::ConnectionConnected { connection_id, .. }) => {
                        // SIP publishes Connected before its staged outbound
                        // activation receipt enables media writes. The
                        // outbound worker explicitly activates this binding
                        // after `PreparedOutboundConnection::commit` returns.
                        if !state.sip_legs.contains_key(&connection_id) {
                            bind_or_end_routed_connection(
                                &orchestrator,
                                &store,
                                vapi.as_ref(),
                                &mut state,
                                &connection_id,
                            )
                            .await;
                        }
                    }
                    Ok(Event::ConnectionEnded { connection_id, .. }) => {
                        finish_connection(&orchestrator, &store, &mut state, &connection_id, false)
                            .await;
                    }
                    Ok(Event::ConnectionFailed {
                        connection_id,
                        detail,
                        ..
                    }) => {
                        tracing::warn!(%connection_id, %detail, "media connection failed");
                        finish_connection(&orchestrator, &store, &mut state, &connection_id, true)
                            .await;
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        tracing::warn!(
                            "media lifecycle observer lagged; durable state remains authoritative"
                        );
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            state.vapi.shutdown().await;
            for (connection_id, session_id) in state.sessions {
                let result = if state.routed_connections.contains(&connection_id) {
                    store
                        .end_routed_inbound_session(&session_id, "server-shutdown")
                        .map(|_| ())
                } else {
                    store
                        .unbind_session_connection(connection_id.as_str())
                        .map(|_| ())
                };
                if let Err(error) = result {
                    tracing::warn!(
                        code = error.code,
                        %session_id,
                        "could not persist media shutdown"
                    );
                }
            }
        });
        Ok(Self {
            stop,
            commands,
            task,
        })
    }

    pub(crate) fn registrar(&self) -> MediaRegistrar {
        MediaRegistrar {
            commands: self.commands.clone(),
        }
    }

    pub(crate) async fn shutdown(self) -> Result<()> {
        self.stop.cancel();
        self.task
            .await
            .map_err(|error| anyhow::anyhow!("media lifecycle task failed: {error}"))
    }
}

impl MediaRegistrar {
    pub(crate) async fn register_outbound(
        &self,
        connection_id: ConnectionId,
        credential_id: String,
        session_id: String,
        sip_leg_id: String,
    ) -> Result<()> {
        let (acknowledged, receiver) = tokio::sync::oneshot::channel();
        self.commands
            .send(Command::RegisterOutbound {
                connection_id,
                credential_id,
                session_id,
                sip_leg_id,
                acknowledged,
            })
            .await
            .map_err(|_| anyhow::anyhow!("media lifecycle is unavailable"))?;
        receiver
            .await
            .map_err(|_| anyhow::anyhow!("media lifecycle registration was not acknowledged"))
    }

    pub(crate) async fn activate_outbound(&self, connection_id: ConnectionId) -> Result<()> {
        let (completed, receiver) = tokio::sync::oneshot::channel();
        self.commands
            .send(Command::ActivateOutbound {
                connection_id,
                completed,
            })
            .await
            .map_err(|_| anyhow::anyhow!("media lifecycle is unavailable"))?;
        receiver
            .await
            .map_err(|_| anyhow::anyhow!("media lifecycle activation was not acknowledged"))?
            .map_err(|reason| anyhow::anyhow!(reason))
    }
}

async fn finish_connection(
    orchestrator: &Arc<Orchestrator>,
    store: &Arc<Store>,
    state: &mut MediaState,
    connection_id: &ConnectionId,
    failed: bool,
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
        if let Err(error) = store.finish_sip_connection(
            connection_id.as_str(),
            failed,
            if failed {
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

async fn bind_or_end_routed_connection(
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
        if state.sessions.contains_key(connection_id) {
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

async fn bind_and_pair_connection(
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
    if let Err(error) =
        store.bind_session_connection(credential, durable_session, connection_id.as_str())
    {
        tracing::warn!(code = error.code, "could not persist media readiness");
        return Err("media-binding-denied");
    }
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
        pair_connection(orchestrator, state, connection_id).await?;
    }
    Ok(())
}

async fn pair_connection(
    orchestrator: &Arc<Orchestrator>,
    state: &mut MediaState,
    connection_id: &ConnectionId,
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
            && orchestrator
                .connection_transport(candidate)
                .is_ok_and(|candidate_transport| candidate_transport != transport)
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
    match orchestrator
        .bridge_connections(peer.clone(), connection_id.clone())
        .await
    {
        Ok(_) => {
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
