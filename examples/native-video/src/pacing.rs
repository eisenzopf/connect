//! Pace codec pipe output before the bounded capture queue.
//!
//! A codec can flush several already-encoded frames together during startup or
//! after a stall. Sending that burst assigns future RTP timestamps faster than
//! wall time and can make every frame expire in a bounded receiver. Preserve
//! the declared capture cadence and skip missed ticks instead of catching up.
use std::time::Duration;
use tokio::time::{Interval, MissedTickBehavior};

pub fn capture_cadence(period: Duration) -> Interval {
    let mut cadence = tokio::time::interval(period);
    cadence.set_missed_tick_behavior(MissedTickBehavior::Skip);
    cadence
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn buffered_startup_frames_keep_the_negotiated_capture_rate() {
        for period in [
            Duration::from_millis(20),
            Duration::from_nanos(1_000_000_000 / 30),
        ] {
            let mut cadence = capture_cadence(period);
            let first = cadence.tick().await;
            // All encoded input is already available; cadence still spans the
            // source clock instead of assigning 400ms of RTP in one burst.
            for index in 1..=20 {
                let actual = cadence.tick().await;
                assert_eq!(actual.duration_since(first), period * index);
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn stalled_capture_does_not_replay_every_missed_tick() {
        let period = Duration::from_millis(20);
        let mut cadence = capture_cadence(period);
        cadence.tick().await;
        tokio::time::sleep(Duration::from_millis(410)).await;
        cadence.tick().await;
        let resumed = tokio::time::Instant::now();
        cadence.tick().await;
        assert!(tokio::time::Instant::now() > resumed);
    }
}
