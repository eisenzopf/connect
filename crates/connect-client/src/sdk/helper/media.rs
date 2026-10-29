//! Typed media IPC routing through the original managed Rust Client.
//! Local attachment never grants another peer's negotiated media bindings.
use super::*;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
mod channel;
pub(super) use channel::Channel;
pub const PROFILE: &str = "connect-sdk-media/2";
pub const CAPABILITY: &str = "managed-media-ipc-v2";
pub const WINDOW: usize = 16;

/// Private SDK-owned listener and one-use attachment token. Never Debug its token.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attachment {
    endpoint: SocketAddr,
    token: String,
}
impl Attachment {
    fn validate(&self) -> Result<()> {
        if !self.endpoint.ip().is_loopback()
            || self.endpoint.port() == 0
            || self.token.len() != 64
            || !self
                .token
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(Error::local(ErrorKind::Validation));
        }
        Ok(())
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub conversation_id: ConversationId,
    pub session_id: SessionId,
    pub connection_id: ConnectionId,
}
impl Binding {
    fn connection(&self, client: &Client) -> Connection {
        client
            .conversation_by_id(self.conversation_id.clone())
            .session_by_id(self.session_id.clone())
            .connection_by_id(self.connection_id.clone())
    }
    fn negotiation(&self, client: &Client) -> Result<NativeNegotiation> {
        self.connection(client)
            .native_negotiation()?
            .ok_or_else(|| Error::media(ErrorKind::Unsupported, false))
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub timeout_ms: u64,
    pub command: Command,
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Describe {
        binding: Binding,
    },
    SendRtp {
        binding: Binding,
        stream_id: StreamId,
        datagram_sequence: u32,
    },
    SendClock {
        binding: Binding,
        stream_id: StreamId,
        datagram_sequence: u32,
        report: Clock,
    },
    Receive {
        binding: Binding,
        stream_id: StreamId,
    },
    SendData {
        binding: Binding,
        stream_id: StreamId,
        message_id: MessageId,
        to: Vec<ParticipantId>,
        format: DataFormat,
        mutation: Mutation,
    },
    ReceiveData {
        binding: Binding,
        stream_id: StreamId,
    },
    DataStats {},
    Stats {},
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataFormat {
    Text,
    Json,
    Binary,
}
impl DataFormat {
    fn payload(self, bytes: Vec<u8>) -> Result<DataPayload> {
        let bad = || Error::local(ErrorKind::Validation);
        match self {
            Self::Text => Ok(DataPayload::Text(
                String::from_utf8(bytes).map_err(|_| bad())?,
            )),
            Self::Json => Ok(DataPayload::Json(
                serde_json::from_slice(&bytes).map_err(|_| bad())?,
            )),
            Self::Binary => Ok(DataPayload::Binary(bytes)),
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Clock {
    ssrc: u32,
    /// Decimal u64; JavaScript must not round the 64-bit NTP value.
    ntp: String,
    rtp: u32,
    packets: u32,
    octets: u32,
    cname: String,
}
impl TryFrom<Clock> for SenderClockReport {
    type Error = Error;
    fn try_from(v: Clock) -> Result<Self> {
        let ntp: u64 = v
            .ntp
            .parse()
            .map_err(|_| Error::media(ErrorKind::Validation, false))?;
        if ntp.to_string() != v.ntp || v.cname.is_empty() || v.cname.len() > 255 {
            return Err(Error::media(ErrorKind::Validation, false));
        }
        Ok(Self {
            ssrc: v.ssrc,
            ntp,
            rtp: v.rtp,
            packets: v.packets,
            octets: v.octets,
            cname: v.cname,
        })
    }
}
fn response(
    id: u32,
    kind: media_ipc::Kind,
    metadata: Value,
    payload: Vec<u8>,
) -> Result<media_ipc::Frame> {
    let metadata = serde_json::to_vec(&metadata).map_err(|_| Error::local(ErrorKind::Protocol))?;
    if metadata.len() > media_ipc::MAX_METADATA_BYTES
        || payload.len() > media_ipc::MAX_PAYLOAD_BYTES
    {
        return Err(Error::local(ErrorKind::Protocol));
    }
    Ok(media_ipc::Frame {
        kind,
        request_id: id,
        metadata,
        payload,
    })
}
pub(super) fn failure(id: u32, error: Error) -> media_ipc::Frame {
    media_ipc::Frame {
        kind: media_ipc::Kind::Failure,
        request_id: id,
        metadata: serde_json::to_vec(&error_json(&error)).unwrap(),
        payload: Vec::new(),
    }
}
fn decode(frame: &media_ipc::Frame) -> Result<Request> {
    if frame.kind != media_ipc::Kind::Request {
        return Err(Error::local(ErrorKind::Protocol));
    }
    let request: Request =
        serde_json::from_slice(&frame.metadata).map_err(|_| Error::local(ErrorKind::Validation))?;
    if !(1..=60_000).contains(&request.timeout_ms)
        || (!matches!(
            request.command,
            Command::SendRtp { .. } | Command::SendData { .. }
        ) && !frame.payload.is_empty())
        || (matches!(request.command, Command::SendRtp { .. }) && frame.payload.is_empty())
    {
        return Err(Error::local(ErrorKind::Validation));
    }
    Ok(request)
}
pub(super) async fn execute(
    client: Client,
    frame: media_ipc::Frame,
    stop: CancellationToken,
) -> media_ipc::Frame {
    let id = frame.request_id;
    let result = async {
        let request = decode(&frame)?;
        let options = RequestOptions { timeout:Duration::from_millis(request.timeout_ms), signal:stop };
        let mut kind = media_ipc::Kind::Response;
        let mut payload = Vec::new();
        let metadata = match request.command {
            Command::Describe { binding } => {
                let selected = binding.negotiation(&client)?;
                json!({"type":"negotiation","binding":binding,"source_clock":selected.source_clock,
                    "stream_revision":selected.stream_revision,
                    "streams":selected.streams.into_iter().map(|s|json!({"id":s.id,"medium":s.medium,
                        "direction":s.direction,"codec":s.codec,"stream_local_id":s.stream_local_id})).collect::<Vec<_>>()})
            }
            Command::SendRtp { binding, stream_id, datagram_sequence } => {
                binding.negotiation(&client)?;
                let stream = binding.connection(&client).stream_by_id(stream_id.clone());
                let accepted = stream.send_rtp(RtpPacket::new(datagram_sequence, frame.payload)?, options).await?;
                json!({"type":"submitted","binding":binding,"stream_id":stream_id,
                    "boundary":"local-media-queue","datagram_sequence":accepted.datagram_sequence})
            }
            Command::SendClock { binding, stream_id, datagram_sequence, report } => {
                binding.negotiation(&client)?;
                let accepted = binding.connection(&client).stream_by_id(stream_id.clone())
                    .send_clock(datagram_sequence, report.try_into()?, options).await?;
                json!({"type":"submitted","binding":binding,"stream_id":stream_id,
                    "boundary":"local-media-queue","datagram_sequence":accepted.datagram_sequence})
            }
            Command::Receive { binding, stream_id } => {
                binding.negotiation(&client)?;
                let packet = binding.connection(&client).stream_by_id(stream_id.clone()).receive_packet(options).await?;
                kind = media_ipc::Kind::Packet;
                let packet = match packet {
                    MediaPacket::Rtp(packet) => {
                        payload = packet.bytes().to_vec();let h = packet.header();
                        json!({"type":"rtp","datagram_sequence":packet.datagram_sequence,
                            "header":{"payload_type":h.payload_type,"sequence_number":h.sequence_number,
                                "timestamp":h.timestamp,"ssrc":h.ssrc,"marker":h.marker}})
                    }
                    MediaPacket::Clock { datagram_sequence, report:r } => json!({"type":"clock","datagram_sequence":datagram_sequence,
                        "report":{"ssrc":r.ssrc,"ntp":r.ntp.to_string(),"rtp":r.rtp,"packets":r.packets,"octets":r.octets,"cname":r.cname}}),
                    MediaPacket::Gap { dropped_packets } => json!({"type":"gap","dropped_packets":dropped_packets.to_string()}),
                };
                json!({"type":"packet","binding":binding,"stream_id":stream_id,"packet":packet})
            }
            Command::SendData { binding, stream_id, message_id, to, format, mutation } => {
                let key = OperationKey {
                    operation_id: mutation.operation_id.clone(),
                    command: client.operation_kinds().data_send.command().into(),
                    scope: OperationScope {
                        conversation_id: Some(binding.conversation_id.clone()),
                        session_id: Some(binding.session_id.clone()),
                        connection_id: Some(binding.connection_id.clone()),
                    },
                };
                let bad = |kind| Error::request(kind, Some(&key), false);
                binding.negotiation(&client).map_err(|e| bad(e.kind))?;
                let mut recipients = to.into_iter();
                let first = recipients.next().ok_or_else(|| bad(ErrorKind::Validation))?;
                let data = format.payload(frame.payload).map_err(|e| bad(e.kind))?;
                let operation = binding.connection(&client).stream_by_id(stream_id.clone())
                    .send_data(message_id, NonEmpty { first, rest: recipients.collect() }, data,
                        mutation.options(options)).await?;
                json!({"type":"data_submitted","binding":binding,"stream_id":stream_id,
                    "reference":operation.reference_json()})
            }
            Command::ReceiveData { binding, stream_id } => {
                binding.negotiation(&client)?;
                let received = binding.connection(&client).stream_by_id(stream_id.clone()).receive_data(options).await?;
                let format = match received.payload {
                    DataPayload::Text(text) => { payload = text.into_bytes(); "text" }
                    DataPayload::Json(value) => { payload = serde_json::to_vec(&value).map_err(|_| Error::local(ErrorKind::Protocol))?; "json" }
                    DataPayload::Binary(bytes) => { payload = bytes; "binary" }
                };
                kind = media_ipc::Kind::Packet;
                json!({"type":"data","binding":binding,"stream_id":stream_id,
                    "message_id":received.message_id,"operation_id":received.operation_id,
                    "from":received.from,"format":format})
            }
            Command::DataStats {} => {
                let s = client.data_stats()?;
                json!({"type":"data_stats","received_frames":s.received_frames.to_string(),
                    "accepted_messages":s.accepted_messages.to_string(),"malformed_frames":s.malformed_frames.to_string(),
                    "unowned_frames":s.unowned_frames.to_string(),"overflow_drops":s.overflow_drops.to_string(),
                    "binding_change_drops":s.binding_change_drops.to_string(),"queued_messages":s.queued_messages,"queued_bytes":s.queued_bytes})
            }
            Command::Stats {} => {
                let s = client.media_stats()?;
                json!({"type":"stats","received_datagrams":s.received_datagrams.to_string(),
                    "rtp_packets":s.rtp_packets.to_string(),"clock_reports":s.clock_reports.to_string(),
                    "malformed_datagrams":s.malformed_datagrams.to_string(),"unknown_handles":s.unknown_handles.to_string(),
                    "overflow_drops":s.overflow_drops.to_string(),"control_drops":s.control_drops.to_string(),
                    "binding_change_drops":s.binding_change_drops.to_string(),"queued_packets":s.queued_packets,"queued_bytes":s.queued_bytes})
            }
        };
        response(id, kind, metadata, payload)
    }.await;
    result.unwrap_or_else(|error| failure(id, error))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typed_data_formats_validate_utf8_json_and_allow_empty_binary_without_echo() {
        assert!(
            matches!(DataFormat::Text.payload(vec![0xff]), Err(e) if e.kind == ErrorKind::Validation)
        );
        assert!(DataFormat::Json.payload(b"{bad}".to_vec()).is_err());
        assert_eq!(
            DataFormat::Binary.payload(Vec::new()).unwrap(),
            DataPayload::Binary(vec![])
        );
        assert_eq!(
            DataFormat::Text
                .payload("native 🦀".as_bytes().to_vec())
                .unwrap(),
            DataPayload::Text("native 🦀".into())
        );
        assert_eq!(
            DataFormat::Json
                .payload(br#"{"native":true}"#.to_vec())
                .unwrap(),
            DataPayload::Json(json!({"native":true}))
        );
    }
    #[test]
    fn attachment_refuses_remote_zero_port_and_noncanonical_tokens_before_connect() {
        for endpoint in ["192.0.2.1:443", "127.0.0.1:0", "[::]:9000"] {
            assert!(Attachment {
                endpoint: endpoint.parse().unwrap(),
                token: "a".repeat(64)
            }
            .validate()
            .is_err());
        }
        for token in ["a".repeat(63), "A".repeat(64), "é".repeat(32)] {
            assert!(Attachment {
                endpoint: "127.0.0.1:9000".parse().unwrap(),
                token
            }
            .validate()
            .is_err());
        }
        Attachment {
            endpoint: "[::1]:9000".parse().unwrap(),
            token: "a".repeat(64),
        }
        .validate()
        .unwrap();
    }
    #[test]
    fn packet_commands_refuse_unknown_fields_deadlines_and_unexpected_binary_bodies() {
        let valid = json!({"timeout_ms":4000,"command":{"action":"stats"}});
        let frame = |value: Value, payload: Vec<u8>| media_ipc::Frame {
            kind: media_ipc::Kind::Request,
            request_id: 2,
            metadata: serde_json::to_vec(&value).unwrap(),
            payload,
        };
        decode(&frame(valid.clone(), Vec::new())).unwrap();
        assert!(decode(&frame(valid.clone(), vec![1])).is_err());
        for timeout in [0, 60_001] {
            let mut value = valid.clone();
            value["timeout_ms"] = json!(timeout);
            assert!(decode(&frame(value, Vec::new())).is_err());
        }
        let mut value = valid;
        value["command"]["credential"] = json!("private");
        assert!(decode(&frame(value, Vec::new())).is_err());
    }
    #[test]
    fn clock_metadata_preserves_exact_u64_and_refuses_lossy_or_noncanonical_values() {
        let clock = |ntp: &str| Clock {
            ssrc: 1,
            ntp: ntp.into(),
            rtp: 2,
            packets: 3,
            octets: 4,
            cname: "source".into(),
        };
        assert_eq!(
            SenderClockReport::try_from(clock("18446744073709551615"))
                .unwrap()
                .ntp,
            u64::MAX
        );
        for value in ["18446744073709551616", "01", "+1", "1.0", "1e3"] {
            assert!(SenderClockReport::try_from(clock(value)).is_err());
        }
    }
}
