//! Native UCTP/QUIC exchange lifecycle. One request at a time per client.
use crate::{ClientConfig, ClientError, Result};
use connect_protocol::{Operation, Outcome, Request, Response, PROFILE, REQUEST_TYPE, RESULT_TYPE};
use rvoip_quic::UctpQuicClient;
use rvoip_uctp::{envelope::UctpEnvelope, substrate, types::MessageType};
use std::{sync::Arc, time::Duration};

pub struct ConnectClient {
    endpoint: quinn::Endpoint,
    peer: Arc<UctpQuicClient>,
    inbox: tokio::sync::mpsc::Receiver<UctpEnvelope>,
    deadline: Duration,
    disconnected: bool,
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
        let deadline = Duration::from_millis(config.timeout_ms);
        let peer = tokio::time::timeout(
            deadline,
            UctpQuicClient::connect(&endpoint, config.endpoint, &config.server_name, tls),
        )
        .await
        .map_err(|_| ClientError::Timeout)?
        .map_err(|_| ClientError::Transport)?;
        let inbox = peer.take_inbound().ok_or(ClientError::Transport)?;
        let mut client = Self {
            endpoint,
            peer,
            inbox,
            deadline,
            disconnected: false,
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
        if response.msg_type != MessageType::AuthSession
            || response.payload["identity_id"] != config.credential_id
        {
            return Err(ClientError::Authentication);
        }
        Ok(client)
    }
    async fn exchange(&mut self, envelope: UctpEnvelope) -> Result<UctpEnvelope> {
        if self.disconnected {
            return Err(ClientError::Transport);
        }
        let id = envelope.id.clone();
        let future = async {
            self.peer
                .send(envelope)
                .await
                .map_err(|_| ClientError::Transport)?;
            while let Some(reply) = self.inbox.recv().await {
                if reply.in_reply_to.as_ref() == Some(&id) {
                    return Ok(reply);
                }
            }
            Err(ClientError::Transport)
        };
        match tokio::time::timeout(self.deadline, future).await {
            Ok(result) => result,
            Err(_) => {
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
        let cid = operation.conversation_id().map(str::to_owned);
        let mut envelope = UctpEnvelope::new(
            MessageType::Unknown(REQUEST_TYPE.into()),
            serde_json::to_value(Request::new(operation)).map_err(|_| ClientError::Protocol)?,
        );
        envelope.cid = cid.clone();
        let reply = self.exchange(envelope).await?;
        if let (Some(expected), Some(actual)) = (cid.as_deref(), reply.cid.as_deref()) {
            if expected != actual {
                return Err(ClientError::Protocol);
            }
        }
        if reply.msg_type == MessageType::Error {
            return Err(ClientError::Authentication);
        }
        if reply.msg_type != MessageType::Unknown(RESULT_TYPE.into()) {
            return Err(ClientError::Protocol);
        }
        let response: Response = reply.decode_payload().map_err(|_| ClientError::Protocol)?;
        if response.profile != PROFILE {
            return Err(ClientError::Protocol);
        }
        response.result.map_err(ClientError::Application)
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
        self.peer.connection.close(0u32.into(), b"client shutdown");
        self.endpoint.close(0u32.into(), b"client shutdown");
        let _ = tokio::time::timeout(Duration::from_secs(2), self.endpoint.wait_idle()).await;
    }
}
impl Drop for ConnectClient {
    fn drop(&mut self) {
        self.peer.connection.close(0u32.into(), b"client dropped");
        self.endpoint.close(0u32.into(), b"client dropped");
    }
}
