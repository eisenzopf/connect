//! Authentication is performed by the owned signaling reader. This module
//! joins its notices to the selected allocation and the actual packet boundary.
use super::*;
use crate::canonical::{Envelope, Scope};

/// Authenticated receive history attached only after a packet's in-band epoch,
/// SSRC and original native sequence agree with the signed allocation notice.
/// This describes decoder history, and grants no media or feedback permission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoReceiveEpoch {
    pub epoch: String,
    pub incarnation: String,
    pub revision: u64,
    pub ssrc: u32,
    pub first_sequence: u32,
    pub first_timestamp: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Role {
    Publisher,
    Receiver,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    profile: String,
    strm_id: StreamId,
    stream_local_id: u16,
    original_operation: OperationId,
    original_offer: String,
    incarnation: String,
    stream_generation: u64,
    connection_generation: u64,
    revision: u64,
    direction: Role,
    epoch: String,
    ssrc: u32,
    first_sequence: u32,
    first_timestamp: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct Notice {
    pub key: streams::StreamKey,
    pub allocation: NativeVideoFeedback,
    pub handle: u16,
    pub role: Role,
    pub history: VideoReceiveEpoch,
    pub tag: u128,
}

fn nonzero_hex(value: &str) -> Option<u128> {
    if value.len() != 32
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    u128::from_str_radix(value, 16).ok().filter(|v| *v != 0)
}

impl Notice {
    /// Called only with a frame from this physical client's signature-verifying
    /// reader. A payload alone, a journal record or another client is not input.
    pub fn read(frame: &Envelope) -> Result<Self> {
        let bad = || Error::local(ErrorKind::Protocol);
        if frame.kind() != "stream.epoch"
            || frame.scope(Scope::Reply).is_some()
            || frame.scope(Scope::Operation).is_some()
        {
            return Err(bad());
        }
        let w: Wire =
            serde_json::from_value(frame.original()["payload"].clone()).map_err(|_| bad())?;
        if w.profile != PROFILE
            || w.stream_local_id == 0
            || w.ssrc == 0
            || !(1..=MAX_SAFE).contains(&w.stream_generation)
            || !(1..=MAX_SAFE).contains(&w.connection_generation)
            || !(1..=MAX_SAFE).contains(&w.revision)
            || crate::canonical::Identifier::try_from(w.original_offer.clone()).is_err()
            || nonzero_hex(&w.incarnation).is_none()
        {
            return Err(bad());
        }
        let tag = nonzero_hex(&w.epoch).ok_or_else(bad)?;
        Ok(Self {
            key: streams::StreamKey {
                conversation_id: frame.scope(Scope::Conversation).ok_or_else(bad)?.parse()?,
                session_id: frame.scope(Scope::Session).ok_or_else(bad)?.parse()?,
                connection_id: frame.scope(Scope::Connection).ok_or_else(bad)?.parse()?,
                stream_id: w.strm_id,
            },
            allocation: NativeVideoFeedback {
                profile: w.profile,
                original_operation: w.original_operation,
                original_offer: w.original_offer,
                stream_generation: w.stream_generation,
                connection_generation: w.connection_generation,
            },
            handle: w.stream_local_id,
            role: w.direction,
            history: VideoReceiveEpoch {
                epoch: w.epoch,
                incarnation: w.incarnation,
                revision: w.revision,
                ssrc: w.ssrc,
                first_sequence: w.first_sequence,
                first_timestamp: w.first_timestamp,
            },
            tag,
        })
    }

    pub fn matches(&self, selected: &NativeNegotiation) -> bool {
        selected.video_feedback.get(&self.key.stream_id) == Some(&self.allocation)
            && selected.streams.iter().any(|s| self.matches_stream(s))
    }

    pub fn matches_stream(&self, stream: &NegotiatedStream) -> bool {
        stream.id == self.key.stream_id
            && stream.medium == Medium::Video
            && stream.stream_local_id == Some(self.handle)
            && match self.role {
                Role::Receiver => {
                    matches!(stream.direction, Direction::Recvonly | Direction::Sendrecv)
                }
                Role::Publisher => {
                    matches!(stream.direction, Direction::Sendonly | Direction::Sendrecv)
                }
            }
    }

    pub fn admits(&self, packet: &RtpPacket) -> bool {
        self.role == Role::Receiver
            && packet.video_tag == Some(self.tag)
            && packet.header().ssrc == self.history.ssrc
            && packet
                .datagram_sequence
                .wrapping_sub(self.history.first_sequence)
                < (1 << 31)
            && (packet.datagram_sequence != self.history.first_sequence
                || packet.header().timestamp == self.history.first_timestamp)
    }

    pub fn definitely_stale(&self, packet: &RtpPacket) -> bool {
        // Unknown future epochs wait for their own notice. A known epoch with
        // the wrong boundary/SSRC, or an older native sequence, cannot qualify.
        !self.admits(packet)
            && (packet.video_tag == Some(self.tag)
                || packet
                    .datagram_sequence
                    .wrapping_sub(self.history.first_sequence)
                    >= (1 << 31))
    }
}

#[cfg(test)]
pub(crate) mod tests;
