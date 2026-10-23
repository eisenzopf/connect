//! Signed audio negotiation and its owned physical QUIC peer.
use super::*;
use rvoip_uctp::substrate::{pack_rtp_datagram, unpack_rtp_datagram, RtpDatagram, RtpMediaPayload};

/// Owns the exact client that negotiated the server-confirmed audio purpose.
/// It cannot be constructed from a Connection ID, answer, or another peer's
/// operation result. This profile carries mono PCMU at8kHz in20ms frames.
pub(crate) struct NativeAudioPeer {
    pub(super) client: CanonicalClient,
    pub(super) handle: u16,
    pub(super) stream_revision: u64,
    sequence: u32,
    clock: tokio::time::Instant,
    pub(crate) scope: CommandScope,
}
impl NativeAudioPeer {
    pub(super) async fn send_dtmf(
        &mut self,
        operation_id: &str,
        expected_revision: Option<u64>,
        digits: &str,
        duration_ms: u32,
        method: &str,
    ) -> Result<payloads::OperationUpdated> {
        let mut scope = self.scope.clone();
        scope.operation_id = Some(operation_id.into());
        // Retry identity is caller-owned. Never substitute a newer local
        // stream revision into an earlier logical signal.
        scope.expected_revision = expected_revision;
        self.client
            .send_native_dtmf(scope, digits, duration_ms, method)
            .await
    }
    pub fn connection_id(&self) -> &str {
        self.scope.connection_id.as_deref().unwrap_or("")
    }
    pub fn send_pcmu(&mut self, bytes: &[u8]) -> Result<()> {
        if bytes.len() != 160 {
            return Err(ClientError::Protocol);
        }
        let packet = pack_rtp_datagram(&RtpDatagram {
            flags: 0,
            stream_local_id: self.handle,
            seq: self.sequence,
            rtp: RtpMediaPayload {
                payload: bytes.to_vec().into(),
                payload_type: 0,
                sequence_number: self.sequence as u16,
                timestamp: ((self.clock.elapsed().as_millis() / 20) * 160) as u32,
                ssrc: 0x6370_7601,
            },
        })
        .map_err(|_| ClientError::Protocol)?;
        self.client.send_native_rtp_datagram(&packet)?;
        self.sequence = self.sequence.wrapping_add(1);
        Ok(())
    }
    /// Atomic datagram reception remains cancellation-safe. The bridge never
    /// cancels a partially read signed signaling frame to service audio.
    pub async fn receive_pcmu(&mut self) -> Result<Vec<u8>> {
        let bytes = self.client.receive_native_rtp_datagram().await?;
        let packet = unpack_rtp_datagram(&bytes).map_err(|_| ClientError::Protocol)?;
        if packet.stream_local_id != self.handle
            || packet.rtp.payload_type != 0
            || packet.rtp.payload.len() != 160
        {
            return Err(ClientError::Protocol);
        }
        Ok(packet.rtp.payload.to_vec())
    }
    pub fn close(&self) {
        self.client.close();
    }
    pub async fn shutdown(self) -> Result<()> {
        self.client.shutdown().await
    }
}
pub struct PrivateNativeAudio {
    peer: NativeAudioPeer,
    context_record: String,
}
impl PrivateNativeAudio {
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
    pub fn context_record_id(&self) -> &str {
        &self.context_record
    }
    pub fn connection_id(&self) -> &str {
        self.peer.connection_id()
    }
    pub fn send_pcmu(&mut self, bytes: &[u8]) -> Result<()> {
        self.peer.send_pcmu(bytes)
    }
    pub async fn receive_pcmu(&mut self) -> Result<Vec<u8>> {
        self.peer.receive_pcmu().await
    }
    /// Receive verified retained signaling without taking over its wire reader.
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
pub(crate) enum AudioPreparation {
    Public,
    Consultation(String),
    Handoff,
}
impl CanonicalClient {
    /// This consumes a fresh physical peer. The caller first registers the full
    /// invitation with its server coordinator. A signed final result confirming
    /// muted/quarantined preparation is mandatory before returning any audio IO.
    pub async fn prepare_private_native_audio(
        self,
        scope: CommandScope,
        context_record: &str,
        offer: Value,
    ) -> Result<PrivateNativeAudio> {
        let (peer, _) = self
            .prepare_owned_native_audio(
                scope,
                AudioPreparation::Consultation(context_record.into()),
                offer,
            )
            .await?;
        Ok(PrivateNativeAudio {
            peer,
            context_record: context_record.into(),
        })
    }
    pub(crate) async fn prepare_owned_native_audio(
        mut self,
        scope: CommandScope,
        preparation: AudioPreparation,
        offer: Value,
    ) -> Result<(NativeAudioPeer, String)> {
        let (request_marker, result_marker, expected_request, expected_result) = match &preparation
        {
            AudioPreparation::Public => ("", "", Value::Null, Value::Null),
            AudioPreparation::Consultation(record) => (
                "private_consultation",
                "private_consultation",
                json!({"profile":"connect-private-audio-v1","operation_record_id":record}),
                json!({"profile":"connect-private-audio-v1","operation_record_id":record,"prepared_muted":true}),
            ),
            AudioPreparation::Handoff => (
                "handoff_preparation",
                "handoff_preparation",
                json!({"profile":"connect-handoff-preparation-v1"}),
                json!({"profile":"connect-handoff-preparation-v1","prepared_muted":true}),
            ),
        };
        let purpose_valid = match &preparation {
            AudioPreparation::Public => {
                offer.get("private_consultation").is_none()
                    && offer.get("handoff_preparation").is_none()
            }
            _ => {
                offer[request_marker] == expected_request
                    && offer
                        .get(if request_marker == "private_consultation" {
                            "handoff_preparation"
                        } else {
                            "private_consultation"
                        })
                        .is_none()
            }
        };
        if !purpose_valid
            || scope.expected_revision != Some(0)
            || scope.connection_id.is_none()
            || scope.operation_id.is_none()
            || offer["streams_offered"].as_array().is_none_or(|s| {
                s.len() != 1
                    || s[0]["kind"] != "audio"
                    || s[0]["direction"] != "sendrecv"
                    || s[0]["codec_preferences"] != json!(["g.711-mu"])
            })
        {
            return Err(ClientError::Protocol);
        }
        let id = self
            .send_value("connection.offer", offer.clone(), scope.clone())
            .await?;
        let mut handle = None;
        let mut prepared_setup = None;
        loop {
            let response = self.receive().await?;
            for (key, expected) in [
                ("cid", scope.conversation_id.as_deref()),
                ("sid", scope.session_id.as_deref()),
                ("connid", scope.connection_id.as_deref()),
                ("operation_id", scope.operation_id.as_deref()),
            ] {
                if response.original().get(key).and_then(Value::as_str) != expected {
                    return Err(ClientError::Protocol);
                }
            }
            let p = &response.original()["payload"];
            match response.kind() {
                "connection.answer"
                    if response.original()["in_reply_to"] == id && handle.is_none() =>
                {
                    let streams = p["substrate_setup"]["streams"]
                        .as_array()
                        .ok_or(ClientError::Protocol)?;
                    if streams.len() != 1
                        || streams[0]["kind"] != "audio"
                        || streams[0]["codec"]["name"] != "pcmu"
                        || streams[0]["codec"]["params"]["payload_type"] != 0
                        || streams[0]["codec"]["params"]["sample_rate"] != 8000
                        || streams[0]["codec"]["params"]["channels"] != 1
                        || streams[0]["direction"] != "sendrecv"
                        || streams[0]["strm_id"] != offer["streams_offered"][0]["id"]
                    {
                        return Err(ClientError::Protocol);
                    }
                    prepared_setup = Some(p["substrate_setup"].clone());
                    handle = Some(
                        u16::try_from(
                            streams[0]["stream_local_id"]
                                .as_u64()
                                .ok_or(ClientError::Protocol)?,
                        )
                        .map_err(|_| ClientError::Protocol)?,
                    );
                }
                "connection.offer" if handle.is_some() => {
                    let streams = p["substrate_setup"]["streams"]
                        .as_array()
                        .ok_or(ClientError::Protocol)?;
                    if streams.len() != 1
                        || prepared_setup.as_ref() != Some(&p["substrate_setup"])
                        || streams[0]["stream_local_id"].as_u64() != handle.map(u64::from)
                    {
                        return Err(ClientError::Protocol);
                    }
                    let answered=streams.iter().map(|s|json!({"id":s["strm_id"],"kind":s["kind"],"direction":"sendrecv","codec":s["codec"]})).collect::<Vec<_>>();
                    let answer=serde_json::from_value(json!({"by_participant":p["by_participant"],"substrate":"quic","capabilities":offer["capabilities"],"streams_answered":answered,"substrate_setup":p["substrate_setup"]})).map_err(|_|ClientError::Protocol)?;
                    self.answer_connection_offer(&response, answer, &scope)
                        .await?;
                    break;
                }
                _ => return Err(ClientError::Protocol),
            }
        }
        // A native offer ends with its actual answer, without an invented final
        // ACK. Query the signed durable result separately after confirmation.
        let deadline = tokio::time::Instant::now() + self.deadline;
        loop {
            let status = self
                .operation_get(
                    scope.operation_id.as_deref().ok_or(ClientError::Protocol)?,
                    CommandScope {
                        conversation_id: scope.conversation_id.clone(),
                        ..Default::default()
                    },
                )
                .await?;
            let value = serde_json::to_value(status).map_err(|_| ClientError::Protocol)?;
            if value["state"] == "completed" {
                let purpose_confirmed = match &preparation {
                    AudioPreparation::Public => {
                        value["result"].get("private_consultation").is_none()
                            && value["result"].get("handoff_preparation").is_none()
                            && value["result"]["streams"]
                                == json!([offer["streams_offered"][0]["id"]])
                            && value["result"]["stream_revision"]
                                .as_u64()
                                .is_some_and(|r| r > 0)
                    }
                    _ => value["result"][result_marker] == expected_result,
                };
                if !purpose_confirmed
                    || value["operation_id"].as_str() != scope.operation_id.as_deref()
                    || !value["problem"].is_null()
                    || value["revision"].as_u64().is_none_or(|r| r == 0)
                {
                    return Err(ClientError::Protocol);
                }
                let policy_record = value["record_id"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or(ClientError::Protocol)?
                    .to_owned();
                self.own_signaling()?;
                return Ok((
                    NativeAudioPeer {
                        client: self,
                        handle: handle.filter(|h| *h != 0).ok_or(ClientError::Protocol)?,
                        stream_revision: value["result"]["stream_revision"]
                            .as_u64()
                            .filter(|r| *r > 0)
                            .ok_or(ClientError::Protocol)?,
                        sequence: 0,
                        clock: tokio::time::Instant::now(),
                        scope,
                    },
                    policy_record,
                ));
            }
            if !matches!(
                value["state"].as_str(),
                Some("accepted" | "preparing" | "ready" | "committing")
            ) || tokio::time::Instant::now() >= deadline
            {
                return Err(ClientError::Protocol);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}
