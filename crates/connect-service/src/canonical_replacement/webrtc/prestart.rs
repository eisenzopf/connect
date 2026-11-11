//! Capture-only control bounds; never a positive authority or media grant.
use connect_protocol::{Error, Result};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::Instant;

/// The caller passes expires_at from the ACTUAL current StartRead, before any
/// retry or once-only begin. Reobserving a later projection cannot renew it.
pub(super) fn narrow(original: i64, until: Instant, captured: i64) -> Result<(i64, Instant)> {
    let expires = original.min(captured);
    let wall = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::storage())?;
    let remaining = Duration::from_secs(u64::try_from(expires).map_err(|_| Error::forbidden())?)
        .checked_sub(wall)
        .ok_or_else(|| Error::new("timeout", "Original RTC Start projection expired"))?;
    let deadline = until.min(Instant::now() + remaining);
    if Instant::now() >= deadline {
        return Err(Error::new(
            "timeout",
            "Original RTC Start projection expired",
        ));
    }
    Ok((expires, deadline))
}
/// Final synchronous boundary, after every awaited policy/projection read and
/// immediately before the actual typed Store begin. This grants no authority.
pub(super) fn before_begin<T>(
    stop: &tokio_util::sync::CancellationToken,
    begin: impl FnOnce() -> Result<T>,
) -> Result<T> {
    if stop.is_cancelled() {
        return Err(Error::forbidden());
    }
    begin()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn first_projection_shorter_than_target_proof_pins_actual_original_expiry() {
        let now = chrono::Utc::now().timestamp();
        let requested = Instant::now() + Duration::from_secs(5);
        let (first, deadline) = narrow(now + 5, requested, now + 1).unwrap();
        assert_eq!(first, now + 1);
        assert!(deadline < requested);
        let (later, after) = narrow(first, deadline, now + 5).unwrap();
        assert_eq!(later, first);
        assert!(after <= deadline);
        let (earliest, shorter) = narrow(first, after, now + 1).unwrap();
        assert_eq!(earliest, first);
        assert!(shorter <= after);
    }
    #[tokio::test]
    async fn naturally_expired_first_projection_cannot_be_renewed_by_a_later_read_or_begin() {
        let now = chrono::Utc::now().timestamp();
        let (first, deadline) =
            narrow(now + 5, Instant::now() + Duration::from_secs(5), now + 1).unwrap();
        tokio::time::sleep_until(deadline).await;
        assert_eq!(
            narrow(first, deadline, chrono::Utc::now().timestamp() + 5)
                .unwrap_err()
                .code,
            "timeout"
        );
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let wait = connect_store::AuthorityWait::new(Duration::ZERO);
        let stop = tokio_util::sync::CancellationToken::new();
        let result = super::super::admission::read_busy(deadline, &wait, stop.cancelled(), || {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            std::future::ready(Ok(()))
        })
        .await;
        assert_eq!(result.unwrap_err().code, "timeout");
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }
    #[test]
    fn expired_original_control_cutoff_cannot_issue_a_fresh_start_window() {
        let now = chrono::Utc::now().timestamp();
        for captured in [now, now - 1] {
            assert!(narrow(now + 5, Instant::now() + Duration::from_secs(5), captured).is_err());
        }
        assert!(narrow(now + 5, Instant::now(), now + 5).is_err());
    }
    #[tokio::test]
    async fn runtime_stop_during_projection_wait_cannot_enter_the_final_begin_callback() {
        let stop = tokio_util::sync::CancellationToken::new();
        let entered = tokio::sync::Notify::new();
        let release = tokio::sync::Notify::new();
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let pending = async {
            entered.notify_one();
            release.notified().await;
            before_begin(&stop, || {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            })
        };
        let stopped = async {
            entered.notified().await;
            stop.cancel();
            release.notify_one();
        };
        let (result, ()) = tokio::join!(pending, stopped);
        assert_eq!(result.unwrap_err().code, "forbidden");
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }
}
