//! Supervised media event worker; bindings own route pairing and cleanup.
mod bindings;
use anyhow::Result;
use bindings::{bind_and_pair_connection, bind_or_end_routed_connection, finish_connection};
use connect_store::{SipConnectionEnd, Store};
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
    time::Duration,
};
use tokio_util::sync::CancellationToken;

pub(crate) struct MediaLifecycle {
    stop: CancellationToken,
    commands: tokio::sync::mpsc::Sender<Command>,
    task: Option<tokio::task::JoinHandle<Result<()>>>,
}

#[derive(Clone)]
pub(crate) struct MediaRegistrar {
    commands: tokio::sync::mpsc::Sender<Command>,
}

enum Command {
    RegisterVapiSetup {
        session_id: String,
        setup: Box<crate::vapi_bootstrap::CallSetup>,
        completed: tokio::sync::oneshot::Sender<Result<(), Box<crate::vapi_bootstrap::CallSetup>>>,
    },
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
    video_bridges: HashMap<ConnectionId, rvoip_core::video_bridge::EncodedVideoBridge>,
    vapi: crate::vapi_media::VapiMedia,
}

impl MediaLifecycle {
    pub(crate) fn start(
        orchestrator: Arc<Orchestrator>,
        store: Arc<Store>,
        readiness: Arc<connect_service::readiness::OperationalReadiness>,
        vapi: Option<Arc<VapiAdapter>>,
        inbound_routes: crate::media_ingress::InboundMediaRoutes,
    ) -> Result<Self> {
        let namespace = store.installation_id()?;
        let prefix = format!("connect:{namespace}:");
        let mut events = orchestrator.subscribe_events();
        let stop = CancellationToken::new();
        let cancelled = stop.clone();
        let (commands, mut command_receiver) = tokio::sync::mpsc::channel(32);
        let task = crate::worker::spawn(readiness, "media-lifecycle", stop.clone(), async move {
            let mut state = MediaState::default();
            let mut expiry = tokio::time::interval(Duration::from_secs(1));
            expiry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                let event = tokio::select! {
                    _ = cancelled.cancelled() => break,
                    _ = expiry.tick() => {
                        state.vapi.expire_setups().await;
                        // Accepted durable Sessions survive socket loss. Explicit durable
                        // terminal state must still release an idle Core projection.
                        let projected: Vec<_> = orchestrator.externally_projected_session_ids()
                            .into_iter().filter_map(|id| id.as_str().strip_prefix(&prefix).map(str::to_owned)).collect();
                        match store.terminal_media_session_ids(&projected) {
                            Ok(terminal) => for session in terminal {
                                let id = rvoip_core::ids::SessionId::from_string(format!("{prefix}{session}"));
                                let _ = orchestrator.end_session(id, EndReason::Normal).await;
                            },
                            Err(error) => tracing::warn!(code = error.code, "could not reconcile durable terminal media projections"),
                        }
                        let denied: Vec<_> = state.sessions.iter().filter_map(|(connection, session)| {
                            let credential = state.credentials.get(connection)?;
                            let allowed = if credential.starts_with(connect_protocol::tasks::TASK_PRINCIPAL_PREFIX) {
                                store.authorize_native_task_session_binding(credential, session)
                            } else {
                                store.authorize_session_binding(credential, session)
                            };
                            let allowed = allowed.and_then(|value| {
                                if value.media == ["audio", "video"]
                                    && orchestrator.connection_transport(connection).is_ok_and(|transport|transport == rvoip_core::connection::Transport::WebRtc)
                                {
                                    let route = inbound_routes.webrtc.as_ref().ok_or_else(connect_protocol::Error::forbidden)?;
                                    store.authorize_selected_browser_video(credential,
                                        &crate::media_ingress::browser_route(route), &value.conversation_id, &value.id)
                                        .map(|binding|binding.session)
                                } else {Ok(value)}
                            });
                            allowed.err().map(|_| connection.clone())
                        }).collect();
                        for connection in denied {
                            let _ = orchestrator.end_connection(connection, EndReason::Failed { detail: "media-authority-ended".into() }).await;
                        }
                        continue;
                    },
                    command = command_receiver.recv() => {
                        let Some(command) = command else { break; };
                        match command {
                            Command::RegisterVapiSetup {session_id, setup, completed} => {
                                let result = state.vapi.install_task_setup(session_id, setup);
                                let _ = completed.send(result);
                            }
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
                // Stream/bridge discovery can wait while an offered peer has no
                // ICE media yet. Cancel that owned preparation before draining
                // bindings; waiting only between events strands shutdown behind it.
                if matches!(
                    &event,
                    Err(tokio::sync::broadcast::error::RecvError::Closed)
                ) {
                    break;
                }
                let handle_event = async {
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
                                        return;
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
                        Ok(Event::ConnectionEnded {
                            connection_id,
                            reason,
                            ..
                        }) => {
                            let end = if matches!(reason, EndReason::Cancelled) {
                                SipConnectionEnd::Cancelled
                            } else {
                                SipConnectionEnd::Ended
                            };
                            finish_connection(
                                &orchestrator,
                                &store,
                                &mut state,
                                &connection_id,
                                end,
                            )
                            .await;
                        }
                        Ok(Event::ConnectionFailed {
                            connection_id,
                            detail,
                            ..
                        }) => {
                            tracing::warn!(%connection_id, %detail, "media connection failed");
                            finish_connection(
                                &orchestrator,
                                &store,
                                &mut state,
                                &connection_id,
                                SipConnectionEnd::Failed,
                            )
                            .await;
                        }
                        Ok(_) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            tracing::warn!(
                            "media lifecycle observer lagged; durable state remains authoritative"
                        );
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {}
                    }
                };
                tokio::select! {
                    biased;
                    _ = cancelled.cancelled() => break,
                    _ = handle_event => {}
                }
            }
            let vapi_shutdown = state.vapi.shutdown().await;
            for (_, bridge) in state.video_bridges.drain() {
                bridge.shutdown().await;
            }
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
            vapi_shutdown
        });
        Ok(Self {
            stop,
            commands,
            task: Some(task),
        })
    }

    pub(crate) fn registrar(&self) -> MediaRegistrar {
        MediaRegistrar {
            commands: self.commands.clone(),
        }
    }

    pub(crate) async fn shutdown(mut self) -> Result<()> {
        self.stop.cancel();
        crate::worker::drain(
            self.task.take().into_iter().collect(),
            std::time::Duration::from_secs(5),
        )
        .await
    }
}
impl Drop for MediaLifecycle {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl MediaRegistrar {
    pub(crate) async fn register_vapi_setup(
        &self,
        session_id: String,
        setup: Box<crate::vapi_bootstrap::CallSetup>,
    ) -> Result<Result<(), Box<crate::vapi_bootstrap::CallSetup>>> {
        let (completed, receiver) = tokio::sync::oneshot::channel();
        self.commands
            .send(Command::RegisterVapiSetup {
                session_id,
                setup,
                completed,
            })
            .await
            .map_err(|_| anyhow::anyhow!("Media lifecycle unavailable; retain setup intent"))?;
        receiver
            .await
            .map_err(|_| anyhow::anyhow!("Setup acknowledgement lost; retain original intent"))
    }
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
