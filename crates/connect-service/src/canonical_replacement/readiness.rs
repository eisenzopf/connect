//! Wait for the original preparation capture before any target allocation.
use connect_protocol::{Error, Result};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

// `preparing` also covers the reservation and once-only Core call. It does
// not mean the owned ticket is already installed and durably recorded. This
// loop only observes local metadata; the caller performs allocation once.
pub(super) async fn wait<T>(
    deadline: Instant,
    stop: &CancellationToken,
    invocation: &CancellationToken,
    physical: &CancellationToken,
    mut read: impl FnMut() -> Result<Option<T>>,
) -> Result<T> {
    loop {
        if stop.is_cancelled() || invocation.is_cancelled() || physical.is_cancelled() {
            return Err(Error::forbidden());
        }
        if Instant::now() >= deadline {
            return Err(Error::new(
                "not-ready",
                "Original preparation is not recorded",
            ));
        }
        if let Some(value) = read()? {
            return Ok(value);
        }
        tokio::select! {
            biased;
            _ = stop.cancelled() => return Err(Error::forbidden()),
            _ = invocation.cancelled() => return Err(Error::forbidden()),
            _ = physical.cancelled() => return Err(Error::forbidden()),
            _ = tokio::time::sleep_until(deadline.min(Instant::now() + std::time::Duration::from_millis(20))) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    };
    use std::time::Duration;

    #[tokio::test]
    async fn reservation_precedes_capture_without_repeating_allocation() {
        let recorded = Arc::new(AtomicBool::new(false));
        let capture = recorded.clone();
        let writer = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(25)).await;
            capture.store(true, Ordering::Release);
        });
        let reads = AtomicUsize::new(0);
        let value = wait(
            Instant::now() + Duration::from_secs(1),
            &CancellationToken::new(),
            &CancellationToken::new(),
            &CancellationToken::new(),
            || {
                reads.fetch_add(1, Ordering::Relaxed);
                Ok(recorded
                    .load(Ordering::Acquire)
                    .then_some("original-ticket"))
            },
        )
        .await
        .unwrap();
        writer.await.unwrap();
        assert_eq!(value, "original-ticket");
        assert!(reads.load(Ordering::Relaxed) >= 2);
        // Only the returned captured ticket can cross the caller's begin.
    }

    #[tokio::test]
    async fn missing_capture_expires_without_allocation() {
        let result = wait::<()>(
            Instant::now() + Duration::from_millis(25),
            &CancellationToken::new(),
            &CancellationToken::new(),
            &CancellationToken::new(),
            || Ok(None),
        )
        .await;
        assert_eq!(result.unwrap_err().code, "not-ready");
    }

    #[tokio::test]
    async fn stop_invocation_and_physical_cancellation_refuse_even_a_ready_capture() {
        for cancelled in 0..3 {
            let tokens = [
                CancellationToken::new(),
                CancellationToken::new(),
                CancellationToken::new(),
            ];
            tokens[cancelled].cancel();
            let result = wait(
                Instant::now() + Duration::from_secs(1),
                &tokens[0],
                &tokens[1],
                &tokens[2],
                || Ok(Some(())),
            )
            .await;
            assert_eq!(result.unwrap_err().code, "forbidden");
        }
    }

    #[tokio::test]
    async fn authority_refusal_is_not_retried() {
        let mut reads = 0;
        let result = wait::<()>(
            Instant::now() + Duration::from_secs(1),
            &CancellationToken::new(),
            &CancellationToken::new(),
            &CancellationToken::new(),
            || {
                reads += 1;
                Err(Error::forbidden())
            },
        )
        .await;
        assert_eq!(result.unwrap_err().code, "forbidden");
        assert_eq!(reads, 1);
    }
}
