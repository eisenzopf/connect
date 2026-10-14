//! Bounded RTP/VP8 frame assembly. A missing packet never becomes a complete
//! frame merely because the RTP marker for its final fragment arrived.
use crate::{ClientError, Result};
use connect_protocol::video_profile::{MAX_VIDEO_FRAME_BYTES, MAX_VIDEO_RTP_BYTES};
use std::fmt;

/// One independently validated complete video RTP packet.
#[derive(Clone, PartialEq, Eq)]
pub struct NativeVideoPacket {
    /// Complete RTP image, including its marker, sequence and timestamp.
    pub rtp: Vec<u8>,
    /// Source RTP sequence, independent of the UCTP datagram counter.
    pub sequence: u16,
    /// Frame time in the negotiated 90 kHz clock.
    pub timestamp_rtp: u32,
    /// Final RTP fragment of an encoded frame.
    pub marker: bool,
    /// Negotiated RTP payload type for this hop.
    pub payload_type: u8,
    /// RTP source identifier; assembly never combines different sources.
    pub ssrc: u32,
    pub(crate) payload_offset: usize,
}
impl fmt::Debug for NativeVideoPacket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeVideoPacket")
            .field("bytes", &self.rtp.len())
            .field("sequence", &self.sequence)
            .field("timestamp_rtp", &self.timestamp_rtp)
            .field("marker", &self.marker)
            .field("payload_type", &self.payload_type)
            .finish()
    }
}
impl NativeVideoPacket {
    /// Validate RTP framing and retain the original packet for forwarding.
    pub fn parse(rtp: Vec<u8>) -> Result<Self> {
        if rtp.len() < 13 || rtp.len() > MAX_VIDEO_RTP_BYTES || rtp[0] >> 6 != 2 {
            return Err(ClientError::Protocol);
        }
        let mut offset = 12 + usize::from(rtp[0] & 15) * 4;
        if offset > rtp.len() {
            return Err(ClientError::Protocol);
        }
        if rtp[0] & 0x10 != 0 {
            let extension = rtp.get(offset..offset + 4).ok_or(ClientError::Protocol)?;
            let words = usize::from(u16::from_be_bytes([extension[2], extension[3]]));
            offset = offset
                .checked_add(4 + words * 4)
                .ok_or(ClientError::Protocol)?;
        }
        if rtp[0] & 0x20 != 0 || offset >= rtp.len() {
            return Err(ClientError::Protocol);
        }
        Ok(Self {
            sequence: u16::from_be_bytes([rtp[2], rtp[3]]),
            timestamp_rtp: u32::from_be_bytes([rtp[4], rtp[5], rtp[6], rtp[7]]),
            marker: rtp[1] & 0x80 != 0,
            payload_type: rtp[1] & 0x7f,
            ssrc: u32::from_be_bytes([rtp[8], rtp[9], rtp[10], rtp[11]]),
            payload_offset: offset,
            rtp,
        })
    }
    pub(super) fn fragment(&self) -> Result<(bool, &[u8])> {
        if self.rtp.len() > MAX_VIDEO_RTP_BYTES {
            return Err(ClientError::Protocol);
        }
        let p = self
            .rtp
            .get(self.payload_offset..)
            .ok_or(ClientError::Protocol)?;
        let first = *p.first().ok_or(ClientError::Protocol)?;
        if first & 0x40 != 0 || first & 0x0f > 8 {
            return Err(ClientError::Protocol);
        }
        let mut n = 1;
        if first & 0x80 != 0 {
            let ext = *p.get(n).ok_or(ClientError::Protocol)?;
            n += 1;
            if ext & 0x80 != 0 {
                let id = *p.get(n).ok_or(ClientError::Protocol)?;
                n += if id & 0x80 != 0 { 2 } else { 1 };
            }
            if ext & 0x40 != 0 {
                n += 1;
            }
            if ext & 0x30 != 0 {
                n += 1;
            }
        }
        let data = p
            .get(n..)
            .filter(|d| !d.is_empty())
            .ok_or(ClientError::Protocol)?;
        Ok((first & 0x10 != 0 && first & 0x0f == 0, data))
    }
}

/// Packetize one encoded VP8 frame with a minimal RFC 7741 descriptor.
#[cfg(test)]
pub(crate) fn packetize(
    encoded: &[u8],
    timestamp: u32,
    next: &mut u16,
    ssrc: u32,
    pt: u8,
) -> Result<Vec<NativeVideoPacket>> {
    packetize_for_mtu(encoded, timestamp, next, ssrc, pt, MAX_VIDEO_RTP_BYTES)
}

pub(crate) fn packetize_for_mtu(
    encoded: &[u8],
    timestamp: u32,
    next: &mut u16,
    ssrc: u32,
    pt: u8,
    max_rtp_bytes: usize,
) -> Result<Vec<NativeVideoPacket>> {
    if encoded.is_empty()
        || encoded.len() > MAX_VIDEO_FRAME_BYTES
        || pt > 127
        || !(512..=MAX_VIDEO_RTP_BYTES).contains(&max_rtp_bytes)
    {
        return Err(ClientError::Protocol);
    }
    let chunks: Vec<_> = encoded.chunks(max_rtp_bytes - 13).collect();
    let mut packets = Vec::with_capacity(chunks.len());
    for (i, chunk) in chunks.iter().enumerate() {
        let marker = i + 1 == chunks.len();
        let mut rtp = vec![0x80, pt | if marker { 0x80 } else { 0 }];
        rtp.extend_from_slice(&next.to_be_bytes());
        *next = next.wrapping_add(1);
        rtp.extend_from_slice(&timestamp.to_be_bytes());
        rtp.extend_from_slice(&ssrc.to_be_bytes());
        rtp.push(if i == 0 { 0x10 } else { 0 });
        rtp.extend_from_slice(chunk);
        packets.push(NativeVideoPacket::parse(rtp)?);
    }
    Ok(packets)
}
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    #[test]
    fn rejects_oversized_and_truncated_packet_extensions() {
        assert!(NativeVideoPacket::parse(vec![0; MAX_VIDEO_RTP_BYTES + 1]).is_err());
        let mut packet = vec![0x90, 96, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 255, 255];
        packet.push(0x10);
        assert!(NativeVideoPacket::parse(packet).is_err());
    }
    #[test]
    fn caller_mutation_cannot_panic_the_fragment_parser() {
        let mut sequence = 0;
        let mut packets = packetize(&[2; 100], 90, &mut sequence, 1, 96).unwrap();
        packets[0].rtp.clear();
        assert!(matches!(packets[0].fragment(), Err(ClientError::Protocol)));
        packets[0].rtp.resize(MAX_VIDEO_RTP_BYTES + 1, 0);
        assert!(matches!(packets[0].fragment(), Err(ClientError::Protocol)));
    }
}
