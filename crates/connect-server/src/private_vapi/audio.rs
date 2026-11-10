//! Bounded PCMU buffering. Queue epochs carry no Read fence and cannot revive.
use std::{collections::VecDeque, time::Duration};
use tokio::time::Instant;
const BYTES: usize = 160;
const FRAMES: usize = 8;
const MAX_AGE: Duration = Duration::from_millis(200);
trait FrameEpoch: Clone {
    fn current(&self) -> bool;
    fn same(&self, other: &Self) -> bool;
}
// The existing assistant queue keeps its separate caller-owned authorization.
// This unit marker is not exported as provider snapshot authority.
impl FrameEpoch for () {
    fn current(&self) -> bool {
        true
    }
    fn same(&self, _: &Self) -> bool {
        true
    }
}
impl FrameEpoch for connect_store::PrivateProviderMediaEpoch {
    fn current(&self) -> bool {
        self.current()
    }
    fn same(&self, other: &Self) -> bool {
        self.same_generation(other)
    }
}
struct Buffer<E: FrameEpoch> {
    frames: VecDeque<(Instant, [u8; BYTES], E)>,
    partial: [u8; BYTES],
    length: usize,
    partial_at: Option<Instant>,
    partial_epoch: Option<E>,
}
impl<E: FrameEpoch> Default for Buffer<E> {
    fn default() -> Self {
        Self {
            frames: VecDeque::with_capacity(FRAMES),
            partial: [0xff; BYTES],
            length: 0,
            partial_at: None,
            partial_epoch: None,
        }
    }
}
impl<E: FrameEpoch> Buffer<E> {
    fn expire(&mut self, now: Instant) {
        while self.frames.front().is_some_and(|(at, _, epoch)| {
            !epoch.current() || now.saturating_duration_since(*at) > MAX_AGE
        }) {
            self.frames.pop_front();
        }
        if self
            .partial_at
            .is_some_and(|at| now.saturating_duration_since(at) > MAX_AGE)
            || self
                .partial_epoch
                .as_ref()
                .is_some_and(|epoch| !epoch.current())
        {
            self.length = 0;
            self.partial_at = None;
            self.partial_epoch = None;
        }
    }
    fn push(&mut self, mut bytes: &[u8], now: Instant, epoch: E) {
        self.expire(now);
        if !epoch.current() {
            return;
        }
        if self
            .partial_epoch
            .as_ref()
            .is_some_and(|old| !old.same(&epoch))
        {
            // Never mix an original partial frame with a successor generation.
            self.length = 0;
            self.partial_at = None;
            self.partial_epoch = None;
        }
        while !bytes.is_empty() {
            if self.length == 0 {
                self.partial_at = Some(now);
                self.partial_epoch = Some(epoch.clone());
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
                let original = self.partial_epoch.take().expect("partial epoch");
                self.frames.push_back((at, self.partial, original));
                self.length = 0;
            }
        }
    }
    fn frame(&mut self, now: Instant) -> Option<([u8; BYTES], E)> {
        self.expire(now);
        self.frames
            .pop_front()
            .map(|(_, frame, epoch)| (frame, epoch))
    }
}
/// Existing untagged queue API for the distinct assistant lane.
#[derive(Default)]
pub(crate) struct AudioQueue {
    inner: Buffer<()>,
}
impl AudioQueue {
    pub(crate) fn push(&mut self, bytes: &[u8], now: Instant) {
        self.inner.push(bytes, now, ());
    }
    pub(crate) fn frame(&mut self, now: Instant) -> [u8; BYTES] {
        self.inner
            .frame(now)
            .map(|(frame, _)| frame)
            .unwrap_or([0xff; BYTES])
    }
}
/// Private provider output requires its original immutable publication epoch.
#[derive(Default)]
pub(crate) struct AuthorizedAudioQueue {
    inner: Buffer<connect_store::PrivateProviderMediaEpoch>,
}
impl AuthorizedAudioQueue {
    pub(crate) fn push(
        &mut self,
        bytes: &[u8],
        now: Instant,
        epoch: connect_store::PrivateProviderMediaEpoch,
    ) {
        self.inner.push(bytes, now, epoch);
    }
    pub(crate) fn frame(
        &mut self,
        now: Instant,
    ) -> Option<([u8; BYTES], connect_store::PrivateProviderMediaEpoch)> {
        self.inner.frame(now)
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
        assert!(q.inner.frames.len() * BYTES + q.inner.length <= FRAMES * BYTES);
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
    #[derive(Clone)]
    struct TestEpoch {
        generation: u64,
        live: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }
    impl FrameEpoch for TestEpoch {
        fn current(&self) -> bool {
            self.live.load(std::sync::atomic::Ordering::Acquire)
        }
        fn same(&self, other: &Self) -> bool {
            self.generation == other.generation
        }
    }
    #[test]
    fn queued_complete_and_partial_audio_never_borrow_successor_epoch() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };
        let now = Instant::now();
        let old = TestEpoch {
            generation: 1,
            live: Arc::new(AtomicBool::new(true)),
        };
        let next = TestEpoch {
            generation: 2,
            live: Arc::new(AtomicBool::new(true)),
        };
        let mut q = Buffer::default();
        q.push(&[0x90; 240], now, old.clone());
        old.live.store(false, Ordering::Release);
        q.push(&[0x10; 80], now, next.clone());
        assert!(
            q.frame(now).is_none(),
            "old complete and partial frames must be discarded"
        );
        q.push(&[0x20; 80], now, next.clone());
        let (frame, epoch) = q.frame(now).unwrap();
        assert!(epoch.same(&next));
        assert_eq!(&frame[..80], &[0x10; 80]);
        assert_eq!(&frame[80..], &[0x20; 80]);
        old.live.store(true, Ordering::Release);
        assert!(
            q.frame(now).is_none(),
            "restoring policy cannot restore discarded old output"
        );
    }
    #[test]
    fn live_foreign_generation_partials_are_not_combined_and_age_bound_is_preserved() {
        use std::sync::{atomic::AtomicBool, Arc};
        let now = Instant::now();
        let first = TestEpoch {
            generation: 1,
            live: Arc::new(AtomicBool::new(true)),
        };
        let second = TestEpoch {
            generation: 2,
            live: Arc::new(AtomicBool::new(true)),
        };
        let mut q = Buffer::default();
        q.push(&[1; 80], now, first);
        q.push(&[2; 80], now, second.clone());
        assert!(q.frame(now).is_none());
        q.push(&[3; 80], now, second.clone());
        let (frame, _) = q.frame(now).unwrap();
        assert_eq!(&frame[..80], &[2; 80]);
        assert_eq!(&frame[80..], &[3; 80]);
        q.push(&[4; 160], now, second);
        assert!(q.frame(now + Duration::from_millis(201)).is_none());
    }
}
