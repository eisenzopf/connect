//! Every physical destination joins the same immutable source program.
use super::*;
mod clock;
pub(crate) use clock::ProgrammeClock;

// Source sends and original status polls must not wait for an input queue to empty.
// The public native datagram read is atomic and cancellation-safe at this boundary.
pub(super) async fn receive_before_tick<T>(
    next: tokio::time::Instant,
    receive: impl std::future::Future<Output = T>,
) -> Option<T> {
    if tokio::time::Instant::now() >= next {
        return None;
    }
    tokio::select! {
        biased;
        _ = tokio::time::sleep_until(next) => None,
        value = receive => Some(value),
    }
}

pub(super) struct Program {
    pub unix: u128,
    pub elapsed: Duration,
    pub audio_index: usize,
    pub video_index: usize,
}

impl Program {
    #[cfg(test)]
    pub fn at(original: Option<u64>, now_us: u128, seconds: u16) -> Result<Self> {
        let unix = original.map(u128::from).unwrap_or(now_us);
        let elapsed = now_us
            .checked_sub(unix)
            .ok_or("original media program has not started")?;
        Self::from_elapsed(
            unix,
            Duration::from_micros(u64::try_from(elapsed)?),
            seconds,
        )
    }
    pub fn from_elapsed(unix: u128, elapsed: Duration, seconds: u16) -> Result<Self> {
        if unix == 0 || seconds == 0 || elapsed >= Duration::from_secs(u64::from(seconds)) {
            return Err("original media program is expired or invalid".into());
        }
        Ok(Self {
            unix,
            elapsed,
            audio_index: usize::try_from(elapsed.as_micros() / 20_000)?,
            video_index: usize::try_from(elapsed.as_micros() * 30 / 1_000_000)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn due_tick_wins_over_continuously_ready_input() {
        let due = tokio::time::Instant::now() - Duration::from_secs(1);
        assert_eq!(receive_before_tick(due, std::future::ready(42)).await, None);
    }
    #[tokio::test]
    async fn input_before_future_tick_and_empty_input_at_due_tick_keep_original_bound() {
        let future = tokio::time::Instant::now() + Duration::from_secs(3600);
        assert_eq!(
            receive_before_tick(future, std::future::ready(42)).await,
            Some(42)
        );
        let due = tokio::time::Instant::now() - Duration::from_secs(1);
        assert_eq!(
            receive_before_tick(due, std::future::pending::<u32>()).await,
            None
        );
    }
    #[test]
    fn successive_destinations_keep_original_frame_indices_and_shared_av_clock() {
        let original = 1_793_600_000_000_000;
        for (elapsed, audio, video) in [
            (0, 0, 0),
            (19_999, 0, 0),
            (20_000, 1, 0),
            (33_334, 1, 1),
            (2_340_000, 117, 70),
            (61_999_999, 3099, 1859),
        ] {
            let program = Program::at(Some(original), u128::from(original) + elapsed, 62).unwrap();
            assert_eq!(program.unix, u128::from(original));
            assert_eq!(program.elapsed.as_micros(), elapsed);
            assert_eq!(program.audio_index, audio);
            assert_eq!(program.video_index, video);
            // Both source timestamps remain coordinates of the original
            // program, including a newly admitted destination near expiry.
            assert_eq!(
                ntp(program.unix + program.audio_index as u128 * 20_000),
                ntp(u128::from(original) + audio as u128 * 20_000)
            );
        }
    }

    #[test]
    fn future_and_expired_original_programs_never_rebase_to_current_time() {
        for (original, now, seconds) in [
            (Some(100), 99, 1),
            (Some(100), 1_000_100, 1),
            (Some(0), 100, 1),
            (Some(100), u128::MAX, 62),
            (Some(100), 100, 0),
        ] {
            assert!(Program::at(original, now, seconds).is_err());
        }
        let local = Program::at(None, 100, 1).unwrap();
        assert_eq!(local.unix, 100);
        assert_eq!(local.audio_index, 0);
        assert_eq!(local.video_index, 0);
    }
}
