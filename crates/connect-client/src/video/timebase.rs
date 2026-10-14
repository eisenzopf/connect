//! One monotonic RTP epoch for independently muted native audio/video streams.
use connect_protocol::video_profile::VIDEO_CLOCK_HZ;
use std::time::{Duration, Instant};
pub(super) struct MediaTimebase {
    epoch: Instant,
    audio: Option<u32>,
    video: Option<u32>,
}
impl Default for MediaTimebase {
    fn default() -> Self {
        Self {
            epoch: Instant::now(),
            audio: None,
            video: None,
        }
    }
}
impl MediaTimebase {
    pub fn audio(&mut self) -> u32 {
        advance(self.epoch.elapsed(), 8000, 160, &mut self.audio)
    }
    pub fn video(&mut self) -> u32 {
        advance(
            self.epoch.elapsed(),
            VIDEO_CLOCK_HZ,
            VIDEO_CLOCK_HZ / 30,
            &mut self.video,
        )
    }
}
fn advance(elapsed: Duration, rate: u32, step: u32, previous: &mut Option<u32>) -> u32 {
    let measured = (elapsed.as_nanos() * u128::from(rate) / 1_000_000_000) as u32;
    let timestamp = match *previous {
        Some(last) => {
            let minimum = last.wrapping_add(step);
            if measured.wrapping_sub(minimum) < (1u32 << 31) {
                measured
            } else {
                minimum
            }
        }
        None => measured,
    };
    *previous = Some(timestamp);
    timestamp
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn muted_or_stalled_capture_resumes_in_the_same_audio_video_epoch() {
        let mut audio = None;
        let mut video = None;
        assert_eq!(advance(Duration::ZERO, 8000, 160, &mut audio), 0);
        assert_eq!(advance(Duration::ZERO, 90000, 3000, &mut video), 0);
        for i in 1..100 {
            advance(Duration::from_millis(i * 20), 8000, 160, &mut audio);
        }
        let elapsed = Duration::from_secs(2);
        assert_eq!(advance(elapsed, 8000, 160, &mut audio), 16000);
        assert_eq!(advance(elapsed, 90000, 3000, &mut video), 180000);
    }
    #[test]
    fn queued_capture_and_rtp_wrap_never_run_the_clock_backwards() {
        let mut previous = Some(u32::MAX - 159);
        assert_eq!(advance(Duration::ZERO, 8000, 160, &mut previous), 0);
        assert_eq!(advance(Duration::ZERO, 8000, 160, &mut previous), 160);
    }
}
