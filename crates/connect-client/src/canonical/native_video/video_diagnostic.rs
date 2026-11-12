//! Test-support only, copied numeric metadata, first64 events, no owners or payloads.
use std::time::{Instant, SystemTime, UNIX_EPOCH};
const LIMIT: usize = 64;
#[derive(Debug)]
struct Event {
    elapsed_us: u128,
    stage: &'static str,
    handle: u16,
    source_ssrc: u32,
    sequence: Option<u32>,
    rtp_timestamp: Option<u32>,
}
#[derive(Debug)]
pub(super) struct History {
    began: Instant,
    began_unix_us: u128,
    clock_pair_window_us: u128,
    total: u64,
    len: usize,
    events: [Option<Event>; LIMIT],
}
impl History {
    pub(super) fn new() -> Self {
        let began = Instant::now();
        let began_unix_us = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_micros());
        let clock_pair_window_us = began.elapsed().as_micros();
        Self {
            began,
            began_unix_us,
            clock_pair_window_us,
            total: 0,
            len: 0,
            events: std::array::from_fn(|_| None),
        }
    }
    pub(super) fn record(
        &mut self,
        stage: &'static str,
        handle: u16,
        source_ssrc: u32,
        sequence: Option<u32>,
        rtp_timestamp: Option<u32>,
    ) {
        self.total = self.total.saturating_add(1);
        if self.len == LIMIT {
            return;
        }
        self.events[self.len] = Some(Event {
            elapsed_us: self.began.elapsed().as_micros(),
            stage,
            handle,
            source_ssrc,
            sequence,
            rtp_timestamp,
        });
        self.len += 1;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raw_video_recovery_diagnostics_remain_numeric_bounded_and_owner_local() {
        let mut a = History::new();
        let b = History::new();
        a.record("auto-pli-locally-submitted", 4, 7, Some(0), None);
        a.record("recovery-key-assembled", 4, 7, Some(3), Some(9000));
        assert_eq!(a.events[0].as_ref().unwrap().rtp_timestamp, None);
        assert_eq!(a.events[1].as_ref().unwrap().rtp_timestamp, Some(9000));
        assert_eq!(a.events[1].as_ref().unwrap().handle, 4);
        assert_eq!(b.total, 0);
        for _ in 0..70 {
            a.record("hint-used", 5, 8, Some(2), None);
        }
        assert_eq!(a.len, LIMIT);
        assert_eq!(a.total, 72);
        assert_eq!(a.events[0].as_ref().unwrap().sequence, Some(0));
    }
}
