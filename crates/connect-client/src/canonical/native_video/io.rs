//! Explicit video-aware IO; ordinary native RTP parsing remains unchanged.
use super::*;
use crate::sdk::VideoReceiveEpoch;
use crate::{NativeVideoPacket, VideoFrameAssembler};
use rvoip_uctp::substrate::{
    observe_video_epoch_datagram, observe_video_feedback_datagram, pack_video_feedback_datagram,
    VideoEpochDatagram, VideoFeedbackDatagram,
};
use tokio::time::Instant;
const AGE: Duration = Duration::from_millis(200);
/// Original bytes and, only for admitted tagged video, authenticated history.
/// Neither the observation nor a keyframe hint grants server media permission.
pub struct NativeVideoDatagram {
    pub bytes: Vec<u8>,
    pub video_epoch: Option<VideoReceiveEpoch>,
}
struct Waiting {
    bytes: Vec<u8>,
    packet: VideoEpochDatagram,
    arrived: Instant,
}
struct Hint {
    value: VideoFeedbackDatagram,
    arrived: Instant,
    version: u64,
}
#[derive(Default)]
struct Recovery {
    history: Option<VideoReceiveEpoch>,
    assembler: VideoFrameAssembler,
    waiting: bool,
    last_frame: Option<u16>,
    sequence: u32,
    last_request: Option<Instant>,
}
/// The sole datagram reader of the already authenticated replacement peer.
/// Its signed reader validates allocations/epochs independently of application
/// control polling. Cancellation retains original packet/hint arrival ages.
/// There is no implicit retry or clock rewrite. Dropping this owner drops all
/// queued feedback/history; physical close and captured expiry still apply.
pub struct NativeVideoDatagrams {
    io: super::super::native_io::NativeDatagrams,
    shared: Arc<Shared>,
    pending: VecDeque<Waiting>,
    hints: BTreeMap<u16, Hint>,
    highwater: BTreeMap<u16, u32>,
    recovery: BTreeMap<u16, Recovery>,
}
impl CanonicalClient {
    /// Claim video-aware IO only after this peer observed and answered the exact
    /// original signed replacement allocation with `rtcp-pli-epoch/1` selected.
    /// This starts the existing bounded signature-verifying reader, never a new
    /// connection. Generic RTP-only readers continue to reject tagged packets.
    pub fn claim_native_video_datagrams(&mut self) -> Result<NativeVideoDatagrams> {
        {
            let guard = self
                .native_video
                .state
                .lock()
                .map_err(|_| ClientError::Protocol)?;
            let s = guard
                .as_ref()
                .filter(|s| !s.ended && !s.suspended)
                .ok_or(ClientError::Protocol)?;
            if s.selected
                .as_ref()
                .is_none_or(|v| v.video_feedback.is_empty())
            {
                return Err(ClientError::Protocol);
            }
        }
        self.start_signaling_reader()?;
        Ok(NativeVideoDatagrams {
            io: self.claim_native_datagrams()?,
            shared: self.native_video.clone(),
            pending: VecDeque::new(),
            hints: BTreeMap::new(),
            highwater: BTreeMap::new(),
            recovery: BTreeMap::new(),
        })
    }
}
impl NativeVideoDatagrams {
    /// Submit original source RTP/clock bytes. The server owns receiver tagging.
    pub fn send(&self, bytes: &[u8]) -> Result<()> {
        self.io.send(bytes)
    }
    /// Consume at most one original hint immediately before fresh encoding.
    /// Calling twice cannot repeat it; age, source SSRC, selected history,
    /// control version, physical lifetime and captured expiry must still match.
    pub fn take_keyframe_request(&mut self, handle: u16, ssrc: u32) -> Result<bool> {
        self.io.current()?;
        let Some(hint) = self.hints.remove(&handle) else {
            return Ok(false);
        };
        let guard = self
            .shared
            .state
            .lock()
            .map_err(|_| ClientError::Protocol)?;
        let Some(s) = guard.as_ref().filter(|s| !s.ended && !s.suspended) else {
            return Ok(false);
        };
        Ok(hint.arrived.elapsed() < AGE
            && hint.version == s.version
            && hint.value.media_ssrc == ssrc
            && s.active
                .get(&(handle, true))
                .is_some_and(|n| n.tag == hint.value.activation_epoch && n.history.ssrc == ssrc))
    }
    fn hint(&mut self, bytes: &[u8], arrived: Instant) -> Result<()> {
        let hint = observe_video_feedback_datagram(bytes).map_err(|_| ClientError::Protocol)?;
        let guard = self
            .shared
            .state
            .lock()
            .map_err(|_| ClientError::Protocol)?;
        let Some(s) = guard.as_ref().filter(|s| !s.ended && !s.suspended) else {
            return Ok(());
        };
        let Some(selected) = &s.selected else {
            return Ok(());
        };
        let selected = selected.streams.iter().any(|v| {
            v.stream_local_id == Some(hint.stream_local_id)
                && v.medium == crate::sdk::Medium::Video
                && v.direction != crate::sdk::Direction::Recvonly
                && selected.video_feedback.contains_key(&v.id)
        });
        if !selected {
            return Ok(());
        }
        let handle = hint.stream_local_id;
        if self.highwater.get(&handle).is_some_and(|old| {
            let d = hint.seq.wrapping_sub(*old);
            d == 0 || d >= (1 << 31)
        }) {
            return Ok(());
        }
        self.highwater.insert(handle, hint.seq);
        self.hints.insert(
            handle,
            Hint {
                value: hint,
                arrived,
                version: s.version,
            },
        );
        Ok(())
    }
    fn queued(&mut self) -> Result<Option<(Waiting, Notice)>> {
        let guard = self
            .shared
            .state
            .lock()
            .map_err(|_| ClientError::Protocol)?;
        let s = guard.as_ref().ok_or(ClientError::Protocol)?;
        if s.ended {
            return Err(ClientError::Transport);
        }
        if s.suspended {
            self.pending.clear();
            return Ok(None);
        }
        for _ in 0..self.pending.len() {
            let item = self.pending.pop_front().ok_or(ClientError::Protocol)?;
            if item.arrived.elapsed() >= AGE {
                continue;
            }
            let Some(n) = s.active.get(&(item.packet.stream_local_id, false)) else {
                // A known retired epoch is never retained for a later resume.
                if s.notices
                    .get(&(item.packet.stream_local_id, false))
                    .is_none_or(|old| old.tag != item.packet.epoch)
                {
                    self.pending.push_back(item);
                }
                continue;
            };
            let h = &item.packet.rtp.packet().header;
            let current = n.tag == item.packet.epoch
                && n.history.ssrc == h.ssrc
                && item.packet.seq.wrapping_sub(n.history.first_sequence) < (1 << 31)
                && (item.packet.seq != n.history.first_sequence
                    || h.timestamp == n.history.first_timestamp);
            if current {
                return Ok(Some((item, n.clone())));
            }
            if n.tag != item.packet.epoch
                && item.packet.seq.wrapping_sub(n.history.first_sequence) < (1 << 31)
            {
                self.pending.push_back(item);
            }
        }
        Ok(None)
    }
    fn receive_video(&mut self, item: Waiting, notice: Notice) -> Result<NativeVideoDatagram> {
        let handle = item.packet.stream_local_id;
        let recovery = self.recovery.entry(handle).or_default();
        if recovery.history.as_ref() != Some(&notice.history) {
            recovery.history = Some(notice.history.clone());
            recovery.assembler = VideoFrameAssembler::default();
            recovery.last_frame = None;
            recovery.waiting = true;
        }
        let packet = NativeVideoPacket::parse(item.bytes[24..].to_vec())?;
        let before = recovery.assembler.diagnostics();
        let frame = recovery.assembler.push(&packet)?;
        let after = recovery.assembler.diagnostics();
        if before.discarded_frames != after.discarded_frames
            || before.expired_frames != after.expired_frames
        {
            recovery.waiting = true;
        }
        if let Some(frame) = frame {
            let range = recovery
                .assembler
                .completed_sequence_range()
                .ok_or(ClientError::Protocol)?;
            if recovery
                .last_frame
                .is_some_and(|last| last.wrapping_add(1) != range.first)
            {
                recovery.waiting = true;
            }
            recovery.last_frame = Some(range.last);
            if frame.keyframe {
                recovery.waiting = false;
            }
        }
        if recovery.waiting
            && recovery
                .last_request
                .is_none_or(|at| at.elapsed() >= Duration::from_millis(100))
        {
            // No await between matching current signed history and atomic QUIC
            // submission. A failed/uncertain send consumes the sequence once.
            let guard = self
                .shared
                .state
                .lock()
                .map_err(|_| ClientError::Protocol)?;
            if guard.as_ref().is_some_and(|s| {
                !s.ended
                    && !s.suspended
                    && s.active
                        .get(&(handle, false))
                        .is_some_and(|n| n.history == notice.history)
            }) {
                let bytes = pack_video_feedback_datagram(&VideoFeedbackDatagram {
                    stream_local_id: handle,
                    seq: recovery.sequence,
                    sender_ssrc: 1,
                    media_ssrc: notice.history.ssrc,
                    activation_epoch: notice.tag,
                })
                .map_err(|_| ClientError::Protocol)?;
                recovery.sequence = recovery.sequence.wrapping_add(1);
                recovery.last_request = Some(Instant::now());
                self.io.submit(&bytes)?;
            }
        }
        Ok(NativeVideoDatagram {
            bytes: item.bytes,
            video_epoch: Some(notice.history),
        })
    }
    pub async fn receive(&mut self) -> Result<NativeVideoDatagram> {
        loop {
            self.io.current()?;
            let shared = self.shared.clone();
            let changed = shared.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if let Some((item, notice)) = self.queued()? {
                return self.receive_video(item, notice);
            }
            let expiry = self.pending.iter().map(|v| v.arrived + AGE).min();
            let bytes = tokio::select! {
                biased;
                _=&mut changed=>continue,
                _=tokio::time::sleep_until(expiry.unwrap_or_else(||Instant::now()+AGE)),if expiry.is_some()=>continue,
                bytes=self.io.read_raw()=>bytes?,
            };
            let arrived = Instant::now();
            if bytes.get(9) == Some(&206) && bytes.get(1) == Some(&0) {
                self.hint(&bytes, arrived)?;
                continue;
            }
            if bytes.get(1) == Some(&1) {
                let packet =
                    observe_video_epoch_datagram(&bytes).map_err(|_| ClientError::Protocol)?;
                let guard = self
                    .shared
                    .state
                    .lock()
                    .map_err(|_| ClientError::Protocol)?;
                let selected = guard
                    .as_ref()
                    .and_then(|s| s.selected.as_ref())
                    .is_some_and(|s| {
                        s.streams.iter().any(|v| {
                            v.stream_local_id == Some(packet.stream_local_id)
                                && v.medium == crate::sdk::Medium::Video
                                && v.direction != crate::sdk::Direction::Sendonly
                                && s.video_feedback.contains_key(&v.id)
                        })
                    });
                if selected
                    && self.pending.len() < 128
                    && self
                        .pending
                        .iter()
                        .filter(|v| v.packet.stream_local_id == packet.stream_local_id)
                        .count()
                        < 32
                {
                    self.pending.push_back(Waiting {
                        bytes,
                        packet,
                        arrived,
                    });
                }
                continue;
            }
            let packet =
                observe_native_media_datagram(&bytes).map_err(|_| ClientError::Protocol)?;
            if let ObservedNativeMediaDatagram::Rtp(packet) = packet {
                let guard = self
                    .shared
                    .state
                    .lock()
                    .map_err(|_| ClientError::Protocol)?;
                if guard
                    .as_ref()
                    .and_then(|s| s.selected.as_ref())
                    .is_some_and(|s| {
                        s.streams.iter().any(|v| {
                            v.stream_local_id == Some(packet.stream_local_id)
                                && s.video_feedback.contains_key(&v.id)
                        })
                    })
                {
                    continue;
                }
            }
            return Ok(NativeVideoDatagram {
                bytes,
                video_epoch: None,
            });
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
