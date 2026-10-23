//! Bounded PCMU buffering. An authority/metadata stall cannot replay old speech.
use std::{collections::VecDeque, time::Duration};
use tokio::time::Instant;
const BYTES: usize = 160;
const FRAMES: usize = 8;
// Match the native mixer's maximum retained input age. Byte capacity alone
// cannot bound latency when forwarding pauses and the provider goes silent.
const MAX_AGE: Duration = Duration::from_millis(200);
pub(crate) struct AudioQueue {
    frames: VecDeque<(Instant, [u8; BYTES])>,
    partial: [u8; BYTES],
    length: usize,
    partial_at: Option<Instant>,
}
impl Default for AudioQueue {
    fn default() -> Self {
        Self {
            frames: VecDeque::with_capacity(FRAMES),
            partial: [0xff; BYTES],
            length: 0,
            partial_at: None,
        }
    }
}
impl AudioQueue {
    fn expire(&mut self, now: Instant) {
        while self
            .frames
            .front()
            .is_some_and(|(at, _)| now.saturating_duration_since(*at) > MAX_AGE)
        {
            self.frames.pop_front();
        }
        if self
            .partial_at
            .is_some_and(|at| now.saturating_duration_since(at) > MAX_AGE)
        {
            self.length = 0;
            self.partial_at = None;
        }
    }
    pub(crate) fn push(&mut self, mut bytes: &[u8], now: Instant) {
        self.expire(now);
        while !bytes.is_empty() {
            if self.length == 0 {
                self.partial_at = Some(now);
                // Reserve space for the partial frame too: at most1280 bytes.
                if self.frames.len() == FRAMES {
                    self.frames.pop_front();
                }
            }
            let take = (BYTES - self.length).min(bytes.len());
            self.partial[self.length..self.length + take].copy_from_slice(&bytes[..take]);
            self.length += take;
            bytes = &bytes[take..];
            if self.length == BYTES {
                let at = self.partial_at.take().unwrap_or(now);
                self.frames.push_back((at, self.partial));
                self.length = 0;
            }
        }
    }
    pub(crate) fn frame(&mut self, now: Instant) -> [u8; BYTES] {
        self.expire(now);
        self.frames
            .pop_front()
            .map(|(_, frame)| frame)
            .unwrap_or([0xff; BYTES])
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn provider_message_boundaries_do_not_split_pcmu_frames() {
        let now = Instant::now();
        let mut q = AudioQueue::default();
        q.push(&[0x90; 80], now);
        assert_eq!(q.frame(now), [0xff; BYTES]);
        q.push(&[0x10; 80], now + Duration::from_millis(20));
        let frame = q.frame(now + Duration::from_millis(20));
        assert_eq!(&frame[..80], &[0x90; 80]);
        assert_eq!(&frame[80..], &[0x10; 80]);
    }
    #[test]
    fn queued_audio_keeps_only_the_newest_bounded_frames_and_partial() {
        let now = Instant::now();
        let mut q = AudioQueue::default();
        for n in 0..20 {
            q.push(&[n; BYTES], now);
        }
        q.push(&[99; 1], now);
        assert!(q.frames.len() * BYTES + q.length <= FRAMES * BYTES);
        for n in 13..20 {
            assert_eq!(q.frame(now), [n; BYTES]);
        }
        assert_eq!(q.frame(now), [0xff; BYTES]);
    }
    #[test]
    fn forwarding_stall_expires_complete_speech_and_replaces_it_with_silence() {
        let now = Instant::now();
        let mut q = AudioQueue::default();
        q.push(&[0x90; BYTES * FRAMES], now);
        assert_eq!(q.frame(now + Duration::from_millis(199)), [0x90; BYTES]);
        assert_eq!(q.frame(now + Duration::from_millis(201)), [0xff; BYTES]);
        q.push(&[0x10; BYTES], now + Duration::from_secs(1));
        assert_eq!(q.frame(now + Duration::from_secs(1)), [0x10; BYTES]);
    }
    #[test]
    fn expired_partial_speech_cannot_be_combined_with_a_fresh_provider_message() {
        let now = Instant::now();
        let mut q = AudioQueue::default();
        q.push(&[0x90; 80], now);
        q.push(&[0x10; 80], now + Duration::from_secs(1));
        assert_eq!(q.frame(now + Duration::from_secs(1)), [0xff; BYTES]);
        q.push(&[0x10; 80], now + Duration::from_secs(1));
        assert_eq!(q.frame(now + Duration::from_secs(1)), [0x10; BYTES]);
    }
}
