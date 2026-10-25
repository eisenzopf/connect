//! Same-root signed native confirmation and bounded atomic media datagrams.
use super::*;
const MAX_NATIVE_MEDIA_DATAGRAM: usize = 16 * 1024 + 20;
fn answer_frame(
    request: &Envelope,
    payload: payloads::ConnectionAnswer,
    root: &CommandScope,
) -> Result<Value> {
    if request.kind() != "connection.offer" {
        return Err(ClientError::Protocol);
    }
    let mut value = envelope(
        "connection.answer",
        serde_json::to_value(Payload::ConnectionAnswer(payload))
            .map_err(|_| ClientError::Protocol)?["payload"]
            .clone(),
    );
    for (scope, field, expected) in [
        (Scope::Conversation, "cid", root.conversation_id.as_deref()),
        (Scope::Session, "sid", root.session_id.as_deref()),
        (Scope::Connection, "connid", root.connection_id.as_deref()),
        (
            Scope::Operation,
            "operation_id",
            root.operation_id.as_deref(),
        ),
    ] {
        let expected = expected.ok_or(ClientError::Protocol)?;
        if request.scope(scope) != Some(expected) {
            return Err(ClientError::Protocol);
        }
        value[field] = json!(expected);
    }
    value["in_reply_to"] = json!(request.id());
    Envelope::from_value(value.clone()).map_err(|_| ClientError::Protocol)?;
    Ok(value)
}
fn datagram(bytes: &[u8]) -> Result<()> {
    if bytes.len() > MAX_NATIVE_MEDIA_DATAGRAM {
        return Err(ClientError::Protocol);
    }
    rvoip_uctp::substrate::observe_native_media_datagram(bytes)
        .map_err(|_| ClientError::Protocol)?;
    Ok(())
}
fn replacement_offer(
    command: &connect_protocol::transitions::TransitionCommand,
    root: &payloads::OperationUpdated,
    offer: payloads::ConnectionOffer,
    profile: &str,
) -> Result<(Value, CommandScope)> {
    let mut command = command.clone();
    command.normalize()?;
    let connect_protocol::transitions::TransitionIntent::Replace {
        candidate_connid, ..
    } = &command.intent
    else {
        return Err(ClientError::Protocol);
    };
    let record = root.record_id.as_ref().ok_or(ClientError::Protocol)?;
    if root.operation_id.as_str() != command.operation_id
        || !matches!(
            root.state,
            rvoip_uctp::v2::OperationState::Accepted | rvoip_uctp::v2::OperationState::Preparing
        )
    {
        return Err(ClientError::Protocol);
    }
    let candidate = candidate_connid
        .clone()
        .unwrap_or_else(|| format!("replacement_{}", record.as_str()));
    let mut payload = serde_json::to_value(offer).map_err(|_| ClientError::Protocol)?;
    payload["replacement_preparation"] = json!({"profile":profile,"operation_record_id":record});
    Ok((
        payload,
        CommandScope {
            conversation_id: Some(command.conversation_id),
            session_id: Some(command.session_id),
            connection_id: Some(candidate),
            operation_id: Some(command.operation_id),
            expected_revision: Some(0),
        },
    ))
}
impl CanonicalClient {
    /// Allocate this signed target against the original replacement root. This
    /// consumes its reserved Core candidate once; it does not negotiate media,
    /// confirm the endpoint, commit movement or retire the old route. Preserve
    /// this command/root on retry; unknown outcomes require status recovery.
    pub async fn allocate_native_replacement_target(
        &mut self,
        command: &connect_protocol::transitions::TransitionCommand,
        root: &payloads::OperationUpdated,
        offer: payloads::ConnectionOffer,
    ) -> Result<payloads::OperationUpdated> {
        let (payload, scope) =
            replacement_offer(command, root, offer, "connect-replacement-preparation-v1")?;
        let batch = self
            .request_value("connection.offer", payload, scope)
            .await?;
        super::calls::operation_reply(&batch, &command.operation_id)
    }
    /// Begin the separately signed media phase on the same allocated target.
    /// Receive the server parameter offer and use answer_connection_offer with
    /// the returned exact scope. Its answer is not a final ACK or media-readiness
    /// event; later native lifecycle hints require actual flow. Observe resource
    /// capture through the original operation status;
    /// movement requires actual IO/confirmation and a later commit.
    pub async fn begin_native_replacement_media(
        &mut self,
        command: &connect_protocol::transitions::TransitionCommand,
        root: &payloads::OperationUpdated,
        offer: payloads::ConnectionOffer,
    ) -> Result<(String, CommandScope)> {
        let (payload, scope) =
            replacement_offer(command, root, offer, "connect-replacement-media-v1")?;
        let id = self
            .send_value("connection.offer", payload, scope.clone())
            .await?;
        Ok((id, scope))
    }
    /// Answer a server native parameter request received through `receive`.
    /// Compare all resources with the caller's original offer scope. The fresh
    /// reply is signed locally; no server signature or revision header is copied.
    /// Successful submission is not media admission or a readiness grant.
    pub async fn answer_connection_offer(
        &mut self,
        request: &Envelope,
        payload: payloads::ConnectionAnswer,
        root: &CommandScope,
    ) -> Result<String> {
        let value = answer_frame(request, payload, root)?;
        let id = value["id"]
            .as_str()
            .ok_or(ClientError::Protocol)?
            .to_owned();
        self.write(value, true).await?;
        Ok(id)
    }
    /// Submit one framed RTP or clock-only SR/SDES datagram. Clock metadata does
    /// not grant media readiness. Negotiated clock support, hop-local handle and
    /// current permission are enforced by the server, not this send.
    pub fn send_native_media_datagram(&self, bytes: &[u8]) -> Result<()> {
        datagram(bytes)?;
        let mut current = self.pending()?;
        current.complete = true;
        if self
            .connection
            .max_datagram_size()
            .is_none_or(|limit| bytes.len() > limit)
        {
            return Err(ClientError::Protocol);
        }
        self.connection
            .send_datagram(bytes.to_vec().into())
            .map_err(|_| ClientError::Transport)
    }
    /// The SDK owns this atomic read alongside its independently owned signed reader.
    /// Idle media has no frame deadline; current peer authority still gates both ends.
    pub(crate) async fn receive_owned_native_datagram(&self) -> Result<Vec<u8>> {
        self.check_current()?;
        let bytes = self
            .connection
            .read_datagram()
            .await
            .map_err(|_| ClientError::Transport)?;
        self.check_current()?;
        if bytes.len() > MAX_NATIVE_MEDIA_DATAGRAM {
            return Err(ClientError::Protocol);
        }
        Ok(bytes.to_vec())
    }
    /// Own the client's single RTP/clock datagram read while borrowed. Inspect
    /// the returned original bytes with observe_native_media_datagram; clock-only
    /// reports never count as decoded media or readiness. Atomic timeout or
    /// cancellation leaves signaling and other streams usable.
    pub async fn receive_native_media_datagram(&mut self) -> Result<Vec<u8>> {
        let mut current = self.pending()?;
        current.complete = true;
        let remaining = (self.authenticated_until.min(self.server_key_expires_at)
            - chrono::Utc::now())
        .to_std()
        .unwrap_or_default();
        let bytes = tokio::time::timeout(
            self.deadline.min(remaining),
            self.connection.read_datagram(),
        )
        .await
        .map_err(|_| ClientError::Timeout)?
        .map_err(|_| ClientError::Transport)?;
        let mut current = self.pending()?;
        current.complete = true;
        datagram(&bytes)?;
        Ok(bytes.to_vec())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_wire_validator_accepts_clock_metadata_and_rtp_without_rewriting_either() {
        let report = SenderClockDatagram {
            stream_local_id: 7,
            seq: u32::MAX,
            report: SenderClockReport {
                ssrc: 41,
                ntp: 0x80000000ffffffff,
                rtp: u32::MAX - 10,
                packets: 9,
                octets: 321,
                cname: "common-π".into(),
            },
        };
        let clock = pack_sender_clock_datagram(&report).unwrap();
        datagram(&clock).unwrap();
        assert_eq!(
            observe_native_media_datagram(&clock).unwrap(),
            ObservedNativeMediaDatagram::Clock(report)
        );
        let media = vec![
            1, 0, 0, 7, 0, 0, 0, 9, 0x80, 0xe0, 0x12, 0x34, 0, 0, 0, 9, 0, 0, 0, 41, 0x10, 0xab,
        ];
        datagram(&media).unwrap();
        let ObservedNativeMediaDatagram::Rtp(parsed) =
            observe_native_media_datagram(&media).unwrap()
        else {
            panic!("RTP became clock")
        };
        assert!(parsed.rtp.packet().header.marker);
        assert_eq!(parsed.rtp.packet().header.sequence_number, 0x1234);
    }
    #[test]
    fn native_wire_validator_refuses_malformed_clocks_without_rtp_fallback() {
        let clock = pack_sender_clock_datagram(&SenderClockDatagram {
            stream_local_id: 7,
            seq: 9,
            report: SenderClockReport {
                ssrc: 41,
                ntp: 1,
                rtp: 0,
                packets: 1,
                octets: 3,
                cname: "clock".into(),
            },
        })
        .unwrap();
        for n in 0..clock.len() {
            assert!(
                matches!(datagram(&clock[..n]), Err(ClientError::Protocol)),
                "truncation {n}"
            );
        }
        let mut invalid = clock.to_vec();
        invalid[1] = 1;
        assert!(matches!(datagram(&invalid), Err(ClientError::Protocol)));
        invalid = clock.to_vec();
        invalid[40..44].copy_from_slice(&42u32.to_be_bytes());
        assert!(matches!(datagram(&invalid), Err(ClientError::Protocol)));
        invalid = clock.to_vec();
        invalid.resize(MAX_NATIVE_MEDIA_DATAGRAM + 1, 0);
        assert!(matches!(datagram(&invalid), Err(ClientError::Protocol)));
    }

    #[test]
    fn native_confirmation_is_fresh_and_binds_every_original_resource() {
        let mut value = envelope(
            "connection.offer",
            json!({"substrate":"quic","capabilities":{},"streams_offered":[{"id":"voice","kind":"audio","direction":"sendonly","codec_preferences":["opus"]}]}),
        );
        for (field, id) in [
            ("cid", "room"),
            ("sid", "session"),
            ("connid", "connection"),
            ("operation_id", "operation"),
        ] {
            value[field] = json!(id);
        }
        value["expected_revision"] = json!(0);
        let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).unwrap();
        let signer = Sig9421Signer::ed25519("server", &key.serialize_der()).unwrap();
        let request = Envelope::from_value(signer.sign(&value).unwrap()).unwrap();
        let answer=serde_json::from_value::<payloads::ConnectionAnswer>(json!({"substrate":"quic","capabilities":{},"streams_answered":[{"id":"voice","kind":"audio","direction":"recvonly","codec":{"name":"opus"}}]})).unwrap();
        let scope = CommandScope {
            conversation_id: Some("room".into()),
            session_id: Some("session".into()),
            connection_id: Some("connection".into()),
            operation_id: Some("operation".into()),
            expected_revision: Some(0),
        };
        let response = answer_frame(&request, answer.clone(), &scope).unwrap();
        assert_ne!(response["id"], value["id"]);
        assert_eq!(response["in_reply_to"], json!(request.id()));
        assert!(response.get("signature").is_none());
        assert!(response.get("expected_revision").is_none());
        for field in ["cid", "sid", "connid", "operation_id"] {
            let mut wrong = value.clone();
            wrong[field] = json!("foreign");
            let wrong = Envelope::from_value(wrong).unwrap();
            assert!(
                answer_frame(&wrong, answer.clone(), &scope).is_err(),
                "{field}"
            );
        }
        assert!(answer_frame(&request, answer, &CommandScope::default()).is_err());
    }
}
