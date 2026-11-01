//! Every physical destination joins the same immutable source program.
use super::*;

pub(super) struct Program {
    pub unix: u128,
    pub elapsed: Duration,
    pub audio_index: usize,
    pub video_index: usize,
}

impl Program {
    pub fn at(original: Option<u64>, now_us: u128, seconds: u16) -> Result<Self> {
        let unix = original.map(u128::from).unwrap_or(now_us);
        let elapsed = now_us
            .checked_sub(unix)
            .ok_or("original media program has not started")?;
        if unix == 0 || seconds == 0 || elapsed >= u128::from(seconds) * 1_000_000 {
            return Err("original media program is expired or invalid".into());
        }
        Ok(Self {
            unix,
            elapsed: Duration::from_micros(u64::try_from(elapsed)?),
            audio_index: usize::try_from(elapsed / 20_000)?,
            video_index: usize::try_from(elapsed * 30 / 1_000_000)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
