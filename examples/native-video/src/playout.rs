//! Application-owned, bounded audio/video release before codec/renderer input.
use connect_client::{NativeAudioFrame, NativeVideoFrame};
use std::time::{Duration, Instant};

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
            due: source_time + DELAY,
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
        self.queue
            .iter()
            .map(|item| item.due.min(item.received + STALE))
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
    video: Track<NativeVideoFrame>,
    pub maximum_hold_ms: f64,
    presentation_origin: Option<i64>,
}
impl Playout {
    pub fn new() -> Self {
        Self {
            audio: Track::new(8000, 10),
            video: Track::new(90000, 8),
            maximum_hold_ms: 0.0,
            presentation_origin: None,
        }
    }
    pub fn audio(&mut self, frame: NativeAudioFrame, received: Instant) {
        self.audio.push(frame.timestamp_rtp, frame, received);
    }
    pub fn video(&mut self, frame: NativeVideoFrame, received: Instant) {
        self.video.push(frame.timestamp_rtp, frame, received);
    }
    pub fn next_deadline(&self) -> Option<Instant> {
        self.audio.next().into_iter().chain(self.video.next()).min()
    }
    pub fn ready(&mut self, now: Instant) -> Vec<Ready> {
        self.audio.expire(now);
        self.video.expire(now);
        let mut result = Vec::new();
        loop {
            match (self.audio.due(now), self.video.due(now)) {
                (Some(a), Some(v)) if a <= v => {
                    let item = self.audio.pop();
                    self.maximum_hold_ms = self
                        .maximum_hold_ms
                        .max(now.duration_since(item.received).as_secs_f64() * 1000.0);
                    result.push(Ready::Audio(item.frame, item.received));
                }
                (_, Some(_)) => {
                    let item = self.video.pop();
                    self.maximum_hold_ms = self
                        .maximum_hold_ms
                        .max(now.duration_since(item.received).as_secs_f64() * 1000.0);
                    let base = *self.presentation_origin.get_or_insert(item.ticks);
                    result.push(Ready::Video(
                        item.frame,
                        item.received,
                        (item.ticks - base) as u64,
                    ));
                }
                (Some(_), None) => {
                    let item = self.audio.pop();
                    self.maximum_hold_ms = self
                        .maximum_hold_ms
                        .max(now.duration_since(item.received).as_secs_f64() * 1000.0);
                    result.push(Ready::Audio(item.frame, item.received));
                }
                (None, None) => break,
            }
        }
        result
    }
    pub fn json(&self) -> String {
        format!("{{\"release_delay_ms\":120,\"stale_deadline_ms\":200,\"video_frames_bound\":8,\"audio_frames_bound\":10,\"peak_video_frames\":{},\"peak_audio_frames\":{},\"dropped_video_frames\":{},\"dropped_audio_frames\":{},\"maximum_hold_ms\":{}}}",
            self.video.peak, self.audio.peak, self.video.dropped, self.audio.dropped, self.maximum_hold_ms)
    }
    pub fn discard_live_buffers(&mut self) {
        self.audio.dropped += self.audio.queue.len();
        self.audio.queue.clear();
        self.video.dropped += self.video.queue.len();
        self.video.queue.clear();
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    #[test]
    fn reorders_before_release_across_wrap_and_rejects_late_replay() {
        let start = Instant::now();
        let mut track = Track::new(90000, 8);
        track.push(u32::MAX - 1000, 1, start);
        track.push(5000, 3, start + Duration::from_millis(20));
        track.push(2000, 2, start + Duration::from_millis(40));
        assert!(track.due(start + Duration::from_millis(119)).is_none());
        assert_eq!(track.pop().frame, 1);
        assert_eq!(track.pop().frame, 2);
        assert_eq!(track.pop().frame, 3);
        track.push(2000, 2, start + Duration::from_millis(180));
        assert!(track.queue.is_empty());
    }
    #[test]
    fn queues_are_bounded_and_stale_frames_cannot_be_released() {
        let start = Instant::now();
        let mut track = Track::new(8000, 10);
        for frame in 0..100 {
            track.push(frame * 160, frame, start);
        }
        assert_eq!(track.peak, 10);
        assert_eq!(track.queue.len(), 10);
        track.expire(start + STALE);
        assert!(track.queue.is_empty());
        assert_eq!(track.dropped, 100);
    }
    #[test]
    fn audio_and_video_use_same_release_delay_with_distinct_rtp_origins() {
        let start = Instant::now();
        let mut audio = Track::new(8000, 10);
        let mut video = Track::new(90000, 8);
        audio.push(100000, 1, start);
        video.push(999999999, 1, start);
        assert_eq!(audio.next(), video.next());
        assert_eq!(audio.next(), Some(start + DELAY));
    }
}
