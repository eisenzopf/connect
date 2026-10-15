//! Application-owned, bounded audio/video release before codec/renderer input.
use anyhow::{bail, Result};
use connect_client::{NativeAudioFrame, NativeVideoFrame, VideoSequenceRange};
use std::time::{Duration, Instant};
use tokio::io::{AsyncWrite, AsyncWriteExt};

const DELAY: Duration = Duration::from_millis(120);
const STALE: Duration = Duration::from_millis(200);

struct Queued<T> {
    ticks: i64,
    received: Instant,
    due: Instant,
    frame: T,
}
struct Track<T> {
    clock: u32,
    maximum: usize,
    origin: Option<(u32, Instant)>,
    last: Option<i64>,
    queue: Vec<Queued<T>>,
    peak: usize,
    dropped: usize,
}
impl<T> Track<T> {
    fn new(clock: u32, maximum: usize) -> Self {
        Self {
            clock,
            maximum,
            origin: None,
            last: None,
            queue: Vec::new(),
            peak: 0,
            dropped: 0,
        }
    }
    fn push(&mut self, timestamp: u32, frame: T, received: Instant) {
        let (origin, began) = *self.origin.get_or_insert((timestamp, received));
        // One run lasts at most 300s, safely inside the RTP half-wrap interval.
        let ticks = i64::from(timestamp.wrapping_sub(origin) as i32);
        if self.last.is_some_and(|last| ticks <= last)
            || self.queue.iter().any(|item| item.ticks == ticks)
        {
            self.dropped += 1;
            return;
        }
        let offset = Duration::from_secs_f64(ticks.unsigned_abs() as f64 / f64::from(self.clock));
        let Some(source_time) = (if ticks >= 0 {
            began.checked_add(offset)
        } else {
            began.checked_sub(offset)
        }) else {
            self.dropped += 1;
            return;
        };
        if self.queue.len() == self.maximum {
            self.queue.remove(0);
            self.dropped += 1;
        }
        self.queue.push(Queued {
            ticks,
            received,
            // A sender clock jump must not extend this receiver's hold budget.
            // Keep timestamp ordering, but cap the local release delay from
            // actual receipt so downstream codec/file work retains headroom.
            due: (source_time + DELAY).min(received + DELAY),
            frame,
        });
        self.queue.sort_by_key(|item| item.ticks);
        self.peak = self.peak.max(self.queue.len());
    }
    fn expire(&mut self, now: Instant) {
        let before = self.queue.len();
        self.queue
            .retain(|item| now.saturating_duration_since(item.received) < STALE);
        self.dropped += before - self.queue.len();
    }
    fn next(&self) -> Option<Instant> {
        // Only the timestamp-ordered head can be released. A later frame can
        // have an earlier local due time after reordered arrival; waking on
        // that blocked frame would repeatedly select an already-past timer.
        self.queue
            .first()
            .map(|item| item.due)
            .into_iter()
            .chain(self.queue.iter().map(|item| item.received + STALE))
            .min()
    }
    fn due(&self, now: Instant) -> Option<Instant> {
        self.queue
            .first()
            .filter(|item| item.due <= now)
            .map(|item| item.due)
    }
    fn pop(&mut self) -> Queued<T> {
        let frame = self.queue.remove(0);
        self.last = Some(frame.ticks);
        frame
    }
}

pub enum Ready {
    Audio(NativeAudioFrame, Instant),
    Video(NativeVideoFrame, Instant, u64),
}
pub struct Playout {
    audio: Track<NativeAudioFrame>,
    video: Track<(NativeVideoFrame, VideoSequenceRange)>,
    continuity: crate::decode_continuity::DecodeContinuity,
    pub maximum_hold_ms: f64,
    emission_timeouts: usize,
    late_emissions: usize,
    presentation_origin: Option<i64>,
}
impl Playout {
    pub fn new() -> Self {
        Self {
            audio: Track::new(8000, 10),
            video: Track::new(90000, 8),
            continuity: crate::decode_continuity::DecodeContinuity::default(),
            maximum_hold_ms: 0.0,
            emission_timeouts: 0,
            late_emissions: 0,
            presentation_origin: None,
        }
    }
    pub fn audio(&mut self, frame: NativeAudioFrame, received: Instant) {
        self.audio.push(frame.timestamp_rtp, frame, received);
    }
    pub fn video(&mut self, frame: NativeVideoFrame, range: VideoSequenceRange, received: Instant) {
        self.video
            .push(frame.timestamp_rtp, (frame, range), received);
    }
    pub fn next_deadline(&self) -> Option<Instant> {
        self.audio.next().into_iter().chain(self.video.next()).min()
    }
    fn ready_one(&mut self, now: Instant) -> Option<Ready> {
        self.audio.expire(now);
        self.video.expire(now);
        loop {
            match (self.audio.due(now), self.video.due(now)) {
                (Some(a), Some(v)) if a <= v => {
                    let item = self.audio.pop();
                    self.maximum_hold_ms = self
                        .maximum_hold_ms
                        .max(now.duration_since(item.received).as_secs_f64() * 1000.0);
                    return Some(Ready::Audio(item.frame, item.received));
                }
                (_, Some(_)) => {
                    let item = self.video.pop();
                    let (frame, range) = item.frame;
                    // Losing even a complete VP8 reference frame invalidates
                    // dependent interframes. Check after timestamp reordering,
                    // so normal reordered arrival does not look like loss.
                    if !self.continuity.accept(frame.keyframe, range) {
                        self.video.dropped += 1;
                        continue;
                    }
                    self.maximum_hold_ms = self
                        .maximum_hold_ms
                        .max(now.duration_since(item.received).as_secs_f64() * 1000.0);
                    let base = *self.presentation_origin.get_or_insert(item.ticks);
                    return Some(Ready::Video(
                        frame,
                        item.received,
                        (item.ticks - base) as u64,
                    ));
                }
                (Some(_), None) => {
                    let item = self.audio.pop();
                    self.maximum_hold_ms = self
                        .maximum_hold_ms
                        .max(now.duration_since(item.received).as_secs_f64() * 1000.0);
                    return Some(Ready::Audio(item.frame, item.received));
                }
                (None, None) => return None,
            }
        }
    }
    /// Recheck freshness between sink operations, not once for an entire batch.
    /// Once a write starts, timeout is fatal: AsyncWrite may have accepted a
    /// partial record, so continuing the same IVF stream would be dishonest.
    pub async fn drain<V, A>(
        &mut self,
        video: &mut V,
        audio: &mut Option<A>,
        mut record: impl FnMut(&Ready, Instant) -> Result<()>,
        mut preview: impl FnMut(NativeVideoFrame),
    ) -> Result<()>
    where
        V: AsyncWrite + Unpin,
        A: AsyncWrite + Unpin,
    {
        while let Some(frame) = self.ready_one(tokio::time::Instant::now().into_std()) {
            let received = match &frame {
                Ready::Audio(_, received) | Ready::Video(_, received, _) => *received,
            };
            let deadline = tokio::time::Instant::from_std(received + STALE);
            // Do not start even the first write if time elapsed after dequeue.
            if tokio::time::Instant::now() >= deadline {
                match &frame {
                    Ready::Audio(..) => self.audio.dropped += 1,
                    Ready::Video(..) => {
                        self.video.dropped += 1;
                        self.continuity = crate::decode_continuity::DecodeContinuity::default();
                    }
                }
                continue;
            }
            let write = async {
                match &frame {
                    Ready::Video(frame, _, presentation) => {
                        connect_native_video::write_frame_at(video, frame, *presentation).await?;
                        video.flush().await?;
                    }
                    Ready::Audio(frame, _) => {
                        if let Some(output) = audio {
                            output.write_all(&frame.payload).await?;
                            output.flush().await?;
                        }
                    }
                }
                Ok::<_, anyhow::Error>(())
            };
            match tokio::time::timeout_at(deadline, write).await {
                Ok(result) => result?,
                Err(_) => {
                    self.emission_timeouts += 1;
                    bail!("Codec sink exceeded the frame's 200ms receipt deadline; partial output is not resumable");
                }
            }
            // Timestamp only after the actual write AND flush. A ready future
            // can finish after its deadline (e.g. scheduler delay), so timeout
            // alone is not enough. Retain that completed sample before failing.
            let emitted = tokio::time::Instant::now().into_std();
            record(&frame, emitted)?;
            if emitted.duration_since(received) > STALE {
                self.late_emissions += 1;
                bail!("Completed codec emission exceeded the frame's 200ms receipt deadline");
            }
            if let Ready::Video(frame, _, _) = frame {
                preview(frame);
            }
        }
        Ok(())
    }
    pub fn json(&self) -> String {
        format!("{{\"release_delay_ms\":120,\"stale_deadline_ms\":200,\"video_frames_bound\":8,\"audio_frames_bound\":10,\"peak_video_frames\":{},\"peak_audio_frames\":{},\"dropped_video_frames\":{},\"dropped_audio_frames\":{},\"maximum_hold_ms\":{},\"emission_timeouts\":{},\"late_emissions\":{}}}",
            self.video.peak, self.audio.peak, self.video.dropped, self.audio.dropped, self.maximum_hold_ms, self.emission_timeouts, self.late_emissions)
    }
    pub fn discard_live_buffers(&mut self) {
        self.audio.dropped += self.audio.queue.len();
        self.audio.queue.clear();
        self.video.dropped += self.video.queue.len();
        self.video.queue.clear();
    }
}

#[cfg(test)]
#[path = "playout_tests.rs"]
mod tests;
