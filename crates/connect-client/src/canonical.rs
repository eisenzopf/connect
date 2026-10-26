//! Signed canonical connect/2 on one TLS-authenticated physical QUIC peer.
//! Callers retain logical operation IDs and correlate verified replies/events.
pub use crate::NativeNetworkProfile;
use crate::{ClientError, Result};
use rvoip_auth_core::sig9421::{
    ResolvedVerificationKey, Sig9421Signer, Sig9421Verifier, SignatureVerificationContext,
    StaticKeyResolver,
};
/// Canonical native RTP and clock-only wire helpers; metadata is not admission.
pub use rvoip_uctp::substrate::{
    observe_native_media_datagram, pack_sender_clock_datagram, ObservedNativeMediaDatagram,
    SenderClockDatagram, SenderClockReport,
};
pub use rvoip_uctp::v2::{payloads, Envelope, Identifier, Payload, Scope};
use rvoip_uctp::v2::{MAX_ENVELOPE_BYTES, RAW_QUIC_ALPN};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalClientConfig {
    #[serde(default)]
    pub network_profile: NativeNetworkProfile,
    pub endpoint: SocketAddr,
    pub server_name: String,
    pub trust_certificate: PathBuf,
    pub credential: String,
    pub identity_id: String,
    pub device_id: String,
    pub device_kind: String,
    pub device_platform: String,
    pub signing_key_id: String,
    pub signing_algorithm: String,
    pub signing_private_key: PathBuf,
    pub server_key_id: String,
    pub server_algorithm: String,
    /// Raw Ed25519 or uncompressed SEC1 P-256 public key; never a TLS key.
    pub server_public_key: PathBuf,
    pub server_key_not_before: chrono::DateTime<chrono::Utc>,
    pub server_key_expires_at: chrono::DateTime<chrono::Utc>,
    pub timeout_ms: u64,
}
impl CanonicalClientConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path).map_err(|_| ClientError::Configuration)?;
        let mut value: Self = toml::from_str(&raw).map_err(|_| ClientError::Configuration)?;
        for p in [
            &mut value.trust_certificate,
            &mut value.signing_private_key,
            &mut value.server_public_key,
        ] {
            if p.is_relative() {
                *p = path.parent().unwrap_or(Path::new(".")).join(&*p);
            }
        }
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<()> {
        let now = chrono::Utc::now();
        if !self.network_profile.permits_endpoint(self.endpoint)
            || self.server_name.is_empty()
            || self.server_name.len() > 253
            || self.timeout_ms == 0
            || self.timeout_ms > 60_000
            || self.credential.is_empty()
            || self.credential.len() > 256
            || !self.credential.is_ascii()
            || self.server_key_not_before > now
            || self.server_key_expires_at <= now
            || self.server_key_not_before >= self.server_key_expires_at
            || !matches!(self.signing_algorithm.as_str(), "EdDSA" | "ES256")
            || !matches!(self.server_algorithm.as_str(), "EdDSA" | "ES256")
            || self.device_kind.is_empty()
            || self.device_kind.len() > 64
            || self.device_platform.is_empty()
            || self.device_platform.len() > 128
        {
            return Err(ClientError::Configuration);
        }
        rustls::pki_types::ServerName::try_from(self.server_name.clone())
            .map_err(|_| ClientError::Configuration)?;
        for id in [
            &self.identity_id,
            &self.device_id,
            &self.signing_key_id,
            &self.server_key_id,
        ] {
            Identifier::try_from(id.to_string()).map_err(|_| ClientError::Configuration)?;
        }
        Ok(())
    }
}
/// Scope assertions are checked by both the wire contract and current server policy.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandScope {
    pub conversation_id: Option<String>,
    pub session_id: Option<String>,
    pub connection_id: Option<String>,
    pub operation_id: Option<String>,
    pub expected_revision: Option<u64>,
}
/// Caller intent only. The SDK supplies a fresh envelope ID, timestamp and signature.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub command: Payload,
    #[serde(default)]
    pub scope: CommandScope,
}

/// Current authenticated peer metadata. Opaque bearer/session secrets are not
/// returned, serialized or included in diagnostics.
#[derive(Clone, Debug)]
pub struct CanonicalAuthentication {
    pub identity_id: String,
    pub assurance: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub application_profile: String,
}
pub struct CanonicalClient {
    network_profile: NativeNetworkProfile,
    server_address: SocketAddr,
    endpoint: quinn::Endpoint,
    connection: quinn::Connection,
    writer: quinn::SendStream,
    reader: Option<quinn::RecvStream>,
    signer: Sig9421Signer,
    verifier: Arc<Sig9421Verifier>,
    verification: SignatureVerificationContext,
    deadline: Duration,
    closed: Arc<AtomicBool>,
    authenticated_until: chrono::DateTime<chrono::Utc>,
    session_token: String,
    record_download: records::RecordDownload,
    identity_id: String,
    assurance: String,
    step_up: Option<step_up::Challenge>,
    events: VecDeque<Envelope>,
    event_bytes: usize,
    server_key_expires_at: chrono::DateTime<chrono::Utc>,
    owned_signaling: Option<owned_signaling::OwnedSignaling>,
}
struct Pending {
    connection: quinn::Connection,
    closed: Arc<AtomicBool>,
    complete: bool,
}
impl Drop for Pending {
    fn drop(&mut self) {
        if !self.complete {
            self.closed.store(true, Ordering::Release);
            self.connection
                .close(0u32.into(), b"canonical exchange interrupted");
        }
    }
}
impl CanonicalClient {
    pub async fn open(config: CanonicalClientConfig) -> Result<Self> {
        config.validate()?;
        if config.endpoint.port() == 0 {
            return Err(ClientError::Configuration);
        }
        let deadline = Duration::from_millis(config.timeout_ms);
        tokio::time::timeout(deadline, Self::open_inner(config))
            .await
            .map_err(|_| ClientError::Timeout)?
    }
    async fn open_inner(mut config: CanonicalClientConfig) -> Result<Self> {
        config.endpoint = NativeNetworkProfile::canonical_address(config.endpoint);
        let _ = rustls::crypto::ring::default_provider().install_default();
        let secret = bounded_file(&config.signing_private_key, 16_384)?;
        let signer = match config.signing_algorithm.as_str() {
            "EdDSA" => Sig9421Signer::ed25519(&config.signing_key_id, &secret),
            "ES256" => Sig9421Signer::es256(&config.signing_key_id, &secret),
            _ => return Err(ClientError::Configuration),
        }
        .map_err(|_| ClientError::Configuration)?;
        let public = bounded_file(&config.server_public_key, 128)?;
        if (config.server_algorithm == "EdDSA" && public.len() != 32)
            || (config.server_algorithm == "ES256" && (public.len() != 65 || public[0] != 4))
        {
            return Err(ClientError::Configuration);
        }
        // A distinct pinned server domain, never a received Identity assertion.
        let verification =
            SignatureVerificationContext::new("connect-server", config.server_key_id.clone())
                .map_err(|_| ClientError::Configuration)?;
        let mut keys = StaticKeyResolver::new();
        keys.insert_contextual(
            &verification,
            &config.server_key_id,
            ResolvedVerificationKey {
                public_key: public,
                algorithm: config.server_algorithm.clone(),
                not_before: Some(config.server_key_not_before),
                expires_at: Some(config.server_key_expires_at),
                revoked: false,
            },
        );
        let verifier = Arc::new(Sig9421Verifier::new(Arc::new(keys)));
        let trust_bytes = bounded_file(&config.trust_certificate, trust::MAX_TRUST_BYTES)?;
        let mut tls = trust::client_config(&trust_bytes)?;
        tls.alpn_protocols = vec![RAW_QUIC_ALPN.to_vec()];
        let mut endpoint = quinn::Endpoint::client(
            config
                .network_profile
                .client_bind(config.endpoint.is_ipv6()),
        )
        .map_err(|_| ClientError::Transport)?;
        let mut transport = quinn::TransportConfig::default();
        transport.max_concurrent_bidi_streams(0u32.into());
        transport.max_concurrent_uni_streams(0u32.into());
        // Negotiate the bounded native media substrate. Actual routes still
        // require the signed offer/answer and current media authority.
        transport.datagram_receive_buffer_size(Some(64 * 1024));
        transport.datagram_send_buffer_size(64 * 1024);
        transport.max_idle_timeout(Some(
            Duration::from_secs(30)
                .try_into()
                .map_err(|_| ClientError::Configuration)?,
        ));
        let mut client_config = quinn::ClientConfig::new(Arc::new(
            quinn::crypto::rustls::QuicClientConfig::try_from(tls)
                .map_err(|_| ClientError::Configuration)?,
        ));
        client_config.transport_config(Arc::new(transport));
        endpoint.set_default_client_config(client_config);
        let deadline = Duration::from_millis(config.timeout_ms);
        let connecting = endpoint
            .connect(config.endpoint, &config.server_name)
            .map_err(|_| ClientError::Transport)?;
        let connected = tokio::time::timeout(deadline, connecting)
            .await
            .map_err(|_| ClientError::Timeout)?
            .map_err(|_| ClientError::Transport)?;
        let closed = Arc::new(AtomicBool::new(false));
        let mut opening = Pending {
            connection: connected.clone(),
            closed: closed.clone(),
            complete: false,
        };
        let (writer, reader) = tokio::time::timeout(deadline, connected.open_bi())
            .await
            .map_err(|_| ClientError::Timeout)?
            .map_err(|_| ClientError::Transport)?;
        let mut client = Self {
            network_profile: config.network_profile,
            server_address: config.endpoint,
            endpoint,
            connection: connected,
            writer,
            reader: Some(reader),
            signer,
            verifier,
            verification,
            deadline,
            closed,
            authenticated_until: config.server_key_expires_at,
            session_token: String::new(),
            record_download: records::RecordDownload::new(
                config.endpoint.ip(),
                &config.server_name,
                &trust_bytes,
                config.credential.clone(),
                config.network_profile,
            )?,
            identity_id: config.identity_id.clone(),
            assurance: "identified".into(),
            step_up: None,
            events: VecDeque::new(),
            event_bytes: 0,
            server_key_expires_at: config.server_key_expires_at,
            owned_signaling: None,
        };
        let hello = envelope(
            "auth.hello",
            json!({"device":{"id":config.device_id,"kind":config.device_kind,"platform":config.device_platform,"sdk_version":env!("CARGO_PKG_VERSION")},"auth_methods":["bearer"],"application_profiles":["connect/2"]}),
        );
        let hello_id = hello["id"]
            .as_str()
            .ok_or(ClientError::Protocol)?
            .to_owned();
        client.write(hello, false).await?;
        let challenge = client.receive().await?;
        if challenge.kind() != "auth.challenge"
            || challenge.original()["in_reply_to"] != hello_id
            || !challenge.original()["payload"]["accepted_methods"]
                .as_array()
                .is_some_and(|m| m.iter().any(|v| v == "bearer"))
            || !challenge.original()["payload"]["server_capabilities"]["application_profiles"]
                .as_array()
                .is_some_and(|m| m.iter().any(|v| v == "connect/2"))
        {
            return Err(ClientError::Authentication);
        }
        let mut response = envelope(
            "auth.response",
            json!({"method":"bearer","credential":config.credential}),
        );
        response["in_reply_to"] = json!(challenge.id());
        let response_id = response["id"]
            .as_str()
            .ok_or(ClientError::Protocol)?
            .to_owned();
        client.write(response, true).await?;
        let session = client.receive().await?;
        let expires = chrono::DateTime::parse_from_rfc3339(
            session.original()["payload"]["expires_at"]
                .as_str()
                .ok_or(ClientError::Authentication)?,
        )
        .map_err(|_| ClientError::Authentication)?
        .with_timezone(&chrono::Utc);
        if session.kind() != "auth.session"
            || session.original()["in_reply_to"] != response_id
            || session.original()["payload"]["identity_id"] != config.identity_id
            || session.original()["payload"]["application_profile"] != "connect/2"
            || !matches!(
                session.original()["payload"]["assurance"].as_str(),
                Some("identified" | "user-authorized")
            )
            || expires <= chrono::Utc::now()
        {
            return Err(ClientError::Authentication);
        }
        client.session_token = session.original()["payload"]["session_token"]
            .as_str()
            .ok_or(ClientError::Authentication)?
            .to_owned();
        client.authenticated_until = expires.min(config.server_key_expires_at);
        client.assurance = session.original()["payload"]["assurance"]
            .as_str()
            .ok_or(ClientError::Authentication)?
            .to_owned();
        opening.complete = true;
        Ok(client)
    }
    fn check_current(&self) -> Result<()> {
        if self.closed.load(Ordering::Acquire) || self.connection.close_reason().is_some() {
            return Err(ClientError::Transport);
        }
        if chrono::Utc::now() >= self.authenticated_until
            || chrono::Utc::now() >= self.server_key_expires_at
        {
            self.connection
                .close(0u32.into(), b"canonical authority expired");
            self.closed.store(true, Ordering::Release);
            return Err(ClientError::Authentication);
        }
        Ok(())
    }
    /// Local lifetime of the already authenticated current native profile. This
    /// does not discover routes, refresh authority or create media ownership.
    pub(crate) fn sdk_authority_expires(&self) -> Result<chrono::DateTime<chrono::Utc>> {
        self.check_current()?;
        Ok(self.authenticated_until.min(self.server_key_expires_at))
    }
    fn pending(&self) -> Result<Pending> {
        self.check_current()?;
        Ok(Pending {
            connection: self.connection.clone(),
            closed: self.closed.clone(),
            complete: false,
        })
    }
    async fn write(&mut self, value: Value, signed: bool) -> Result<()> {
        let value = if signed {
            self.signer
                .sign(&value)
                .map_err(|_| ClientError::Configuration)?
        } else {
            value
        };
        let checked = Envelope::from_value(value).map_err(|_| ClientError::Protocol)?;
        let raw = serde_json::to_vec(checked.original()).map_err(|_| ClientError::Protocol)?;
        let mut pending = self.pending()?;
        tokio::time::timeout(self.deadline, async {
            self.writer
                .write_all(&(raw.len() as u32).to_be_bytes())
                .await
                .map_err(|_| ClientError::Transport)?;
            self.writer
                .write_all(&raw)
                .await
                .map_err(|_| ClientError::Transport)?;
            Ok::<(), ClientError>(())
        })
        .await
        .map_err(|_| ClientError::Timeout)??;
        pending.complete = true;
        Ok(())
    }
    /// Sends one typed command with a fresh envelope ID/timestamp. Persist the
    /// operation ID and intent before calling; successful send is not acceptance.
    pub async fn send(&mut self, payload: Payload, scope: CommandScope) -> Result<String> {
        let value = command(payload, scope)?;
        let id = value["id"]
            .as_str()
            .ok_or(ClientError::Protocol)?
            .to_owned();
        self.write(value, true).await?;
        Ok(id)
    }
    /// Send a validated command payload while preserving optional extension
    /// fields and array order in its signed original. Known required fields,
    /// scopes, protocol bounds and the normal authentication path still apply.
    pub async fn send_value(
        &mut self,
        kind: &str,
        payload: Value,
        scope: CommandScope,
    ) -> Result<String> {
        let value = command_value(kind, payload, scope)?;
        let id = value["id"]
            .as_str()
            .ok_or(ClientError::Protocol)?
            .to_owned();
        self.write(value, true).await?;
        Ok(id)
    }
    /// Refresh this exact signed peer with an explicitly supplied successor
    /// bearer. No credential is issued or chosen automatically. A verified
    /// refusal preserves the previous local binding; uncertainty retires it.
    pub async fn refresh_credential(
        &mut self,
        credential: String,
    ) -> Result<CanonicalAuthentication> {
        self.refresh_credential_with_cancel(credential, tokio_util::sync::CancellationToken::new())
            .await
    }
    pub async fn refresh_credential_with_cancel(
        &mut self,
        credential: String,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<CanonicalAuthentication> {
        if self.step_up.is_some() {
            return Err(ClientError::Protocol);
        }
        if cancellation.is_cancelled() {
            return Err(ClientError::Cancelled);
        }
        if credential.is_empty()
            || credential.len() > 256
            || !credential.bytes().all(|byte| byte.is_ascii_graphic())
        {
            return Err(ClientError::Configuration);
        }
        let request = envelope(
            "auth.refresh",
            json!({"method":"bearer","credential":credential}),
        );
        let id = request["id"]
            .as_str()
            .ok_or(ClientError::Protocol)?
            .to_owned();
        let mut pending = self.pending()?;
        let remaining = (self.authenticated_until.min(self.server_key_expires_at)
            - chrono::Utc::now())
        .to_std()
        .unwrap_or_default();
        let deadline = tokio::time::Instant::now() + self.deadline.min(remaining);
        let mut settled = false;
        let work = async {
            self.write(request, true).await?;
            loop {
                let frame = self.receive_wire().await?;
                if cancellation.is_cancelled() {
                    return Err(ClientError::Cancelled);
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err(ClientError::Timeout);
                }
                if matches!(frame.kind(), "auth.bye" | "identity.assurance-changed") {
                    self.queue_event(frame)?;
                    return Err(ClientError::Transport);
                }
                if frame.scope(Scope::Reply) != Some(id.as_str()) {
                    self.queue_event(frame)?;
                    continue;
                }
                check_reply_scope(&frame, &CommandScope::default(), false)?;
                match frame.payload() {
                    Payload::Error(error) => {
                        settled = true;
                        return Err(remote_error(error.code));
                    }
                    Payload::AuthSession(session) => {
                        if session.identity_id.as_str() != self.identity_id
                            || session.assurance != self.assurance
                            || frame.original()["payload"]["application_profile"] != "connect/2"
                            || session.expires_at <= chrono::Utc::now()
                            || session.session_token.is_empty()
                            || session.session_token == self.session_token
                            || session.session_token.len() > 4096
                            || !session
                                .session_token
                                .bytes()
                                .all(|byte| byte.is_ascii_graphic())
                        {
                            return Err(ClientError::Authentication);
                        }
                        let expires_at = session.expires_at.min(self.server_key_expires_at);
                        if let Some(signaling) = &self.owned_signaling {
                            signaling.refresh_until(expires_at)?;
                        }
                        self.session_token = session.session_token.clone();
                        self.authenticated_until = expires_at;
                        self.record_download.credential = credential.clone();
                        settled = true;
                        return Ok(CanonicalAuthentication {
                            identity_id: self.identity_id.clone(),
                            assurance: session.assurance.clone(),
                            expires_at,
                            application_profile: "connect/2".into(),
                        });
                    }
                    _ => return Err(ClientError::Protocol),
                }
            }
        };
        let result = tokio::select! {biased;_=cancellation.cancelled()=>Err(ClientError::Cancelled),result=tokio::time::timeout_at(deadline,work)=>result.map_err(|_|ClientError::Timeout)?};
        pending.complete = settled;
        result
    }
    /// Send a signed heartbeat using only this peer's current opaque session
    /// token. Receive and correlate its ACK; this cannot extend credentials.
    pub async fn keepalive(&mut self) -> Result<String> {
        let value = envelope(
            "auth.keepalive",
            json!({"session_token":self.session_token}),
        );
        let id = value["id"]
            .as_str()
            .ok_or(ClientError::Protocol)?
            .to_owned();
        self.write(value, true).await?;
        Ok(id)
    }
    /// Receives one verified reply or live fact. Use `in_reply_to` and resource
    /// scopes; a live event or operation receipt is not another command's ACK.
    /// Cancelling/expiring a partial frame retires this physical peer.
    pub async fn receive(&mut self) -> Result<Envelope> {
        // Directed invitations have their own receive API. They share this
        // bounded inbox, but cannot displace unrelated live facts or replies.
        if let Some(index) = self
            .events
            .iter()
            .position(|event| event.kind() != "session.invite")
        {
            let event = self.events.remove(index).ok_or(ClientError::Protocol)?;
            self.event_bytes = self.event_bytes.saturating_sub(encoded_size(&event)?);
            return Ok(event);
        }
        loop {
            let event = self.receive_wire().await?;
            if event.kind() != "session.invite" {
                return Ok(event);
            }
            self.queue_event(event)?;
        }
    }
    async fn receive_wire(&mut self) -> Result<Envelope> {
        self.receive_wire_transition(None).await
    }
    async fn receive_wire_transition(
        &mut self,
        step_up_response: Option<&str>,
    ) -> Result<Envelope> {
        if let Some(signaling) = &self.owned_signaling {
            // These facts were already received and verified. Keep terminal
            // authentication observations readable after retirement, just as
            // the ordinary event cache does. This is no new transport IO;
            // sends and command batches still require their current guards.
            let frame = tokio::time::timeout(self.deadline, signaling.receive())
                .await
                .map_err(|_| ClientError::Timeout)??;
            return Ok(frame);
        }
        let mut pending = self.pending()?;
        let result = tokio::time::timeout(self.deadline, async {
            let mut prefix = [0; 4];
            self.reader
                .as_mut()
                .ok_or(ClientError::Protocol)?
                .read_exact(&mut prefix)
                .await
                .map_err(|_| ClientError::Transport)?;
            let size = u32::from_be_bytes(prefix) as usize;
            if size == 0 || size > MAX_ENVELOPE_BYTES {
                return Err(ClientError::Protocol);
            }
            let mut bytes = vec![0; size];
            self.reader
                .as_mut()
                .ok_or(ClientError::Protocol)?
                .read_exact(&mut bytes)
                .await
                .map_err(|_| ClientError::Transport)?;
            let value = Envelope::parse(&bytes).map_err(|_| ClientError::Protocol)?;
            self.verifier
                .verify_with_context(value.original(), &self.verification)
                .await
                .map_err(|_| ClientError::Authentication)?;
            Ok(value)
        })
        .await
        .map_err(|_| ClientError::Timeout)??;
        let verified_step_up = step_up_response.is_some_and(|response| {
            result.scope(Scope::Reply) == Some(response)
                && matches!(result.payload(), Payload::IdentityAssuranceChanged(change)
                    if change.reason == "verified-step-up")
        });
        if result.kind() == "auth.bye"
            || (result.kind() == "identity.assurance-changed" && !verified_step_up)
        {
            self.closed.store(true, Ordering::Release);
            self.connection
                .close(0u32.into(), b"canonical authority retired");
        }
        pending.complete = true;
        Ok(result)
    }
    /// One bounded, correlated command batch. Live facts and replies to other
    /// request IDs stay in a bounded inbox for `receive`; they cannot complete
    /// this command. A successful ACK is not a logical operation completion.
    pub async fn request(&mut self, payload: Payload, scope: CommandScope) -> Result<ReplyBatch> {
        self.request_with_cancel(payload, scope, tokio_util::sync::CancellationToken::new())
            .await
    }
    pub async fn request_with_cancel(
        &mut self,
        payload: Payload,
        scope: CommandScope,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<ReplyBatch> {
        self.request_value_with_cancel(
            payload.kind(),
            serde_json::to_value(&payload).map_err(|_| ClientError::Protocol)?["payload"].clone(),
            scope,
            cancellation,
        )
        .await
    }
    /// Validate and send original payload values, retaining optional extensions.
    pub async fn request_value(
        &mut self,
        kind: &str,
        payload: Value,
        scope: CommandScope,
    ) -> Result<ReplyBatch> {
        self.request_value_with_cancel(
            kind,
            payload,
            scope,
            tokio_util::sync::CancellationToken::new(),
        )
        .await
    }
    pub async fn request_value_with_cancel(
        &mut self,
        kind: &str,
        payload: Value,
        scope: CommandScope,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<ReplyBatch> {
        self.request_correlated_value(kind, payload, scope, cancellation, None)
            .await
    }
    async fn request_correlated_value(
        &mut self,
        kind: &str,
        payload: Value,
        scope: CommandScope,
        cancellation: tokio_util::sync::CancellationToken,
        invitation_reference: Option<&str>,
    ) -> Result<ReplyBatch> {
        let (value, mut exchange) =
            self.prepare_exchange(kind, payload, scope, invitation_reference)?;
        if cancellation.is_cancelled() {
            return Err(ClientError::Cancelled);
        }
        let mut pending = self.pending()?;
        let deadline = exchange.deadline;
        let work = async {
            self.write(value, true).await?;
            loop {
                if cancellation.is_cancelled() {
                    return Err(ClientError::Cancelled);
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err(ClientError::Timeout);
                }
                tokio::task::consume_budget().await;
                let frame = self.receive_wire().await?;
                if cancellation.is_cancelled() {
                    return Err(ClientError::Cancelled);
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err(ClientError::Timeout);
                }
                if matches!(frame.kind(), "auth.bye" | "identity.assurance-changed") {
                    self.queue_event(frame)?;
                    return Err(ClientError::Transport);
                }
                if frame.scope(Scope::Reply) != Some(exchange.id.as_str()) {
                    self.queue_event(frame)?;
                    continue;
                }
                if self.accept_exchange_frame(&mut exchange, frame)? {
                    return Ok(exchange.finish());
                }
            }
        };
        let result = tokio::select! {biased;_=cancellation.cancelled()=>Err(ClientError::Cancelled),result=tokio::time::timeout_at(deadline,work)=>result.map_err(|_|ClientError::Timeout)?};
        if result.is_ok() {
            pending.complete = true;
        }
        result
    }
    fn queue_event(&mut self, frame: Envelope) -> Result<()> {
        let size = encoded_size(&frame)?;
        if self.events.len() >= MAX_BATCH_FRAMES
            || self.event_bytes.saturating_add(size) > MAX_BATCH_BYTES
        {
            return Err(ClientError::Protocol);
        }
        self.event_bytes += size;
        self.events.push_back(frame);
        Ok(())
    }
    /// Move the same authenticated QUIC peer to a new local UDP socket. QUIC
    /// validates the path; logical operation and native Connection IDs persist.
    /// This does not reconnect or renew application/credential authority.
    pub fn rebind_network(&mut self, bind: SocketAddr) -> Result<SocketAddr> {
        self.check_current()?;
        let bind = NativeNetworkProfile::canonical_address(bind);
        if bind.is_ipv6() != self.server_address.is_ipv6()
            || !self.network_profile.permits_listener(bind)
        {
            return Err(ClientError::Configuration);
        }
        let socket = std::net::UdpSocket::bind(bind).map_err(|_| ClientError::Transport)?;
        self.endpoint
            .rebind(socket)
            .map_err(|_| ClientError::Transport)?;
        self.endpoint
            .local_addr()
            .map_err(|_| ClientError::Transport)
    }
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.connection
            .close(0u32.into(), b"canonical client closed");
        self.endpoint.close(0u32.into(), b"canonical client closed");
    }
    /// Close owned transport and give QUIC its bounded protocol-drain interval.
    /// A drain failure does not revoke a receipt already returned to the caller.
    pub async fn shutdown(mut self) -> Result<()> {
        // Match the pinned transport's three-PTO closing interval. Its RTT
        // estimator bounds variance by twice smoothed RTT; a fixed two-second
        // allowance can therefore expire while protocol drain is still valid.
        let protocol_drain_budget = self
            .connection
            .rtt()
            .saturating_mul(27)
            .saturating_add(Duration::from_millis(250))
            .clamp(Duration::from_secs(5), Duration::from_secs(30));
        self.close();
        tokio::time::timeout(protocol_drain_budget, async {
            if let Some(reader) = self.owned_signaling.as_mut() {
                reader.shutdown().await?;
            }
            self.endpoint.wait_idle().await;
            Ok(())
        })
        .await
        .map_err(|_| ClientError::Transport)?
    }
}
impl Drop for CanonicalClient {
    fn drop(&mut self) {
        self.close();
    }
}
fn envelope(kind: &str, payload: Value) -> Value {
    json!({"v":2,"type":kind,"id":format!("env_{}",uuid::Uuid::new_v4().simple()),"ts":chrono::Utc::now(),"payload":payload})
}
fn bounded_file(path: &Path, maximum: usize) -> Result<Vec<u8>> {
    use std::io::Read;
    let metadata = std::fs::metadata(path).map_err(|_| ClientError::Configuration)?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > maximum as u64 {
        return Err(ClientError::Configuration);
    }
    let file = std::fs::File::open(path).map_err(|_| ClientError::Configuration)?;
    let mut bytes = Vec::new();
    file.take((maximum + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| ClientError::Configuration)?;
    if bytes.is_empty() || bytes.len() > maximum {
        return Err(ClientError::Configuration);
    }
    Ok(bytes)
}

const MAX_BATCH_FRAMES: usize = 128;
const MAX_BATCH_BYTES: usize = 2 * 1024 * 1024;
/// Verified frames through the command's correlated ACK/error or step-up challenge. Earlier facts
/// remain observations; inspect operation.updated state for logical completion.
pub struct ReplyBatch {
    pub request_id: String,
    pub frames: Vec<Envelope>,
}
impl ReplyBatch {
    /// A challenge suspends the original command; it is not an ACK or completion.
    /// Supply an explicit successor to `CanonicalClient::complete_step_up`.
    pub fn step_up_request(&self) -> Option<&payloads::IdentityStepUpRequest> {
        match self.frames.last().map(Envelope::payload) {
            Some(Payload::IdentityStepUpRequest(value)) => Some(value),
            _ => None,
        }
    }
    pub fn ack(&self) -> Result<&payloads::Ack> {
        match self.frames.last().map(Envelope::payload) {
            Some(Payload::Ack(value)) => Ok(value),
            Some(Payload::IdentityStepUpRequest(_)) => {
                Err(ClientError::Application(connect_protocol::Error::new(
                    "step-up-required",
                    "Explicit current authority is required for the suspended command",
                )))
            }
            Some(Payload::Error(value)) => Err(remote_error(value.code)),
            _ => Err(ClientError::Protocol),
        }
    }
}
fn remote_error(code: u64) -> ClientError {
    let (code, message) = match code {
        400 => ("invalid-request", "The canonical command is invalid"),
        401 => ("unauthorized", "Canonical authentication is unavailable"),
        403 => ("forbidden", "The canonical command is not authorized"),
        404 => ("not-found", "The canonical resource is unavailable"),
        409 => (
            "conflict",
            "The canonical operation conflicts with current state",
        ),
        429 => ("capacity-exceeded", "Canonical capacity is unavailable"),
        488 => (
            "incompatible-capabilities",
            "Requested media capabilities are incompatible",
        ),
        501 => (
            "unsupported-command",
            "The canonical command is not installed",
        ),
        503 => (
            "storage-unavailable",
            "Canonical state is temporarily unavailable",
        ),
        _ => ("operation-rejected", "The canonical command was refused"),
    };
    ClientError::Application(connect_protocol::Error::new(code, message))
}
fn command(payload: Payload, scope: CommandScope) -> Result<Value> {
    let kind = payload.kind();
    let value =
        serde_json::to_value(&payload).map_err(|_| ClientError::Protocol)?["payload"].clone();
    command_value(kind, value, scope)
}
fn command_value(kind: &str, payload: Value, scope: CommandScope) -> Result<Value> {
    if kind.starts_with("auth.") {
        return Err(ClientError::Protocol);
    }
    let mut value = envelope(kind, payload);
    for (name, id) in [
        ("cid", scope.conversation_id),
        ("sid", scope.session_id),
        ("connid", scope.connection_id),
        ("operation_id", scope.operation_id),
    ] {
        if let Some(id) = id {
            value[name] = json!(id);
        }
    }
    if let Some(revision) = scope.expected_revision {
        value["expected_revision"] = json!(revision);
    }
    Envelope::from_value(value.clone()).map_err(|_| ClientError::Protocol)?;
    Ok(value)
}
fn encoded_size(frame: &Envelope) -> Result<usize> {
    serde_json::to_vec(frame.original())
        .map(|v| v.len())
        .map_err(|_| ClientError::Protocol)
}
fn check_reply_scope(
    frame: &Envelope,
    scope: &CommandScope,
    assigned_conversation: bool,
) -> Result<()> {
    if scope.session_id.is_none()
        && scope.connection_id.is_none()
        && scope.operation_id.is_none()
        && matches!(
            frame.kind(),
            "conversation.event"
                | "session.started"
                | "session.participant.joined"
                | "session.participant.left"
                | "session.ended"
                | "recording.vcon-ready"
                | "message.read"
                | "message.delivered"
        )
    {
        if frame.scope(Scope::Conversation) != scope.conversation_id.as_deref() {
            return Err(ClientError::Protocol);
        }
        if frame.kind() == "conversation.event" {
            let event: connect_protocol::events::JournalEvent =
                serde_json::from_value(frame.original()["payload"]["data"].clone())
                    .map_err(|_| ClientError::Protocol)?;
            if frame.original()["payload"]["event_id"] != event.event_id
                || frame.original()["payload"]["cursor"] != event.cursor
                || frame.scope(Scope::Conversation) != Some(event.conversation_id.as_str())
                || frame.scope(Scope::Session) != event.session_id.as_deref()
                || frame.scope(Scope::Connection) != event.connection_id.as_deref()
                || frame.scope(Scope::Operation) != event.operation_id.as_deref()
            {
                return Err(ClientError::Protocol);
            }
        } else if matches!(
            frame.kind(),
            "session.started"
                | "session.ended"
                | "session.participant.joined"
                | "session.participant.left"
        ) {
            if frame.scope(Scope::Session).is_none()
                || frame.scope(Scope::Connection).is_some()
                || frame.scope(Scope::Operation).is_some()
            {
                return Err(ClientError::Protocol);
            }
        } else if frame.kind() == "recording.vcon-ready" {
            let handle = &frame.original()["payload"]["vcon_handle"];
            if frame.scope(Scope::Session).is_none()
                || frame.scope(Scope::Connection).is_some()
                || frame.scope(Scope::Operation).is_some()
                || handle["uuid"]
                    .as_str()
                    .is_none_or(|id| uuid::Uuid::parse_str(id).is_err())
                || handle["revision"]
                    .as_u64()
                    .is_none_or(|v| v == 0 || v > 9_007_199_254_740_991)
            {
                return Err(ClientError::Protocol);
            }
        } else if frame.scope(Scope::Session).is_some()
            || frame.scope(Scope::Connection).is_some()
            || frame.original()["payload"]["evidence"]["event_id"]
                .as_str()
                .is_none_or(|id| Identifier::try_from(id.to_owned()).is_err())
        {
            return Err(ClientError::Protocol);
        }
        return Ok(());
    }
    for (field, expected) in [
        (Scope::Session, scope.session_id.as_deref()),
        (Scope::Connection, scope.connection_id.as_deref()),
        (Scope::Operation, scope.operation_id.as_deref()),
    ] {
        if frame.scope(field) != expected {
            return Err(ClientError::Protocol);
        }
    }
    if frame.scope(Scope::Conversation) != scope.conversation_id.as_deref()
        && !(assigned_conversation && scope.conversation_id.is_none())
    {
        return Err(ClientError::Protocol);
    }
    Ok(())
}
mod calls;
mod records;
mod trust;
pub use records::{CanonicalRecord, CanonicalRecordPage, CanonicalRecordSelector, RecordTlsConfig};
mod capabilities;
mod dtmf;
pub mod http;
mod native;
mod quality;
mod step_up;
pub use calls::CanonicalHistoryPage;
pub use http::CanonicalHttpClient;
pub use step_up::StepUpCompletion;
#[cfg(test)]
mod tests;

mod handoff_audio;
mod owned_signaling;
mod pcmu_bridge;
mod private_audio;
mod public_audio;
pub use handoff_audio::PreparedHandoffNativeAudio;
pub use pcmu_bridge::{PcmuBridgeExit, PcmuBridgeReport, PcmuDevice, PublicPcmuBridge};
pub use private_audio::PrivateNativeAudio;
pub use public_audio::PublicNativeAudio;

mod session_invitations;
pub use session_invitations::SessionInvitationAnswer;

pub(crate) mod exchange;
