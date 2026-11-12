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
    /// Original physical read, retained while waiting for a signed epoch.
    pub received_at: Instant,
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
    decoder_started: Option<Instant>,
    history: Option<VideoReceiveEpoch>,
    assembler: VideoFrameAssembler,
    waiting: bool,
    last_frame: Option<u16>,
    sequence: u32,
    last_request: Option<Instant>,
    // Only a fresh packet admitted after the latest decoder reset can start
    // recovery. Retained history alone cannot solicit a bootstrap request.
    requestable: bool,
}
#[cfg(feature = "test-support")]
#[path = "video_diagnostic.rs"]
mod diagnostic;
/// The sole datagram reader of the already authenticated replacement peer.
/// Its signed reader validates allocations/epochs independently of application
/// control polling. Cancellation retains original packet/hint arrival ages.
/// No media or feedback request is replayed and no clock is rewritten. While
/// receive is polled, a decoder waiting for its first keyframe may issue new
/// current-history requests at the existing 100ms cooldown, including silence.
/// Dropping this owner drops all queued feedback/history; physical close and
/// captured expiry still apply.
pub struct NativeVideoDatagrams {
    #[cfg(feature = "test-support")]
    diagnostic: Option<diagnostic::History>,
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
            #[cfg(feature = "test-support")]
            diagnostic:
                tracing::enabled!(target: "connect_client::video_recovery", tracing::Level::DEBUG)
                    .then(diagnostic::History::new),
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
    /// Wait for non-evicting local space for one original RTP/clock datagram.
    /// Use the original source capture/publication cutoff. Cancellation before
    /// enqueue submits nothing; this does not authorize or confirm server IO.
    pub async fn send_until(&self, bytes: &[u8], until: Instant) -> Result<()> {
        self.io.send_until(bytes, until).await
    }
    /// Start a fresh application decoder for one selected receiving video stream.
    /// Returns the exact monotonic boundary: older raw packets may still be
    /// returned for custody, but cannot bootstrap this decoder's recovery state.
    /// The next fresh interframe requests a keyframe through current signed
    /// history. Existing feedback sequence/rate bounds, RTP clocks and source
    /// authorization remain unchanged; this creates no media permission.
    pub fn reset_video_receive_decoder(&mut self, handle: u16) -> Result<Instant> {
        self.io.current()?;
        let guard = self
            .shared
            .state
            .lock()
            .map_err(|_| ClientError::Protocol)?;
        let selected = guard
            .as_ref()
            .filter(|s| !s.ended && !s.suspended)
            .and_then(|s| s.selected.as_ref())
            .is_some_and(|s| {
                s.streams.iter().any(|v| {
                    v.stream_local_id == Some(handle)
                        && v.medium == crate::sdk::Medium::Video
                        && v.direction != crate::sdk::Direction::Sendonly
                        && s.video_feedback.contains_key(&v.id)
                })
            });
        if !selected {
            return Err(ClientError::Protocol);
        }
        drop(guard);
        let at = Instant::now();
        let recovery = self.recovery.entry(handle).or_default();
        recovery.decoder_started = Some(at);
        recovery.assembler = VideoFrameAssembler::default();
        recovery.last_frame = None;
        recovery.waiting = true;
        recovery.requestable = false;
        Ok(at)
    }
    /// Check the exact receiver history against the current verified physical
    /// owner. A completed frame from an older epoch is not current readiness.
    pub fn is_current_video_receive_epoch(
        &self,
        handle: u16,
        epoch: &VideoReceiveEpoch,
    ) -> Result<bool> {
        self.io.current()?;
        let guard = self
            .shared
            .state
            .lock()
            .map_err(|_| ClientError::Protocol)?;
        Ok(guard.as_ref().is_some_and(|s| {
            !s.ended
                && !s.suspended
                && s.active
                    .get(&(handle, false))
                    .is_some_and(|n| &n.history == epoch)
        }))
    }
    /// Test-only numeric observation of this original verified physical owner.
    /// Counts signed receiver histories including inactive notices; no decoder,
    /// route, payload or authority capability leaves the SDK.
    #[cfg(feature = "test-support")]
    pub fn verified_video_receiver_notice_count_for_test(&self) -> Result<usize> {
        self.io.current()?;
        let guard = self
            .shared
            .state
            .lock()
            .map_err(|_| ClientError::Protocol)?;
        let state = guard.as_ref().ok_or(ClientError::Protocol)?;
        Ok(state
            .notices
            .keys()
            .filter(|(_, publisher)| !*publisher)
            .count())
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
        let accepted = hint.arrived.elapsed() < AGE
            && hint.version == s.version
            && hint.value.media_ssrc == ssrc
            && s.active
                .get(&(handle, true))
                .is_some_and(|n| n.tag == hint.value.activation_epoch && n.history.ssrc == ssrc);
        #[cfg(feature = "test-support")]
        if let Some(diagnostic) = &mut self.diagnostic {
            diagnostic.record(
                if accepted {
                    "hint-used"
                } else {
                    "hint-refused-current-age-version-history"
                },
                handle,
                ssrc,
                Some(hint.value.seq),
                None,
            );
        }
        Ok(accepted)
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
        #[cfg(feature = "test-support")]
        if let Some(diagnostic) = &mut self.diagnostic {
            diagnostic.record(
                "hint-accepted",
                handle,
                hint.media_ssrc,
                Some(hint.seq),
                None,
            );
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
                tracing::debug!(target: "connect_client::canonical::native_video", handle=item.packet.stream_local_id,
                    notice_present=s.active.contains_key(&(item.packet.stream_local_id,false)),
                    "Video packet expired waiting for matching signed history");
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
        if recovery.decoder_started.is_some_and(|at| item.arrived < at) {
            return Ok(NativeVideoDatagram {
                bytes: item.bytes,
                video_epoch: Some(notice.history),
                received_at: item.arrived,
            });
        }
        if recovery.history.as_ref() != Some(&notice.history) {
            #[cfg(feature = "test-support")]
            if let Some(diagnostic) = &mut self.diagnostic {
                diagnostic.record(
                    "receive-history-changed",
                    handle,
                    notice.history.ssrc,
                    Some(notice.history.first_sequence),
                    Some(notice.history.first_timestamp),
                );
            }
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
            #[cfg(feature = "test-support")]
            if let Some(diagnostic) = &mut self.diagnostic {
                diagnostic.record(
                    "assembly-expired-or-discarded",
                    handle,
                    packet.ssrc,
                    Some(u32::from(packet.sequence)),
                    Some(packet.timestamp_rtp),
                );
            }
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
                #[cfg(feature = "test-support")]
                if let Some(diagnostic) = &mut self.diagnostic {
                    diagnostic.record(
                        "completed-sequence-gap",
                        handle,
                        packet.ssrc,
                        Some(u32::from(range.first)),
                        Some(packet.timestamp_rtp),
                    );
                }
            }
            recovery.last_frame = Some(range.last);
            if frame.keyframe {
                #[cfg(feature = "test-support")]
                if recovery.waiting {
                    if let Some(diagnostic) = &mut self.diagnostic {
                        diagnostic.record(
                            "recovery-key-assembled",
                            handle,
                            packet.ssrc,
                            Some(u32::from(range.first)),
                            Some(frame.timestamp_rtp),
                        );
                    }
                }
                recovery.assembler.retire_older_partials_after_keyframe();
                recovery.waiting = false;
            }
        }
        recovery.requestable = true;
        self.retry_waiting_receivers()?;
        Ok(NativeVideoDatagram {
            bytes: item.bytes,
            video_epoch: Some(notice.history),
            received_at: item.arrived,
        })
    }
    // Every retry is a NEW feedback sequence under current signed history,
    // original physical ownership and the existing 100ms cooldown. No frame,
    // current permission or successful server admission is retained here.
    fn retry_waiting_receivers(&mut self) -> Result<Option<Instant>> {
        self.io.current()?;
        let guard = self
            .shared
            .state
            .lock()
            .map_err(|_| ClientError::Protocol)?;
        let Some(state) = guard
            .as_ref()
            .filter(|state| !state.ended && !state.suspended)
        else {
            return Ok(None);
        };
        let mut next = None;
        for (&handle, recovery) in &mut self.recovery {
            if !recovery.waiting || !recovery.requestable {
                continue;
            }
            let Some(notice) = state.active.get(&(handle, false)).filter(|notice| {
                recovery.history.as_ref() == Some(&notice.history)
                    && state.selected.as_ref().is_some_and(|selected| {
                        selected.streams.iter().any(|stream| {
                            stream.stream_local_id == Some(handle)
                                && stream.medium == crate::sdk::Medium::Video
                                && stream.direction != crate::sdk::Direction::Sendonly
                                && selected.video_feedback.contains_key(&stream.id)
                        })
                    })
            }) else {
                continue;
            };
            let now = Instant::now();
            let due = recovery
                .last_request
                .map(|at| at + Duration::from_millis(100));
            if due.is_none_or(|due| now >= due) {
                let bytes = pack_video_feedback_datagram(&VideoFeedbackDatagram {
                    stream_local_id: handle,
                    seq: recovery.sequence,
                    sender_ssrc: 1,
                    media_ssrc: notice.history.ssrc,
                    activation_epoch: notice.tag,
                })
                .map_err(|_| ClientError::Protocol)?;
                // A failed/uncertain local send consumes this request exactly
                // once. Later recovery uses a new sequence and fresh server
                // authority, never a retransmission of this request.
                recovery.sequence = recovery.sequence.wrapping_add(1);
                recovery.last_request = Some(now);
                let result = self.io.submit(&bytes);
                #[cfg(feature = "test-support")]
                if let Some(diagnostic) = &mut self.diagnostic {
                    diagnostic.record(
                        if result.is_ok() {
                            "auto-pli-locally-submitted"
                        } else {
                            "auto-pli-local-refused"
                        },
                        handle,
                        notice.history.ssrc,
                        Some(recovery.sequence.wrapping_sub(1)),
                        None,
                    );
                }
                result?;
            }
            let due = recovery.last_request.unwrap() + Duration::from_millis(100);
            next = Some(next.map_or(due, |old: Instant| old.min(due)));
        }
        Ok(next)
    }
    pub async fn receive(&mut self) -> Result<NativeVideoDatagram> {
        let deadline = self.io.receive_deadline()?;
        loop {
            self.io.current()?;
            if Instant::now() >= deadline {
                return Err(ClientError::Timeout);
            }
            let shared = self.shared.clone();
            let changed = shared.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if let Some((item, notice)) = self.queued()? {
                return self.receive_video(item, notice);
            }
            let expiry = self.pending.iter().map(|v| v.arrived + AGE).min();
            let feedback = self.retry_waiting_receivers()?;
            let bytes = tokio::select! {
                biased;
                _=tokio::time::sleep_until(deadline)=>return Err(ClientError::Timeout),
                _=&mut changed=>continue,
                _=tokio::time::sleep_until(feedback.unwrap_or(deadline)),if feedback.is_some()=>continue,
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
                tracing::debug!(target: "connect_client::canonical::native_video", handle=packet.stream_local_id,
                    selected, sequence=packet.seq, "Received tagged video packet");
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
                received_at: arrived,
            });
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;

#[cfg(feature = "test-support")]
impl Drop for NativeVideoDatagrams {
    fn drop(&mut self) {
        if let Some(diagnostic) = &self.diagnostic {
            tracing::debug!(target: "connect_client::video_recovery", evidence=?diagnostic, "Original raw video recovery terminal evidence");
        }
    }
}
