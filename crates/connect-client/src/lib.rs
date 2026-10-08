//! Native Connect SDK. TLS verification is mandatory and logical IDs belong to callers.
use connect_protocol::{
    Error, Operation, Outcome, Request, Response, PROFILE, REQUEST_TYPE, RESULT_TYPE,
};
use rvoip_quic::UctpQuicClient;
use rvoip_uctp::{envelope::UctpEnvelope, substrate, types::MessageType};
use serde::{Deserialize, Serialize};
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientConfig {
    pub endpoint: SocketAddr,
    pub server_name: String,
    pub trust_certificate: PathBuf,
    pub credential: String,
    pub identity_id: String,
    pub credential_id: String,
    #[serde(default = "default_deadline")]
    pub timeout_ms: u64,
}
fn default_deadline() -> u64 {
    10_000
}
impl ClientConfig {
    pub fn load(path: &Path) -> std::result::Result<Self, ClientError> {
        let content = std::fs::read_to_string(path).map_err(|_| ClientError::Configuration)?;
        let mut config: Self = toml::from_str(&content).map_err(|_| ClientError::Configuration)?;
        if config.trust_certificate.is_relative() {
            config.trust_certificate = path
                .parent()
                .unwrap_or(Path::new("."))
                .join(&config.trust_certificate);
        }
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<()> {
        if !self.endpoint.ip().is_loopback()
            || self.timeout_ms == 0
            || self.timeout_ms > 60_000
            || self.server_name.is_empty()
            || self.credential.len() != 64
        {
            return Err(ClientError::Configuration);
        }
        Ok(())
    }
}
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("Client configuration or trust material is invalid")]
    Configuration,
    #[error("Native transport disconnected or failed TLS verification")]
    Transport,
    #[error("Request deadline expired; acceptance is unknown; preserve the logical request ID")]
    Timeout,
    #[error("Server rejected authentication or the required profile")]
    Authentication,
    #[error("Server response violates the negotiated contract")]
    Protocol,
    #[error(transparent)]
    Application(#[from] Error),
}
pub type Result<T> = std::result::Result<T, ClientError>;

pub struct ConnectClient {
    endpoint: quinn::Endpoint,
    peer: Arc<UctpQuicClient>,
    inbox: tokio::sync::mpsc::Receiver<UctpEnvelope>,
    deadline: Duration,
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
        Ok(client)
    }
    async fn exchange(&mut self, envelope: UctpEnvelope) -> Result<UctpEnvelope> {
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
        tokio::time::timeout(self.deadline, future)
            .await
            .map_err(|_| ClientError::Timeout)?
    }
    pub async fn request(&mut self, operation: Operation) -> Result<Outcome> {
        let envelope = UctpEnvelope::new(
            MessageType::Unknown(REQUEST_TYPE.into()),
            serde_json::to_value(Request::new(operation)).map_err(|_| ClientError::Protocol)?,
        );
        let reply = self.exchange(envelope).await?;
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
