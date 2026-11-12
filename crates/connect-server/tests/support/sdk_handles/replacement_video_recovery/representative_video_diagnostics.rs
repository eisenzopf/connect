//! Fixed copied numeric events only, emitted by existing terminal Progress reports.
//! Unknown sequence/timestamp fields remain None; no media or permission retained.
use super::*;
const LIMIT: usize = 64;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Event {
    stage: &'static str,
    elapsed_us: Option<i128>,
    captured_us: Option<i128>,
    source_ssrc: Option<u32>,
    sequence_or_index: Option<u32>,
    rtp_timestamp: Option<u32>,
}
#[derive(Clone, Debug)]
pub(super) struct History {
    pub(super) began: Option<Instant>,
    total: u64,
    len: usize,
    events: [Option<Event>; LIMIT],
}
impl Default for History {
    fn default() -> Self {
        Self {
            began: None,
            total: 0,
            len: 0,
            events: [None; LIMIT],
        }
    }
}
fn relative(at: Instant, began: Instant) -> i128 {
    if at >= began {
        at.duration_since(began).as_micros() as i128
    } else {
        -(began.duration_since(at).as_micros() as i128)
    }
}
impl History {
    pub(super) fn record(
        &mut self,
        stage: &'static str,
        stamp: Option<(u32, u32, u32)>,
        captured: Option<Instant>,
    ) {
        self.total = self.total.saturating_add(1);
        if self.len == LIMIT {
            return;
        }
        let now = Instant::now();
        let source_ssrc = stamp.map(|s| s.0);
        let sequence_or_index = stamp
            .filter(|_| {
                matches!(
                    stage,
                    "completed-sequence-gap"
                        | "assembly-expired-or-discarded"
                        | "receiver-pli-locally-submitted"
                        | "requested-key-encoded"
                        | "periodic-key-request-still-pending"
                )
            })
            .map(|s| s.1);
        let rtp_timestamp = stamp
            .filter(|_| {
                matches!(
                    stage,
                    "completed-sequence-gap"
                        | "assembly-expired-or-discarded"
                        | "genuine-recovery-key-decoded"
                        | "requested-key-encoded"
                        | "periodic-key-request-still-pending"
                )
            })
            .map(|s| s.2);
        self.events[self.len] = Some(Event {
            stage,
            elapsed_us: self.began.map(|b| relative(now, b)),
            captured_us: captured.and_then(|at| self.began.map(|b| relative(at, b))),
            source_ssrc,
            sequence_or_index,
            rtp_timestamp,
        });
        self.len += 1;
    }
}
#[test]
fn video_recovery_history_keeps_actual_stage_source_clock_and_fixed_numeric_limit() {
    let began = Instant::now();
    let mut a = History {
        began: Some(began),
        ..Default::default()
    };
    let b = History::default();
    a.record("completed-sequence-gap", Some((3, 9, 27000)), Some(began));
    a.record("source-feedback-used", Some((3, 0, 0)), None);
    assert_eq!(a.events[0].unwrap().rtp_timestamp, Some(27000));
    assert_eq!(a.events[0].unwrap().sequence_or_index, Some(9));
    assert_eq!(a.events[0].unwrap().captured_us, Some(0));
    assert_eq!(a.events[1].unwrap().rtp_timestamp, None);
    assert_eq!(a.events[1].unwrap().sequence_or_index, None);
    assert_eq!(b.total, 0);
    for _ in 0..70 {
        a.record("sdk-queue-gap", None, None);
    }
    assert_eq!(a.total, 72);
    assert_eq!(a.len, LIMIT);
    assert_eq!(a.events[0].unwrap().source_ssrc, Some(3));
    assert_eq!(relative(began, began + Duration::from_micros(1)), -1);
}
