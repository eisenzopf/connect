//! One pending feedback hint per existing stream. No tasks or media-direction
//! bypass are introduced by this separately negotiated control path.
use super::*;
use rvoip_uctp::substrate::{
    observe_video_feedback_datagram, pack_video_feedback_datagram, VideoFeedbackDatagram,
};

pub(super) struct FeedbackState {
    binding_id: u128,
    sender_ssrc: u32,
    send_sequence: u32,
    last_observed: Option<u32>,
    pending: Option<(VideoFeedbackDatagram, Instant)>,
    publisher_active: bool,
}
impl Default for FeedbackState {
    fn default() -> Self {
        let id = uuid::Uuid::new_v4().as_u128();
        Self {
            binding_id: id,
            sender_ssrc: (id as u32).max(1),
            send_sequence: 0,
            last_observed: None,
            pending: None,
            publisher_active: false,
        }
    }
}
impl FeedbackState {
    pub(super) fn disable_publisher(&mut self) {
        self.publisher_active = false;
        self.pending = None;
    }
    pub(super) fn announce_publisher(&mut self) {
        self.publisher_active = true;
    }
}

#[derive(Clone)]
pub(crate) struct KeyframeTarget {
    key: StreamKey,
    allocation: NativeVideoFeedback,
    history: VideoReceiveEpoch,
    binding_id: u128,
}
fn publishes(queue: &Queue) -> bool {
    queue.binding.stream.medium == Medium::Video
        && queue.video.is_some()
        && matches!(
            queue.binding.stream.direction,
            Direction::Sendonly | Direction::Sendrecv
        )
}
fn newer(value: u32, previous: u32) -> bool {
    let delta = value.wrapping_sub(previous);
    delta > 0 && delta < (1 << 31)
}
fn pending_matches(queue: &Queue, request: &VideoFeedbackDatagram) -> bool {
    publishes(queue)
        && queue.feedback.publisher_active
        && queue.publisher_epoch.as_ref().is_some_and(|n| {
            n.tag == request.activation_epoch
                && n.history.ssrc == request.media_ssrc
                && n.handle == request.stream_local_id
        })
}

impl Inbox {
    // Invoked before the ordinary media parser, after counting this datagram.
    // The caller holds the inbox mutex; notification happens after it is released.
    pub(super) fn push_feedback(s: &mut State, bytes: &[u8]) {
        let Ok(request) = observe_video_feedback_datagram(bytes) else {
            s.stats.malformed_datagrams = s.stats.malformed_datagrams.saturating_add(1);
            return;
        };
        let Some(key) = s.handles.get(&request.stream_local_id) else {
            s.stats.unknown_handles = s.stats.unknown_handles.saturating_add(1);
            return;
        };
        let Some(queue) = s.queues.get_mut(key) else {
            return;
        };
        if !publishes(queue) {
            s.stats.control_drops = s.stats.control_drops.saturating_add(1);
            return;
        }
        // Keep the serial high-water mark even when a hint expires or is
        // consumed. A duplicate cannot acquire a new arrival time later.
        if queue
            .feedback
            .last_observed
            .is_some_and(|old| !newer(request.seq, old))
        {
            s.stats.feedback_drops = s.stats.feedback_drops.saturating_add(1);
            return;
        }
        queue.feedback.last_observed = Some(request.seq);
        if queue.feedback.pending.is_some() {
            s.stats.feedback_drops = s.stats.feedback_drops.saturating_add(1);
        }
        queue.feedback.pending = Some((request, Instant::now()));
        s.stats.feedback_datagrams = s.stats.feedback_datagrams.saturating_add(1);
    }

    pub(crate) fn keyframe_target(&self, key: &StreamKey) -> Result<KeyframeTarget> {
        let s = self
            .state
            .lock()
            .map_err(|_| Error::media(ErrorKind::Protocol, false))?;
        if s.ended {
            return Err(Error::media(ErrorKind::Cancelled, false));
        }
        let queue = s
            .queues
            .get(key)
            .ok_or_else(|| Error::media(ErrorKind::Unsupported, false))?;
        let n = queue
            .receiver_epoch
            .as_ref()
            .filter(|n| queue.receiver_epoch_active && n.matches_stream(&queue.binding.stream))
            .ok_or_else(|| Error::media(ErrorKind::Unsupported, false))?;
        Ok(KeyframeTarget {
            key: key.clone(),
            allocation: n.allocation.clone(),
            history: n.history.clone(),
            binding_id: queue.feedback.binding_id,
        })
    }

    /// The queued request carries its original target. It must never silently
    /// turn into a request for a different route after local queueing.
    pub(crate) fn send_keyframe(
        &self,
        binding: &Binding,
        target: &KeyframeTarget,
        send: impl FnOnce(&[u8]) -> Result<()>,
    ) -> Result<LocalMediaSubmission> {
        let mut s = self
            .state
            .lock()
            .map_err(|_| Error::media(ErrorKind::Protocol, false))?;
        if s.ended {
            return Err(Error::media(ErrorKind::Cancelled, false));
        }
        let queue = s
            .queues
            .get_mut(&target.key)
            .ok_or_else(|| Error::media(ErrorKind::Unsupported, false))?;
        let current = queue
            .receiver_epoch
            .as_ref()
            .filter(|n| {
                queue.receiver_epoch_active
                    && n.matches_stream(&queue.binding.stream)
                    && n.matches_stream(&binding.stream)
                    && n.allocation == target.allocation
                    && n.history == target.history
                    && queue.feedback.binding_id == target.binding_id
                    && queue.video.as_ref() == Some(&target.allocation)
            })
            .ok_or_else(|| Error::media(ErrorKind::Conflict, false))?;
        let sequence = queue.feedback.send_sequence;
        let bytes = pack_video_feedback_datagram(&VideoFeedbackDatagram {
            stream_local_id: current.handle,
            seq: sequence,
            sender_ssrc: queue.feedback.sender_ssrc,
            media_ssrc: current.history.ssrc,
            activation_epoch: current.tag,
        })
        .map_err(|_| Error::media(ErrorKind::Protocol, false))?;
        // A possibly uncertain send consumes its serial once; no implicit retry.
        queue.feedback.send_sequence = sequence.wrapping_add(1);
        send(&bytes)?;
        Ok(LocalMediaSubmission {
            datagram_sequence: sequence,
        })
    }

    pub(crate) fn feedback_is_current(
        &self,
        key: &StreamKey,
        request: &VideoKeyframeRequest,
    ) -> Result<bool> {
        let s = self
            .state
            .lock()
            .map_err(|_| Error::local(ErrorKind::Protocol))?;
        Ok(!s.ended
            && &request.key == key
            && Instant::now() < request.expires
            && s.queues.get(key).is_some_and(|q| {
                publishes(q)
                    && q.feedback.publisher_active
                    && q.feedback.binding_id == request.binding_id
                    && q.video.as_ref() == Some(&request.allocation)
                    && q.publisher_epoch
                        .as_ref()
                        .is_some_and(|n| n.history == request.history)
            }))
    }

    pub(crate) async fn receive_feedback(
        &self,
        key: &StreamKey,
        options: RequestOptions,
    ) -> Result<VideoKeyframeRequest> {
        options.validate()?;
        let until = Instant::now() + options.timeout;
        loop {
            if options.signal.is_cancelled() {
                return Err(Error::media(ErrorKind::Cancelled, false));
            }
            if Instant::now() >= until {
                return Err(Error::media(ErrorKind::Deadline, false));
            }
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let mut expiry = None;
            {
                let mut s = self
                    .state
                    .lock()
                    .map_err(|_| Error::local(ErrorKind::Protocol))?;
                if s.ended {
                    return Err(Error::media(ErrorKind::Cancelled, false));
                }
                let q = s
                    .queues
                    .get_mut(key)
                    .filter(|q| publishes(q))
                    .ok_or_else(|| Error::media(ErrorKind::Unsupported, false))?;
                if let Some((pending, arrived)) = &q.feedback.pending {
                    let expires = *arrived + RECEIVE_QUEUE_AGE;
                    if Instant::now() >= expires {
                        q.feedback.pending = None;
                        s.stats.feedback_drops = s.stats.feedback_drops.saturating_add(1);
                    } else if pending_matches(q, pending) {
                        let n = q.publisher_epoch.as_ref().expect("matched epoch");
                        let request = VideoKeyframeRequest {
                            datagram_sequence: pending.seq,
                            ssrc: pending.media_ssrc,
                            epoch: n.history.epoch.clone(),
                            epoch_revision: n.history.revision,
                            key: key.clone(),
                            allocation: n.allocation.clone(),
                            history: n.history.clone(),
                            binding_id: q.feedback.binding_id,
                            expires,
                        };
                        q.feedback.pending = None;
                        return Ok(request);
                    } else {
                        expiry = Some(expires);
                    }
                }
            }
            tokio::select! {biased;
                _=options.signal.cancelled()=>return Err(Error::media(ErrorKind::Cancelled,false)),
                _=tokio::time::sleep_until(until)=>return Err(Error::media(ErrorKind::Deadline,false)),
                _=tokio::time::sleep_until(expiry.unwrap_or(until)),if expiry.is_some()=>{},
                _=changed=>{},
            }
        }
    }
}

#[cfg(test)]
mod tests;
