//! An owned public audio peer, created by its own signed offer and confirmation.
use super::private_audio::{AudioPreparation, NativeAudioPeer};
use super::*;

/// Mono PCMU at8kHz in20ms frames. Construction consumes the peer that actually
/// negotiated this fresh Connection; IDs and another peer's answer cannot
/// construct it. Context delivery and provider readiness are separate facts.
pub struct PublicNativeAudio {
    peer: NativeAudioPeer,
    capabilities: Value,
}
impl PublicNativeAudio {
    pub fn step_up_required(&self) -> Option<&str> {
        self.peer.client.step_up_required()
    }

    /// Complete this peer's pending signed challenge with explicit consent or
    /// task authority. The server resumes its original logical mutation.
    pub async fn complete_step_up(&mut self, credential: String) -> Result<StepUpCompletion> {
        self.peer.client.complete_step_up(credential).await
    }
    pub async fn complete_step_up_with_cancel(
        &mut self,
        credential: String,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<StepUpCompletion> {
        self.peer
            .client
            .complete_step_up_with_cancel(credential, cancellation)
            .await
    }

    /// Refresh this physical peer with an explicit successor credential. The
    /// original signaling reader and its queued observations remain owned.
    pub async fn refresh_credential(
        &mut self,
        credential: String,
    ) -> Result<CanonicalAuthentication> {
        self.peer.client.refresh_credential(credential).await
    }
    pub async fn refresh_credential_with_cancel(
        &mut self,
        credential: String,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<CanonicalAuthentication> {
        self.peer
            .client
            .refresh_credential_with_cancel(credential, cancellation)
            .await
    }

    /// Submit on this exact physical peer. Preserve the original operation ID
    /// and optional revision after uncertainty; reconcile local writer outcomes.
    pub async fn send_dtmf(
        &mut self,
        operation_id: &str,
        expected_revision: Option<u64>,
        digits: &str,
        duration_ms: u32,
        method: &str,
    ) -> Result<payloads::OperationUpdated> {
        self.peer
            .send_dtmf(operation_id, expected_revision, digits, duration_ms, method)
            .await
    }
    pub fn connection_id(&self) -> &str {
        self.peer.connection_id()
    }
    pub fn conversation_id(&self) -> Option<&str> {
        self.peer.scope.conversation_id.as_deref()
    }
    pub fn session_id(&self) -> Option<&str> {
        self.peer.scope.session_id.as_deref()
    }
    /// Fresh offers require expected_revision=0 and are admitted as generation1.
    pub fn generation(&self) -> u64 {
        1
    }
    pub fn stream_revision(&self) -> u64 {
        self.peer.stream_revision
    }
    /// Select audible public sources through this peer's signed native control
    /// interface before handing it to a bridge. This fixed profile accepts only
    /// one sendrecv PCMU stream. An uncertain/unsupported selection closes the
    /// peer; it cannot silently keep an earlier media route after an error.
    /// Retain the original operation ID, expected revision and selectors for a
    /// replay. A newer local revision must never rewrite that logical intent.
    pub async fn subscribe_audio(
        &mut self,
        operation_id: &str,
        expected_revision: u64,
        subscriptions: Vec<payloads::StreamSubscription>,
    ) -> Result<u64> {
        let mut pending = self.peer.client.pending()?;
        let remaining = (self
            .peer
            .client
            .authenticated_until
            .min(self.peer.client.server_key_expires_at)
            - chrono::Utc::now())
        .to_std()
        .unwrap_or_default();
        let until = tokio::time::Instant::now() + self.peer.client.deadline.min(remaining);
        let result = tokio::time::timeout_at(
            until,
            self.select_audio(operation_id, expected_revision, subscriptions),
        )
        .await
        .unwrap_or(Err(ClientError::Timeout));
        if result.is_err() {
            self.peer.close();
        } else {
            pending.complete = true;
        }
        result
    }
    async fn select_audio(
        &mut self,
        operation_id: &str,
        expected_revision: u64,
        subscriptions: Vec<payloads::StreamSubscription>,
    ) -> Result<u64> {
        let mut scope = self.peer.scope.clone();
        scope.operation_id = Some(operation_id.into());
        scope.expected_revision = Some(expected_revision);
        let expected = expected_revision
            .checked_add(1)
            .ok_or(ClientError::Protocol)?;
        let id = self
            .peer
            .client
            .send(
                Payload::StreamSubscribe(payloads::StreamSubscribe {
                    by_participant: None,
                    subscriptions,
                }),
                scope.clone(),
            )
            .await?;
        let mut handle = self.peer.handle;
        let mut completed = false;
        for _ in 0..MAX_BATCH_FRAMES {
            let frame = self.peer.client.receive_wire().await?;
            if matches!(frame.kind(), "auth.bye" | "identity.assurance-changed") {
                self.peer.client.queue_event(frame)?;
                return Err(ClientError::Transport);
            }
            // A server parameter request belongs to this command by its exact
            // operation and resource scope. Other events/replies stay available
            // in the bounded inbox instead of becoming a false control result.
            let parameter_request = matches!(frame.payload(), Payload::ConnectionOffer(_))
                && frame.scope(Scope::Operation) == Some(operation_id)
                && frame.scope(Scope::Conversation) == scope.conversation_id.as_deref()
                && frame.scope(Scope::Session) == scope.session_id.as_deref()
                && frame.scope(Scope::Connection) == scope.connection_id.as_deref();
            if frame.scope(Scope::Reply) != Some(id.as_str()) && !parameter_request {
                self.peer.client.queue_event(frame)?;
                continue;
            }
            check_reply_scope(&frame, &scope, false)?;
            match frame.payload() {
                Payload::ConnectionOffer(_) => {
                    let p = &frame.original()["payload"];
                    let streams = p["substrate_setup"]["streams"]
                        .as_array()
                        .ok_or(ClientError::Protocol)?;
                    if streams.len() != 1
                        || streams[0]["kind"] != "audio"
                        || streams[0]["direction"] != "sendrecv"
                        || streams[0]["codec"]["name"] != "pcmu"
                        || streams[0]["codec"]["params"]["payload_type"] != 0
                        || streams[0]["codec"]["params"]["sample_rate"] != 8000
                        || streams[0]["codec"]["params"]["channels"] != 1
                    {
                        return Err(ClientError::Protocol);
                    }
                    handle = u16::try_from(
                        streams[0]["stream_local_id"]
                            .as_u64()
                            .ok_or(ClientError::Protocol)?,
                    )
                    .map_err(|_| ClientError::Protocol)?;
                    if handle == 0 {
                        return Err(ClientError::Protocol);
                    }
                    let answer=serde_json::from_value(json!({"by_participant":p["by_participant"],"substrate":"quic","capabilities":self.capabilities,"streams_answered":[{"id":streams[0]["strm_id"],"kind":"audio","direction":"sendrecv","codec":streams[0]["codec"]}],"substrate_setup":p["substrate_setup"]})).map_err(|_|ClientError::Protocol)?;
                    self.peer
                        .client
                        .answer_connection_offer(&frame, answer, &scope)
                        .await?;
                }
                Payload::Error(error) => return Err(super::remote_error(error.code)),
                Payload::OperationUpdated(update)
                    if update.operation_id.as_str() == operation_id && !completed =>
                {
                    let result = update.result.as_ref().ok_or(ClientError::Protocol)?;
                    if update.state != payloads::OperationState::Completed
                        || update.problem.is_some()
                        || result["connection_id"].as_str() != Some(self.connection_id())
                        || result["stream_revision"] != expected
                    {
                        return Err(ClientError::Protocol);
                    }
                    completed = true;
                }
                Payload::Ack(_) if frame.scope(Scope::Reply) == Some(id.as_str()) && completed => {
                    // Cached facts do not reactivate an expired/retired audio
                    // peer, even if its old receipt was already received.
                    let mut current = self.peer.client.pending()?;
                    current.complete = true;
                    self.peer.handle = handle;
                    self.peer.stream_revision = self.peer.stream_revision.max(expected);
                    return Ok(expected);
                }
                _ => return Err(ClientError::Protocol),
            }
        }
        Err(ClientError::Protocol)
    }
    pub fn send_pcmu(&mut self, bytes: &[u8]) -> Result<()> {
        self.peer.send_pcmu(bytes)
    }
    pub async fn receive_pcmu(&mut self) -> Result<Vec<u8>> {
        self.peer.receive_pcmu().await
    }
    /// Read verified signaling retained by this audio peer. Passive facts
    /// coalesce by resource; replies and signals retain order. Cancelling this
    /// inbox wait leaves the owned reliable reader and atomic audio IO intact.
    pub async fn receive_signaling(&mut self) -> Result<Envelope> {
        self.peer.client.receive().await
    }
    pub fn close(&self) {
        self.peer.close();
    }
    pub async fn shutdown(self) -> Result<()> {
        self.peer.shutdown().await
    }
}
impl CanonicalClient {
    /// Establish a new public native audio Connection. Private consultation and
    /// handoff preparation markers are refused before sending an offer. The
    /// signed durable result must confirm this exact ordinary admission.
    pub async fn prepare_public_native_audio(
        self,
        scope: CommandScope,
        offer: Value,
    ) -> Result<PublicNativeAudio> {
        let capabilities = offer["capabilities"].clone();
        let (peer, _) = self
            .prepare_owned_native_audio(scope, AudioPreparation::Public, offer)
            .await?;
        Ok(PublicNativeAudio { peer, capabilities })
    }
}
