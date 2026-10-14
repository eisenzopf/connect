//! Native UCTP/QUIC exchange lifecycle. One request at a time per client.
use crate::{ClientConfig, ClientError, Result};
use connect_protocol::{
    tasks, Operation, Outcome, Request, Response, PROFILE, REQUEST_TYPE, RESULT_TYPE,
};
use rvoip_quic::UctpQuicClient;
use rvoip_uctp::{envelope::UctpEnvelope, substrate, types::MessageType};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::{broadcast, oneshot};

pub struct ConnectClient {
    endpoint: quinn::Endpoint,
    pub(crate) peer: Arc<UctpQuicClient>,
    pending: Arc<parking_lot::Mutex<HashMap<String, oneshot::Sender<UctpEnvelope>>>>,
    pub(crate) events: broadcast::Sender<UctpEnvelope>,
    pub(crate) deadline: Duration,
    pub(crate) participant_id: String,
    pub(crate) identity_id: String,
    pub(crate) media_router: Arc<rvoip_uctp::substrate::PeerMediaRouter>,
    disconnected: bool,
    task_profile: bool,
    authenticated_until: Option<chrono::DateTime<chrono::Utc>>,
}
impl ConnectClient {
    pub async fn open(config: ClientConfig) -> Result<Self> {
        config.validate()?;
        let _ = rustls::crypto::ring::default_provider().install_default();
        let cert = rustls::pki_types::CertificateDer::from(
            std::fs::read(&config.trust_certificate).map_err(|_| ClientError::Configuration)?,
        );
        let tls = Arc::new(
            substrate::dev_client_config_trusting(&cert).map_err(|_| ClientError::Configuration)?,
        );
        let endpoint = substrate::make_client_endpoint(
            (if config.endpoint.is_ipv6() {
                "[::1]:0"
            } else {
                "127.0.0.1:0"
            })
            .parse()
            .map_err(|_| ClientError::Configuration)?,
            tls.clone(),
        )
        .map_err(|_| ClientError::Transport)?;
        // Bound the physical peer's shared DATAGRAM byte queue independently of
        // the audio/video application packet queues. The encoded-video opt-in
        // also adjusts connection-wide loss detection, including reliable control;
        // application authorization and operation deadlines remain unchanged.
        let mut transport = quinn::TransportConfig::default();
        transport.max_idle_timeout(Some(
            Duration::from_secs(30)
                .try_into()
                .map_err(|_| ClientError::Configuration)?,
        ));
        rvoip_quic::apply_encoded_video_transport_budget(&mut transport);
        let deadline = Duration::from_millis(config.timeout_ms);
        let peer = tokio::time::timeout(
            deadline,
            UctpQuicClient::connect_with_transport(
                &endpoint,
                config.endpoint,
                &config.server_name,
                tls,
                transport,
            ),
        )
        .await
        .map_err(|_| ClientError::Timeout)?
        .map_err(|_| ClientError::Transport)?;
        let mut inbox = peer.take_inbound().ok_or(ClientError::Transport)?;
        let pending = Arc::new(parking_lot::Mutex::new(HashMap::<
            String,
            oneshot::Sender<UctpEnvelope>,
        >::new()));
        let (events, _) = broadcast::channel(256);
        let pending_for_reader = pending.clone();
        let events_for_reader = events.clone();
        tokio::spawn(async move {
            while let Some(envelope) = inbox.recv().await {
                tracing::debug!(message_type = %envelope.msg_type.as_wire_str(), "Connect client received UCTP envelope");
                let waiter = envelope
                    .in_reply_to
                    .as_ref()
                    .and_then(|id| pending_for_reader.lock().remove(id));
                if let Some(waiter) = waiter {
                    let _ = waiter.send(envelope);
                } else {
                    let _ = events_for_reader.send(envelope);
                }
            }
        });
        let media_router = rvoip_uctp::substrate::PeerMediaRouter::new();
        let mut client = Self {
            endpoint,
            peer,
            pending,
            events,
            deadline,
            participant_id: String::new(),
            identity_id: config.identity_id.clone(),
            media_router,
            disconnected: false,
            task_profile: false,
            authenticated_until: None,
        };
        let hello = UctpEnvelope::new(
            MessageType::AuthHello,
            serde_json::json!({"device":{"id":"connect-sdk","kind":"desktop","platform":std::env::consts::OS,"sdk_version":env!("CARGO_PKG_VERSION")},"auth_methods":["bearer"],"capabilities":{}}),
        );
        let challenge = client.exchange(hello).await?;
        if challenge.msg_type != MessageType::AuthChallenge
            || !challenge.payload["server_capabilities"]["application_profiles"]
                .as_array()
                .is_some_and(|v| v.iter().any(|p| p == PROFILE))
        {
            return Err(ClientError::Authentication);
        }
        let response = client
            .exchange(UctpEnvelope::new(
                MessageType::AuthResponse,
                serde_json::json!({"method":"bearer","credential":config.credential}),
            ))
            .await?;
        if response.msg_type != MessageType::AuthSession {
            return Err(ClientError::Authentication);
        }
        let session: rvoip_uctp::payloads::auth::AuthSession = response
            .decode_payload()
            .map_err(|_| ClientError::Authentication)?;
        if session.identity_id != config.credential_id || session.expires_at <= chrono::Utc::now() {
            return Err(ClientError::Authentication);
        }
        client.participant_id = session.participant_id;
        client.authenticated_until = Some(session.expires_at);
        std::mem::drop(rvoip_quic::spawn_datagram_reader_with_cancel(
            client.peer.connection.clone(),
            client.media_router.clone(),
            None,
            tokio_util::sync::CancellationToken::new(),
        ));
        Ok(client)
    }
    pub(crate) async fn exchange(&mut self, envelope: UctpEnvelope) -> Result<UctpEnvelope> {
        self.exchange_with_deadline(envelope, self.deadline).await
    }
    async fn exchange_with_deadline(
        &mut self,
        envelope: UctpEnvelope,
        deadline: Duration,
    ) -> Result<UctpEnvelope> {
        if self.disconnected {
            return Err(ClientError::Transport);
        }
        // The trusted auth.session supplies this expiry. Reject before sending;
        // expiry during an exchange must not turn unknown acceptance into denial.
        if self
            .authenticated_until
            .is_some_and(|expires_at| expires_at <= chrono::Utc::now())
        {
            return Err(ClientError::Authentication);
        }
        let id = envelope.id.clone();
        let (reply_tx, reply_rx) = oneshot::channel();
        self.pending.lock().insert(id.clone(), reply_tx);
        let future = async {
            if self.peer.send(envelope).await.is_err() {
                self.pending.lock().remove(&id);
                return Err(ClientError::Transport);
            }
            reply_rx.await.map_err(|_| ClientError::Transport)
        };
        match tokio::time::timeout(deadline, future).await {
            Ok(result) => result,
            Err(_) => {
                self.pending.lock().remove(&id);
                self.disconnect();
                Err(ClientError::Timeout)
            }
        }
    }
    pub(crate) fn disconnect(&mut self) {
        self.disconnected = true;
        self.peer
            .connection
            .close(0u32.into(), b"exchange cancelled or timed out");
        self.endpoint
            .close(0u32.into(), b"exchange cancelled or timed out");
    }
    pub async fn request(&mut self, operation: Operation) -> Result<Outcome> {
        self.request_inner(operation, false).await
    }
    pub(crate) async fn bootstrap_task(&mut self, task_id: &str) -> Result<tasks::TaskRecord> {
        tasks::validate_task_identifier(task_id).map_err(ClientError::Application)?;
        let outcome = self
            .request_inner(
                Operation::Task {
                    request: Box::new(tasks::TaskRequest::new(tasks::TaskCommand::Get {
                        task_id: task_id.into(),
                    })),
                },
                true,
            )
            .await?;
        let Outcome::Task(outcome) = outcome else {
            return Err(ClientError::Protocol);
        };
        let tasks::TaskOutcome::Record(record) = *outcome else {
            return Err(ClientError::Protocol);
        };
        if record.id != task_id {
            return Err(ClientError::Protocol);
        }
        self.task_profile = true;
        Ok(*record)
    }
    async fn request_inner(&mut self, operation: Operation, bootstrap: bool) -> Result<Outcome> {
        let wait = if let Operation::Task { request } = &operation {
            if !self.task_profile && !bootstrap {
                return Err(ClientError::Application(connect_protocol::Error::new(
                    "unsupported-profile",
                    "Discover connect-task/1 through capabilities before task requests",
                )));
            }
            // A valid server wait gets its bounded wait plus the configured
            // exchange budget. Invalid input cannot expand this to an unbounded wait.
            request
                .command
                .wait_ms()
                .unwrap_or(0)
                .min(tasks::MAX_TOOL_WAIT_MS)
        } else {
            0
        };
        let cid = operation.conversation_id().map(str::to_owned);
        let mut envelope = UctpEnvelope::new(
            MessageType::Unknown(REQUEST_TYPE.into()),
            serde_json::to_value(Request::new(operation)).map_err(|_| ClientError::Protocol)?,
        );
        envelope.cid = cid.clone();
        let reply = self
            .exchange_with_deadline(
                envelope,
                self.deadline
                    .saturating_add(Duration::from_millis(u64::from(wait))),
            )
            .await?;
        if let (Some(expected), Some(actual)) = (cid.as_deref(), reply.cid.as_deref()) {
            if expected != actual {
                return Err(ClientError::Protocol);
            }
        }
        if reply.msg_type == MessageType::Error {
            // Envelope authorization can reject a previously authenticated
            // task peer before the application handler creates a Response.
            // Preserve that typed denial instead of misclassifying it as a
            // failed initial handshake. Never copy the raw reason to callers.
            return Err(match reply.payload.get("code").and_then(|v| v.as_u64()) {
                Some(403) => ClientError::Application(connect_protocol::Error::forbidden()),
                Some(401) => ClientError::Authentication,
                Some(503) => ClientError::Application(connect_protocol::Error::storage()),
                _ => ClientError::Protocol,
            });
        }
        if reply.msg_type != MessageType::Unknown(RESULT_TYPE.into()) {
            return Err(ClientError::Protocol);
        }
        let response: Response = reply.decode_payload().map_err(|_| ClientError::Protocol)?;
        if response.profile != PROFILE {
            return Err(ClientError::Protocol);
        }
        let outcome = response.result.map_err(ClientError::Application)?;
        if let Outcome::Capabilities(capabilities) = &outcome {
            self.task_profile = capabilities
                .extensions
                .iter()
                .any(|p| p == tasks::TASK_PROFILE);
        }
        Ok(outcome)
    }
    pub async fn resolve(&mut self, address: &str) -> Result<connect_protocol::Identity> {
        match self
            .request(Operation::Resolve {
                address: address.into(),
            })
            .await?
        {
            Outcome::Identity(i) => Ok(i),
            _ => Err(ClientError::Protocol),
        }
    }
    pub async fn close(self) {
        let path = self.peer.connection.stats().path;
        tracing::debug!(
            rtt_ms = path.rtt.as_millis(),
            min_rtt_ms = path.min_rtt.as_millis(),
            "Closing native client transport"
        );
        self.peer.connection.close(0u32.into(), b"client shutdown");
        self.endpoint.close(0u32.into(), b"client shutdown");
        if tokio::time::timeout(Duration::from_secs(2), self.endpoint.wait_idle())
            .await
            .is_err()
        {
            tracing::warn!(
                open_connections = self.endpoint.open_connections(),
                rtt_ms = path.rtt.as_millis(),
                "Native client transport did not finish its protocol drain within two seconds"
            );
        }
    }
}
impl Drop for ConnectClient {
    fn drop(&mut self) {
        self.peer.connection.close(0u32.into(), b"client dropped");
        self.endpoint.close(0u32.into(), b"client dropped");
    }
}
