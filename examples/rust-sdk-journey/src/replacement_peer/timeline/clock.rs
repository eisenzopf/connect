//! Match Node/libuv's same-host clock, never reconstruct elapsed time from UTC.
use super::*;
use nix::time::{clock_gettime, ClockId};

#[derive(Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProgrammeClock {
    kind: String,
    domain: String,
    origin_ns: String,
}
fn domain() -> Result<(&'static str, String)> {
    #[cfg(target_os = "linux")]
    {
        Ok((
            "linux-monotonic-v1",
            format!(
                "{}/{}",
                std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?
                    .trim()
                    .to_lowercase(),
                std::fs::read_link("/proc/self/ns/time")?
                    .to_str()
                    .ok_or("time namespace invalid")?
            ),
        ))
    }
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("/usr/sbin/sysctl")
            .args(["-n", "kern.bootsessionuuid"])
            .output()?;
        if !output.status.success() {
            return Err("boot clock domain unavailable".into());
        }
        Ok((
            "darwin-continuous-v1",
            String::from_utf8(output.stdout)?.trim().to_lowercase(),
        ))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Err("unsupported programme clock host".into())
    }
}
fn now_ns() -> Result<u64> {
    #[cfg(target_os = "macos")]
    let id = ClockId::from_raw(nix::libc::CLOCK_MONOTONIC_RAW);
    #[cfg(not(target_os = "macos"))]
    let id = ClockId::CLOCK_MONOTONIC;
    let now = clock_gettime(id)?;
    u64::try_from(now.tv_sec())?
        .checked_mul(1_000_000_000)
        .and_then(|n| n.checked_add(u64::try_from(now.tv_nsec()).ok()?))
        .ok_or_else(|| "monotonic clock overflow".into())
}
impl ProgrammeClock {
    pub fn validate(&self) -> Result<()> {
        if !matches!(
            self.kind.as_str(),
            "linux-monotonic-v1" | "darwin-continuous-v1"
        ) || self.domain.is_empty()
            || self.domain.len() > 160
            || self.origin_ns.is_empty()
            || self.origin_ns.len() > 20
            || self.origin_ns.starts_with('0')
            || !self.origin_ns.bytes().all(|b| b.is_ascii_digit())
            || self.origin_ns.parse::<u64>().is_err()
        {
            return Err("invalid original monotonic programme anchor".into());
        }
        Ok(())
    }
    fn elapsed_at(&self, kind: &str, domain: &str, now: u64, seconds: u16) -> Result<Duration> {
        self.validate()?;
        if self.kind != kind || self.domain != domain {
            return Err("foreign programme clock domain".into());
        }
        let elapsed = now
            .checked_sub(self.origin_ns.parse()?)
            .ok_or("programme has not started")?;
        if seconds == 0 || elapsed >= u64::from(seconds) * 1_000_000_000 {
            return Err("original monotonic programme expired".into());
        }
        Ok(Duration::from_nanos(elapsed))
    }
    pub fn join(&self, unix: u64, seconds: u16) -> Result<(Program, tokio::time::Instant)> {
        let (kind, domain) = domain()?;
        // Bound the conversion to the local timer representation. Using the
        // earlier Instant can only narrow the original cutoff, never renew it.
        for _ in 0..3 {
            let before = tokio::time::Instant::now();
            let elapsed = self.elapsed_at(kind, &domain, now_ns()?, seconds)?;
            if before.elapsed() > Duration::from_millis(1) {
                continue;
            }
            let start = before
                .checked_sub(elapsed)
                .ok_or("programme origin out of bounds")?;
            return Ok((
                Program::from_elapsed(u128::from(unix), elapsed, seconds)?,
                start,
            ));
        }
        Err("monotonic programme sampling exceeded bound".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_monotonic_origin_survives_wall_steps_and_preserves_frame_cutoffs() {
        let clock = ProgrammeClock {
            kind: "linux-monotonic-v1".into(),
            domain: "same-boot/time:[1]".into(),
            origin_ns: "1000000000000".into(),
        };
        let unix = 1_793_789_677_045_000u64;
        let elapsed_us = 44_943_000u64;
        let now = 1_000_000_000_000 + elapsed_us * 1000;
        for wall_step_us in [-100_000i64, 100_000, -86_400_000_000, 86_400_000_000] {
            let wall_after = i128::from(unix) + i128::from(elapsed_us) + i128::from(wall_step_us);
            // This is the old reconstruction which v119 proved inconsistent.
            assert_ne!(wall_after - i128::from(unix), i128::from(elapsed_us));
            let elapsed = clock
                .elapsed_at(&clock.kind, &clock.domain, now, 62)
                .unwrap();
            let program = Program::from_elapsed(u128::from(unix), elapsed, 62).unwrap();
            assert_eq!(program.elapsed.as_micros(), u128::from(elapsed_us));
            assert_eq!((program.audio_index, program.video_index), (2247, 1348));
            assert_eq!(program.unix, u128::from(unix));
            // The original Data bound is still exactly eight seconds, and the
            // fixed media programme still ends at 62s in that same coordinate.
            assert_eq!(
                Duration::from_millis(52943) - elapsed,
                Duration::from_secs(8)
            );
            assert_eq!(
                Duration::from_secs(62) - elapsed,
                Duration::from_millis(17057)
            );
            assert_eq!(
                ntp(program.unix + program.audio_index as u128 * 20000),
                ntp(u128::from(unix) + 44_940_000)
            );
        }
        for (kind, boot, when, seconds) in [
            ("darwin-continuous-v1", clock.domain.as_str(), now, 62),
            (clock.kind.as_str(), "foreign", now, 62),
            (
                clock.kind.as_str(),
                clock.domain.as_str(),
                999_999_999_999,
                62,
            ),
            (
                clock.kind.as_str(),
                clock.domain.as_str(),
                1_062_000_000_000,
                62,
            ),
            (clock.kind.as_str(), clock.domain.as_str(), now, 0),
        ] {
            assert!(clock.elapsed_at(kind, boot, when, seconds).is_err());
        }
    }

    #[test]
    fn actual_node_anchor_joins_rust_in_the_same_boot_clock_domain() {
        let module = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/lib/transport_programme_clock.mjs")
            .canonicalize()
            .unwrap();
        let script = format!("const m=await import({}); const c=m.startProgrammeClock(); console.log(JSON.stringify(c));",
            serde_json::to_string(&format!("file://{}", module.display())).unwrap());
        let output = std::process::Command::new("node")
            .args(["--input-type=module", "-e", &script])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let clock: ProgrammeClock = serde_json::from_slice(&output.stdout).unwrap();
        let before = now_ns().unwrap() - clock.origin_ns.parse::<u64>().unwrap();
        let (program, origin) = clock.join(1_793_789_677_045_000, 62).unwrap();
        let after = now_ns().unwrap() - clock.origin_ns.parse::<u64>().unwrap();
        assert!(
            before <= program.elapsed.as_nanos() as u64
                && program.elapsed.as_nanos() as u64 <= after
        );
        assert!(program.elapsed < Duration::from_secs(5));
        assert!(origin.elapsed() >= program.elapsed);
        assert!(origin.elapsed() - program.elapsed < Duration::from_millis(20));
    }
}
