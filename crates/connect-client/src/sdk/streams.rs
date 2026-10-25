//! Scoped Stream observations and explicit bounded native packet I/O.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct StreamKey {
    pub conversation_id: ConversationId,
    pub session_id: SessionId,
    pub connection_id: ConnectionId,
    pub stream_id: StreamId,
}
#[derive(Clone)]
pub struct Stream {
    pub(super) client: Client,
    pub(super) key: StreamKey,
}
impl std::fmt::Debug for Stream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stream")
            .field("id", &self.key.stream_id)
            .field("connection_id", &self.key.connection_id)
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaOwnership {
    Observation,
    LocalSource,
    LocalSink,
    LocalDuplex,
}
#[derive(Clone, Debug)]
pub struct MediaFormat {
    pub codec: String,
    pub clock_rate: u32,
    pub channels: Option<u16>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}
#[derive(Clone, Debug)]
pub struct SourceClock {
    pub source_id: String,
    pub generation: u64,
    pub units_per_second: u64,
    pub synchronized: bool,
}
#[derive(Clone, Debug)]
pub struct StreamSnapshot {
    pub id: StreamId,
    pub conversation_id: ConversationId,
    pub session_id: SessionId,
    pub connection_id: ConnectionId,
    pub medium: Medium,
    pub state: String,
    pub revision: StreamResourceRevision,
    pub direction: Option<Direction>,
    pub format: Option<MediaFormat>,
    pub clock: Option<SourceClock>,
    pub ownership: MediaOwnership,
}
impl StreamSnapshot {
    pub(super) fn from_wire(
        v: connect_protocol::events::StreamState,
        cid: &ConversationId,
    ) -> Result<Option<Self>> {
        let fail = || Error::request(ErrorKind::Protocol, None, false);
        let Some(connection) = v.connection_id else {
            return Ok(None);
        }; // Other journal streams have no Connection ownership.
        let medium = serde_json::from_value(serde_json::json!(v.medium)).map_err(|_| fail())?;
        Ok(Some(Self {
            id: v.stream_id.parse().map_err(|_| fail())?,
            conversation_id: cid.clone(),
            session_id: v.session_id.parse().map_err(|_| fail())?,
            connection_id: connection.parse().map_err(|_| fail())?,
            medium,
            state: v.state,
            revision: StreamResourceRevision::try_from(v.revision).map_err(|_| fail())?,
            direction: None,
            format: None,
            clock: None,
            ownership: MediaOwnership::Observation,
        }))
    }
}
impl Connection {
    pub fn stream_by_id(&self, id: StreamId) -> Stream {
        Stream {
            client: self.client().clone(),
            key: StreamKey {
                conversation_id: self.conversation_id().clone(),
                session_id: self.session_id().clone(),
                connection_id: self.id().clone(),
                stream_id: id,
            },
        }
    }
}
impl Stream {
    pub fn id(&self) -> &StreamId {
        &self.key.stream_id
    }
    pub fn connection_id(&self) -> &ConnectionId {
        &self.key.connection_id
    }
    pub fn session_id(&self) -> &SessionId {
        &self.key.session_id
    }
    pub fn conversation_id(&self) -> &ConversationId {
        &self.key.conversation_id
    }
    /// Read the server's current Stream projection. Servers without Stream
    /// snapshot rows return ReceiptUnavailable; local parameters are not used
    /// to manufacture a current stream state.
    pub async fn refresh(&self, options: RequestOptions) -> Result<StreamSnapshot> {
        let current = snapshot::read(&self.client, &self.key.conversation_id, options)
            .await?
            .streams
            .remove(&self.key.stream_id)
            .ok_or_else(|| Error::request(ErrorKind::ReceiptUnavailable, None, false))?;
        if current.connection_id != self.key.connection_id
            || current.session_id != self.key.session_id
        {
            return Err(Error::request(ErrorKind::Protocol, None, false));
        }
        Ok(current)
    }
    /// Submit exactly these RTP bytes on this peer's negotiated binding. A
    /// successful return is local transport queue admission, never delivery.
    pub async fn send_rtp(
        &self,
        packet: RtpPacket,
        options: RequestOptions,
    ) -> Result<LocalMediaSubmission> {
        self.client
            .shared
            .send_media(self.key.clone(), MediaPacket::Rtp(packet), options)
            .await
    }
    /// Counters, RTP/NTP samples and CNAME describe the caller's actual source.
    /// A clock report cannot establish readiness or stand in for a media packet.
    pub async fn send_clock(
        &self,
        sequence: u32,
        report: SenderClockReport,
        options: RequestOptions,
    ) -> Result<LocalMediaSubmission> {
        self.client
            .shared
            .send_media(
                self.key.clone(),
                MediaPacket::Clock {
                    datagram_sequence: sequence,
                    report,
                },
                options,
            )
            .await
    }
    /// Cloned handles consume one shared per-stream queue. Cancellation and
    /// timeout leave packets and other streams/control waits owned and usable.
    pub async fn receive_packet(&self, options: RequestOptions) -> Result<MediaPacket> {
        self.client.shared.receive_media(&self.key, options).await
    }
}
pub use crate::canonical::SenderClockReport;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RtpHeader {
    pub payload_type: u8,
    pub sequence_number: u16,
    pub timestamp: u32,
    pub ssrc: u32,
    pub marker: bool,
}
#[derive(Clone)]
pub struct RtpPacket {
    pub datagram_sequence: u32,
    bytes: Vec<u8>,
    header: RtpHeader,
    payload_start: usize,
    payload_len: usize,
}
impl std::fmt::Debug for RtpPacket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RtpPacket")
            .field("datagram_sequence", &self.datagram_sequence)
            .field("header", &self.header)
            .field("bytes", &self.bytes.len())
            .finish()
    }
}
impl RtpPacket {
    /// Validate a complete RTP packet without rewriting marker, CSRC,
    /// extension, padding, sequence, timestamp or payload bytes.
    pub fn new(datagram_sequence: u32, bytes: Vec<u8>) -> Result<Self> {
        let fail = || Error::media(ErrorKind::Validation, false);
        if bytes.len() > 16 * 1024 + 12 {
            return Err(fail());
        }
        let framed = frame(1, datagram_sequence, &bytes);
        let crate::canonical::ObservedNativeMediaDatagram::Rtp(packet) =
            crate::canonical::observe_native_media_datagram(&framed).map_err(|_| fail())?
        else {
            return Err(fail());
        };
        let rtp = packet.rtp.packet();
        let h = &rtp.header;
        let header = RtpHeader {
            payload_type: h.payload_type,
            sequence_number: h.sequence_number,
            timestamp: h.timestamp,
            ssrc: h.ssrc,
            marker: h.marker,
        };
        let payload_len = rtp.payload.len();
        let payload_start = bytes
            .len()
            .checked_sub(payload_len + usize::from(rtp.padding_size))
            .ok_or_else(fail)?;
        Ok(Self {
            datagram_sequence,
            bytes,
            header,
            payload_start,
            payload_len,
        })
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn payload(&self) -> &[u8] {
        &self.bytes[self.payload_start..self.payload_start + self.payload_len]
    }
    pub fn header(&self) -> RtpHeader {
        self.header
    }
}
#[derive(Clone, Debug)]
pub enum MediaPacket {
    Rtp(RtpPacket),
    Clock {
        datagram_sequence: u32,
        report: SenderClockReport,
    },
    /// Packets dropped by the local SDK queue, after all older queued packets.
    /// Network loss remains separately observable through RTP/datagram sequence.
    Gap {
        dropped_packets: u64,
    },
}
#[derive(Clone, Copy, Debug)]
pub struct LocalMediaSubmission {
    pub datagram_sequence: u32,
}
pub(super) fn frame(handle: u16, sequence: u32, rtp: &[u8]) -> Vec<u8> {
    let mut wire = Vec::with_capacity(rtp.len() + 8);
    wire.extend_from_slice(&[1, 0]);
    wire.extend_from_slice(&handle.to_be_bytes());
    wire.extend_from_slice(&sequence.to_be_bytes());
    wire.extend_from_slice(rtp);
    wire
}
