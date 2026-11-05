//! Sample clock for the explicitly host-generated 20 ms preparation source.
//! One wake generates at most its current slot. Missed slots remain gaps.
use connect_protocol::{Error, Result};
use std::time::Duration;
use tokio::time::Instant;

const QUANTUM: Duration = Duration::from_millis(20);

pub(super) struct PreparationAudioClock {
    start: Instant,
    next_due: Instant,
    samples_per_frame: u32,
}
impl PreparationAudioClock {
    pub(super) fn new(rate: u32, start: Instant) -> Result<Self> {
        if rate == 0 || rate % 50 != 0 {
            return Err(Error::new(
                "unsupported-media",
                "Preparation requires an exact 20 ms sample quantum",
            ));
        }
        Ok(Self {
            start,
            next_due: start + QUANTUM,
            samples_per_frame: rate / 50,
        })
    }
    pub(super) fn next_due(&self) -> Instant {
        self.next_due
    }
    pub(super) fn take_due(&mut self, now: Instant) -> Option<u32> {
        if now < self.next_due {
            return None;
        }
        let elapsed = now.duration_since(self.start);
        let slot = elapsed.as_nanos() / QUANTUM.as_nanos();
        let remainder = elapsed.as_nanos() % QUANTUM.as_nanos();
        self.next_due = now + Duration::from_nanos((QUANTUM.as_nanos() - remainder) as u64);
        // RTP clocks wrap naturally. Only this original host-generated source
        // is clocked here; original remote media never passes through it.
        Some((slot * u128::from(self.samples_per_frame)) as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn codec_quantum_stays_contiguous_through_subslot_scheduler_jitter() {
        let start = Instant::now();
        for (rate, samples) in [(8000, 160), (48000, 960)] {
            let mut clock = PreparationAudioClock::new(rate, start).unwrap();
            assert_eq!(clock.take_due(start + Duration::from_millis(19)), None);
            for (index, micros) in [21_200, 40_900, 62_100, 80_001].into_iter().enumerate() {
                assert_eq!(
                    clock.take_due(start + Duration::from_micros(micros)),
                    Some((index as u32 + 1) * samples)
                );
                assert_eq!(
                    clock.next_due(),
                    start + Duration::from_millis((index as u64 + 2) * 20)
                );
            }
        }
    }
    #[test]
    fn missed_quantum_is_one_real_clock_gap_and_cannot_catch_up_or_rebase() {
        let start = Instant::now();
        for (rate, samples) in [(8000, 160), (48000, 960)] {
            let mut clock = PreparationAudioClock::new(rate, start).unwrap();
            assert_eq!(
                clock.take_due(start + Duration::from_millis(20)),
                Some(samples)
            );
            let late = start + Duration::from_millis(67);
            assert_eq!(clock.take_due(late), Some(samples * 3));
            assert_eq!(clock.take_due(late), None);
            assert_eq!(clock.take_due(start + Duration::from_millis(79)), None);
            assert_eq!(
                clock.take_due(start + Duration::from_millis(80)),
                Some(samples * 4)
            );
            assert_eq!(
                clock.take_due(start + Duration::from_millis(100)),
                Some(samples * 5)
            );
        }
    }
    #[test]
    fn original_sample_clock_wraps_rtp_without_resetting_its_schedule() {
        let start = Instant::now();
        let mut clock = PreparationAudioClock::new(48000, start).unwrap();
        let slot = u64::from(u32::MAX / 960);
        let first_due = start + Duration::from_millis(slot * 20);
        let before = clock.take_due(first_due).unwrap();
        let next_due = first_due + QUANTUM;
        assert_eq!(clock.next_due(), next_due);
        let after = clock.take_due(next_due).unwrap();
        assert_eq!(after, before.wrapping_add(960));
        assert!(after < before);
        assert_eq!(clock.take_due(next_due), None);
    }
    #[test]
    fn unsupported_nonintegral_codec_clock_is_refused_before_source_creation() {
        let start = Instant::now();
        for rate in [0, 1, 7999, 48001] {
            assert!(PreparationAudioClock::new(rate, start).is_err());
        }
    }
}
