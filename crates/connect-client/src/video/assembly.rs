//! Bounded VP8 reordering and frame assembly. Missing packet sequences cannot
//! become a complete frame merely because its final marker arrived.
use super::NativeVideoPacket;
use crate::{ClientError, Result};
use connect_protocol::video_profile::{
    MAX_VIDEO_FRAME_BYTES, VIDEO_ASSEMBLY_MS, VIDEO_QUEUE_PACKETS,
};
use std::{
    collections::{HashMap, VecDeque},
    fmt,
    time::{Duration, Instant},
};
const MAX_INCOMPLETE_FRAMES: usize = 8;

/// Observed RTP packet range for one contiguous completed encoded frame.
/// Sequence numbers wrap at 16 bits; the range includes both endpoints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoSequenceRange {
    /// RTP source synchronization identifier observed on the frame's packets.
    pub ssrc: u32,
    /// Sequence number of the frame's start packet.
    pub first: u16,
    /// Sequence number of the frame's final marker packet.
    pub last: u16,
}

/// Numeric assembly observations; no media, source addresses or credentials.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct VideoAssemblyDiagnostics {
    /// Currently retained incomplete frames.
    pub pending_frames: usize,
    /// Out-of-order fragments currently retained across all frames.
    pub queued_fragments: usize,
    /// Maximum retained frames observed by this assembler.
    pub peak_pending_frames: usize,
    /// Maximum retained fragments observed by this assembler.
    pub peak_queued_fragments: usize,
    /// Frames discarded for loss, conflict or capacity.
    pub discarded_frames: u64,
    /// Incomplete frames removed at the assembly deadline.
    pub expired_frames: u64,
    /// Complete start-to-marker frames emitted once.
    pub completed_frames: u64,
}

/// Completed encoded VP8 frame for an independent application-owned decoder.
#[derive(Clone, PartialEq, Eq)]
pub struct NativeVideoFrame {
    /// Encoded source frame timestamp in the negotiated 90 kHz clock.
    pub timestamp_rtp: u32,
    /// VP8 uncompressed frame tag identifies a keyframe.
    pub keyframe: bool,
    /// Complete bounded VP8 frame, without RTP payload descriptors.
    pub encoded: Vec<u8>,
}
impl fmt::Debug for NativeVideoFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeVideoFrame")
            .field("timestamp_rtp", &self.timestamp_rtp)
            .field("keyframe", &self.keyframe)
            .field("bytes", &self.encoded.len())
            .finish()
    }
}
struct Fragment {
    bytes: Vec<u8>,
    marker: bool,
}
struct Pending {
    source: (u32, u32),
    started: Instant,
    first: Option<u16>,
    next: Option<u16>,
    bytes: Vec<u8>,
    fragments: HashMap<u16, Fragment>,
}
/// At most eight incomplete frames and 64 reordered packets across all frames.
/// Partial assemblies expire after 200 ms; accumulated bytes are independently
/// bounded to 256 KiB per frame. Completed frame duplicates are ignored.
#[derive(Default)]
pub struct VideoFrameAssembler {
    pending: VecDeque<Pending>,
    completed: VecDeque<(u32, u32)>,
    retired: VecDeque<(u32, u32)>,
    diagnostics: VideoAssemblyDiagnostics,
    completed_sequence: Option<VideoSequenceRange>,
}
impl VideoFrameAssembler {
    /// Packet range for the frame returned by the latest [`Self::push`].
    /// Returns `None` when that push returned no frame or an error. Applications
    /// can retain this numeric metadata beside a frame to check decoder continuity
    /// after their own bounded reordering and playout; it contains no media bytes.
    pub fn completed_sequence_range(&self) -> Option<VideoSequenceRange> {
        self.completed_sequence
    }
    /// Observe bounded assembly usage. Historical peaks include every insertion,
    /// rather than sampling only the queue depth after a completed frame.
    pub fn diagnostics(&self) -> VideoAssemblyDiagnostics {
        VideoAssemblyDiagnostics {
            pending_frames: self.pending.len(),
            queued_fragments: self.pending.iter().map(|p| p.fragments.len()).sum(),
            ..self.diagnostics
        }
    }
    /// Earliest incomplete-frame expiry. Schedule this deadline when no new
    /// packets arrive; this codec-independent assembler owns no background task.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.pending
            .iter()
            .map(|frame| frame.started + Duration::from_millis(VIDEO_ASSEMBLY_MS))
            .min()
    }
    /// Release expired partials during mute/stall as well as on packet arrival.
    /// Applications should call this at [`Self::next_deadline`].
    pub fn expire_stale(&mut self) {
        let mut index = 0;
        while index < self.pending.len() {
            if self.pending[index].started.elapsed() > Duration::from_millis(VIDEO_ASSEMBLY_MS) {
                if let Some(expired) = self.pending.remove(index) {
                    self.retire(expired.source);
                    self.diagnostics.expired_frames += 1;
                }
            } else {
                index += 1;
            }
        }
    }
    /// Accept reordered fragments but yield only a contiguous start-to-marker
    /// frame. Callers should decode completed frames in their RTP timestamp order.
    pub fn push(&mut self, packet: &NativeVideoPacket) -> Result<Option<NativeVideoFrame>> {
        self.completed_sequence = None;
        self.expire_stale();
        let source = (packet.ssrc, packet.timestamp_rtp);
        if self.completed.contains(&source)
            || self.retired.iter().any(|(ssrc, timestamp)| {
                *ssrc == source.0 && source.1.wrapping_sub(*timestamp) as i32 <= 0
            })
        {
            return Ok(None);
        }
        let (start, data) = packet.fragment()?;
        let index = if let Some(i) = self.pending.iter().position(|p| p.source == source) {
            i
        } else {
            while self.pending.len() >= MAX_INCOMPLETE_FRAMES {
                if let Some(discarded) = self.pending.pop_front() {
                    self.retire(discarded.source);
                }
                self.diagnostics.discarded_frames += 1;
            }
            self.pending.push_back(Pending {
                source,
                started: Instant::now(),
                first: None,
                next: None,
                bytes: Vec::new(),
                fragments: HashMap::new(),
            });
            self.pending.len() - 1
        };
        self.diagnostics.peak_pending_frames =
            self.diagnostics.peak_pending_frames.max(self.pending.len());
        let p = self.pending.get_mut(index).ok_or(ClientError::Protocol)?;
        if start {
            if p.first.is_some_and(|seq| seq != packet.sequence) {
                self.discard(index);
                return Ok(None);
            }
            if p.first.is_none() {
                p.first = Some(packet.sequence);
                p.next = Some(packet.sequence);
            }
        }
        if let Some(next) = p.next {
            let distance = packet.sequence.wrapping_sub(next);
            if distance >= 32768 {
                return Ok(None);
            } // A previously consumed duplicate/late packet.
            if usize::from(distance) >= VIDEO_QUEUE_PACKETS {
                self.discard(index);
                return Ok(None);
            }
        }
        if let Some(existing) = p.fragments.get(&packet.sequence) {
            if existing.bytes != data || existing.marker != packet.marker {
                self.discard(index);
            }
            return Ok(None);
        }
        if self
            .pending
            .iter()
            .map(|p| p.fragments.len())
            .sum::<usize>()
            >= VIDEO_QUEUE_PACKETS
        {
            self.discard(index);
            return Ok(None);
        }
        let p = self.pending.get_mut(index).ok_or(ClientError::Protocol)?;
        p.fragments.insert(
            packet.sequence,
            Fragment {
                bytes: data.to_vec(),
                marker: packet.marker,
            },
        );
        self.diagnostics.peak_queued_fragments = self
            .diagnostics
            .peak_queued_fragments
            .max(self.pending.iter().map(|p| p.fragments.len()).sum());
        let Some(p) = self.pending.iter_mut().find(|p| p.source == source) else {
            return Ok(None);
        };
        while let Some(next) = p.next {
            let Some(fragment) = p.fragments.remove(&next) else {
                break;
            };
            if p.bytes.len() + fragment.bytes.len() > MAX_VIDEO_FRAME_BYTES {
                let i = self
                    .pending
                    .iter()
                    .position(|p| p.source == source)
                    .ok_or(ClientError::Protocol)?;
                self.discard(i);
                return Ok(None);
            }
            p.bytes.extend_from_slice(&fragment.bytes);
            p.next = Some(next.wrapping_add(1));
            if fragment.marker {
                let i = self
                    .pending
                    .iter()
                    .position(|p| p.source == source)
                    .ok_or(ClientError::Protocol)?;
                let p = self.pending.remove(i).ok_or(ClientError::Protocol)?;
                let range = VideoSequenceRange {
                    ssrc: p.source.0,
                    first: p.first.ok_or(ClientError::Protocol)?,
                    last: next,
                };
                self.remember(source);
                self.diagnostics.completed_frames += 1;
                self.completed_sequence = Some(range);
                return Ok(Some(NativeVideoFrame {
                    timestamp_rtp: packet.timestamp_rtp,
                    keyframe: p.bytes.first().is_some_and(|b| b & 1 == 0),
                    encoded: p.bytes,
                }));
            }
        }
        Ok(None)
    }
    fn remember(&mut self, source: (u32, u32)) {
        while self.completed.len() >= MAX_INCOMPLETE_FRAMES {
            self.completed.pop_front();
        }
        self.completed.push_back(source);
    }
    fn discard(&mut self, index: usize) {
        if let Some(p) = self.pending.remove(index) {
            self.remember(p.source);
            self.retire(p.source);
            self.diagnostics.discarded_frames += 1;
        }
    }
    // One monotonic RTP watermark per recently observed source prevents a late
    // fragment from restarting an expired frame's deadline. Source history is
    // bounded just like concurrent assembly; timestamp wrap remains supported.
    fn retire(&mut self, source: (u32, u32)) {
        if let Some((_, timestamp)) = self.retired.iter_mut().find(|(ssrc, _)| *ssrc == source.0) {
            if source.1.wrapping_sub(*timestamp) as i32 > 0 {
                *timestamp = source.1;
            }
        } else {
            while self.retired.len() >= MAX_INCOMPLETE_FRAMES {
                self.retired.pop_front();
            }
            self.retired.push_back(source);
        }
    }
}
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::video::packet::packetize;
    #[test]
    fn reordered_wrap_and_duplicate_packets_complete_once_without_losing_bytes() {
        let mut seq = u16::MAX;
        let packets = packetize(&vec![6; 3000], 90, &mut seq, 42, 96).unwrap();
        let mut a = VideoFrameAssembler::default();
        assert!(a.push(&packets[2]).unwrap().is_none());
        assert_eq!(a.completed_sequence_range(), None);
        assert!(a.push(&packets[0]).unwrap().is_none());
        assert!(a.push(&packets[0]).unwrap().is_none());
        assert_eq!(a.push(&packets[1]).unwrap().unwrap().encoded, vec![6; 3000]);
        assert_eq!(
            a.completed_sequence_range(),
            Some(VideoSequenceRange {
                ssrc: 42,
                first: u16::MAX,
                last: 1
            })
        );
        for packet in &packets {
            assert!(a.push(packet).unwrap().is_none());
            assert_eq!(a.completed_sequence_range(), None);
        }
    }
    #[test]
    fn sequence_metadata_uses_buffered_marker_and_clears_before_an_incomplete_or_invalid_push() {
        let mut sequence = 100;
        let packets = packetize(&vec![6; 3000], 90, &mut sequence, 7, 96).unwrap();
        let mut assembler = VideoFrameAssembler::default();
        // The packet completing assembly is the middle fragment, not the marker.
        assert!(assembler.push(&packets[2]).unwrap().is_none());
        assert!(assembler.push(&packets[0]).unwrap().is_none());
        assert!(assembler.push(&packets[1]).unwrap().is_some());
        assert_eq!(
            assembler.completed_sequence_range(),
            Some(VideoSequenceRange {
                ssrc: 7,
                first: 100,
                last: 102,
            })
        );
        let next = packetize(&vec![6; 3000], 180, &mut sequence, 7, 96).unwrap();
        assert!(assembler.push(&next[0]).unwrap().is_none());
        assert_eq!(assembler.completed_sequence_range(), None);
        let complete = packetize(&[4; 100], 270, &mut sequence, 9, 96).unwrap();
        assert!(assembler.push(&complete[0]).unwrap().is_some());
        assert_eq!(assembler.completed_sequence_range().unwrap().ssrc, 9);
        let mut invalid = next[1].clone();
        invalid.rtp.clear();
        assert!(assembler.push(&invalid).is_err());
        assert_eq!(assembler.completed_sequence_range(), None);
    }
    #[test]
    fn marker_without_a_start_packet_cannot_publish_sequence_metadata() {
        let mut sequence = 30;
        let packets = packetize(&vec![6; 3000], 90, &mut sequence, 7, 96).unwrap();
        let mut assembler = VideoFrameAssembler::default();
        for packet in packets.iter().skip(1).rev() {
            assert!(assembler.push(packet).unwrap().is_none());
            assert_eq!(assembler.completed_sequence_range(), None);
        }
        assert_eq!(assembler.diagnostics().completed_frames, 0);
    }
    #[test]
    fn a_missing_fragment_and_a_conflicting_duplicate_never_complete() {
        let mut seq = 0;
        let packets = packetize(&vec![6; 3000], 90, &mut seq, 42, 96).unwrap();
        let mut a = VideoFrameAssembler::default();
        a.push(&packets[0]).unwrap();
        assert!(a.push(&packets[2]).unwrap().is_none());
        let mut conflicting = packets[2].clone();
        *conflicting.rtp.last_mut().unwrap() = 7;
        assert!(a.push(&conflicting).unwrap().is_none());
        assert!(a.push(&packets[1]).unwrap().is_none());
    }
    #[test]
    fn sources_and_timestamps_cannot_contaminate_each_other() {
        let mut seq = 0;
        let a = packetize(&vec![2; 2000], 90, &mut seq, 1, 96).unwrap();
        let mut seq = 0;
        let b = packetize(&vec![4; 2000], 90, &mut seq, 2, 96).unwrap();
        let mut assembler = VideoFrameAssembler::default();
        assembler.push(&a[0]).unwrap();
        assembler.push(&b[1]).unwrap();
        assert_eq!(
            assembler.push(&b[0]).unwrap().unwrap().encoded,
            vec![4; 2000]
        );
        assert_eq!(
            assembler.push(&a[1]).unwrap().unwrap().encoded,
            vec![2; 2000]
        );
        let measured = assembler.diagnostics();
        assert_eq!(measured.completed_frames, 2);
        assert_eq!(measured.peak_pending_frames, 2);
        assert_eq!(measured.pending_frames, 0);
        assert_eq!(measured.queued_fragments, 0);
    }
    #[test]
    fn many_partial_sources_keep_observed_usage_inside_frozen_limits() {
        let mut assembler = VideoFrameAssembler::default();
        for source in 1..=20 {
            let mut seq = 0;
            let fragments = packetize(&vec![2; 30_000], 90, &mut seq, source, 96).unwrap();
            for packet in fragments.iter().skip(1).take(10) {
                assert!(assembler.push(packet).unwrap().is_none());
                let observed = assembler.diagnostics();
                assert!(observed.pending_frames <= MAX_INCOMPLETE_FRAMES);
                assert!(observed.queued_fragments <= VIDEO_QUEUE_PACKETS);
            }
        }
        let measured = assembler.diagnostics();
        assert!(measured.discarded_frames > 0);
        assert!(measured.peak_pending_frames <= MAX_INCOMPLETE_FRAMES);
        assert!(measured.peak_queued_fragments <= VIDEO_QUEUE_PACKETS);
        assert_eq!(measured.completed_frames, 0);
    }
    #[test]
    fn expired_frame_cannot_restart_its_deadline_or_reconstruct_from_late_fragments() {
        let mut sequence = 0;
        let old = packetize(&vec![2; 3000], u32::MAX - 1, &mut sequence, 1, 96).unwrap();
        let mut assembler = VideoFrameAssembler::default();
        assembler.push(&old[0]).unwrap();
        assembler.pending[0].started =
            Instant::now() - Duration::from_millis(VIDEO_ASSEMBLY_MS + 1);
        assert!(assembler.next_deadline().unwrap() < Instant::now());
        assembler.expire_stale();
        assert_eq!(assembler.diagnostics().pending_frames, 0);
        assert!(assembler.next_deadline().is_none());
        for packet in old.iter().rev() {
            assert!(assembler.push(packet).unwrap().is_none());
        }
        for packet in &old {
            assert!(assembler.push(packet).unwrap().is_none());
        }
        assert_eq!(assembler.diagnostics().expired_frames, 1);
        assert_eq!(assembler.diagnostics().pending_frames, 0);
        // A genuinely newer frame across the RTP wrap can still complete.
        let new = packetize(&[4; 100], 10, &mut sequence, 1, 96).unwrap();
        assert_eq!(
            assembler.push(&new[0]).unwrap().unwrap().encoded,
            vec![4; 100]
        );
    }
}
