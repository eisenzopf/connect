//! Fresh, effect-free resource admission after the original RTC answer.
use connect_protocol::{Error, Result};
use connect_store::AuthorityWait;
use std::{future::Future, time::Duration};
use tokio::time::Instant;

fn expired() -> Error {
    Error::new("timeout", "RTC replacement resource admission deadline")
}

/// Never wrap allocation, physical start, SDP, negotiation, mute or new effect
/// recording here. Exact idempotent SQL settlement of an ALREADY captured
/// positive receipt may use read_busy; it grants no new IO and repeats no effect.
/// Each retry must derive fresh authority and exact resource metadata. The
/// original absolute budget and cancellation owner never change.
pub(crate) async fn read<T, F, Fut, Cancel>(
    until: Instant,
    wait: &AuthorityWait,
    cancelled: Cancel,
    current: F,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
    Cancel: Future<Output = ()>,
{
    read_with(until, wait, cancelled, current, |error| {
        matches!(
            error.code.as_str(),
            "authority-busy" | "storage-unavailable"
        )
    })
    .await
}

/// Reobserve only immediate, effect-free metadata exclusion. In particular a
/// storage outage is not a reason to repeat provenance issuance or the reply read.
/// The caller must retain its exact original attempt/read and submit no effect.
pub(super) async fn read_busy<T, F, Fut, Cancel>(
    until: Instant,
    wait: &AuthorityWait,
    cancelled: Cancel,
    current: F,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
    Cancel: Future<Output = ()>,
{
    read_with(until, wait, cancelled, current, |error| {
        error.code == "authority-busy"
    })
    .await
}

async fn read_with<T, F, Fut, Cancel>(
    until: Instant,
    wait: &AuthorityWait,
    cancelled: Cancel,
    mut current: F,
    transient: fn(&Error) -> bool,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
    Cancel: Future<Output = ()>,
{
    tokio::pin!(cancelled);
    loop {
        tokio::select! { biased;
            _ = &mut cancelled => return Err(Error::forbidden()),
            _ = tokio::time::sleep_until(until) => return Err(expired()),
            _ = std::future::ready(()) => {},
        }
        // Timer-wheel readiness may lag the actual monotonic deadline, even
        // when sleep_until was constructed with an already expired instant.
        if Instant::now() >= until {
            return Err(expired());
        }
        let observed = current();
        tokio::pin!(observed);
        let result = tokio::select! { biased;
            _ = &mut cancelled => {
                wait.cancel();
                // The metadata closure owns its blocking Store task. Join it
                // and drop any late reader before the physical owner cleans up.
                let _ = observed.await;
                return Err(Error::forbidden());
            },
            _ = tokio::time::sleep_until(until) => {
                wait.cancel();
                let _ = observed.await;
                return Err(expired());
            },
            result = &mut observed => result,
        };
        // Cancellation/expiry during the final metadata poll cannot promote
        // that result into a grant for stream negotiation.
        tokio::select! { biased;
            _ = &mut cancelled => return Err(Error::forbidden()),
            _ = tokio::time::sleep_until(until) => return Err(expired()),
            _ = std::future::ready(()) => {},
        }
        if Instant::now() >= until {
            return Err(expired());
        }
        match result {
            Ok(value) => return Ok(value),
            Err(error) if transient(&error) => {
                tokio::select! { biased;
                    _ = &mut cancelled => return Err(Error::forbidden()),
                    _ = tokio::time::sleep_until(until) => return Err(expired()),
                    _ = tokio::time::sleep(Duration::from_millis(20)) => {},
                }
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use tokio::sync::Notify;
    use tokio_util::sync::CancellationToken;

    struct Reader(Arc<AtomicUsize>);
    impl Drop for Reader {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn busy_resource_read_drops_partial_authority_before_fresh_admission() {
        for code in ["authority-busy", "storage-unavailable"] {
            let calls = AtomicUsize::new(0);
            let dropped = Arc::new(AtomicUsize::new(0));
            let stop = CancellationToken::new();
            let until = Instant::now() + Duration::from_secs(1);
            let wait = AuthorityWait::new(until.saturating_duration_since(Instant::now()));
            let admitted = read(until, &wait, stop.cancelled(), || async {
                let attempt = calls.fetch_add(1, Ordering::SeqCst);
                assert_eq!(dropped.load(Ordering::SeqCst), attempt);
                let fresh = Reader(dropped.clone());
                if attempt == 0 {
                    Err(Error::new(code, "Original metadata is busy"))
                } else {
                    Ok((attempt, fresh))
                }
            })
            .await
            .unwrap();
            assert_eq!(calls.load(Ordering::SeqCst), 2);
            assert_eq!(admitted.0, 1);
            assert_eq!(dropped.load(Ordering::SeqCst), 1);
            drop(admitted);
            assert_eq!(dropped.load(Ordering::SeqCst), 2);
        }
    }

    #[tokio::test]
    async fn current_revocation_after_contention_is_not_retried() {
        let calls = AtomicUsize::new(0);
        let stop = CancellationToken::new();
        let until = Instant::now() + Duration::from_secs(1);
        let wait = AuthorityWait::new(until.saturating_duration_since(Instant::now()));
        let result: Result<()> = read(until, &wait, stop.cancelled(), || async {
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                Err(Error::new("authority-busy", "Original metadata is busy"))
            } else {
                Err(Error::forbidden())
            }
        })
        .await;
        assert_eq!(result.unwrap_err().code, "forbidden");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn permanent_refusal_is_never_retried() {
        let calls = AtomicUsize::new(0);
        let stop = CancellationToken::new();
        let until = Instant::now() + Duration::from_secs(1);
        let wait = AuthorityWait::new(until.saturating_duration_since(Instant::now()));
        let result: Result<()> = read(until, &wait, stop.cancelled(), || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Err(Error::new("conflict", "Original target changed"))
        })
        .await;
        assert_eq!(result.unwrap_err().code, "conflict");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cancellation_joins_and_discards_the_original_pending_reader() {
        let stop = CancellationToken::new();
        let started = Notify::new();
        let release = Notify::new();
        let dropped = Arc::new(AtomicUsize::new(0));
        let until = Instant::now() + Duration::from_secs(1);
        let wait = AuthorityWait::new(until.saturating_duration_since(Instant::now()));
        let admission = read(until, &wait, stop.cancelled(), || async {
            let reader = Reader(dropped.clone());
            started.notify_one();
            release.notified().await;
            Ok(reader)
        });
        let cancel = async {
            started.notified().await;
            stop.cancel();
            release.notify_one();
        };
        let (result, ()) = tokio::join!(admission, cancel);
        assert_eq!(result.err().unwrap().code, "forbidden");
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn original_expired_deadline_cannot_start_or_renew_admission() {
        let stop = CancellationToken::new();
        let calls = AtomicUsize::new(0);
        let until = Instant::now();
        let wait = AuthorityWait::new(Duration::ZERO);
        let result: Result<()> = read(until, &wait, stop.cancelled(), || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .await;
        assert_eq!(result.unwrap_err().code, "timeout");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn synchronous_metadata_finishing_after_the_deadline_cannot_lend_a_reader() {
        let stop = CancellationToken::new();
        let dropped = Arc::new(AtomicUsize::new(0));
        let until = Instant::now() + Duration::from_millis(10);
        let wait = AuthorityWait::new(until.saturating_duration_since(Instant::now()));
        let result = read(until, &wait, stop.cancelled(), || async {
            let reader = Reader(dropped.clone());
            // One synchronous metadata poll can cross the original deadline
            // before the runtime gets a chance to advance its timer wheel.
            std::thread::sleep(Duration::from_millis(20));
            Ok(reader)
        })
        .await;
        assert_eq!(result.err().unwrap().code, "timeout");
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_reader_finishing_at_the_original_deadline_is_not_admitted() {
        let stop = CancellationToken::new();
        let dropped = Arc::new(AtomicUsize::new(0));
        let until = Instant::now() + Duration::from_millis(20);
        let wait = AuthorityWait::new(until.saturating_duration_since(Instant::now()));
        let result = read(until, &wait, stop.cancelled(), || async {
            let reader = Reader(dropped.clone());
            tokio::time::sleep_until(until).await;
            Ok(reader)
        })
        .await;
        assert_eq!(result.err().unwrap().code, "timeout");
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
    }
}

#[cfg(test)]
mod start_metadata_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio_util::sync::CancellationToken;

    // The attempt is intentionally neither Clone nor consumed by observation.
    struct OriginalAttempt(usize);

    #[tokio::test]
    async fn provenance_busy_reobserves_the_same_noncloneable_attempt_without_restarting_it() {
        let attempt = OriginalAttempt(42);
        let address = std::ptr::addr_of!(attempt) as usize;
        let observations = AtomicUsize::new(0);
        let until = Instant::now() + Duration::from_secs(1);
        let wait = AuthorityWait::new(Duration::from_secs(1));
        let stop = CancellationToken::new();
        let result = read_busy(until, &wait, stop.cancelled(), || {
            assert_eq!(std::ptr::addr_of!(attempt) as usize, address);
            let observed = observations.fetch_add(1, Ordering::SeqCst);
            std::future::ready(if observed == 0 {
                Err(Error::new(
                    "authority-busy",
                    "Original primary SQL mutex is excluded",
                ))
            } else {
                Ok(attempt.0)
            })
        })
        .await
        .unwrap();
        assert_eq!(result, 42);
        assert_eq!(observations.load(Ordering::SeqCst), 2);
        assert_eq!(attempt.0, 42);
    }

    #[tokio::test]
    async fn provenance_storage_or_permanent_refusal_is_not_reobserved() {
        for code in [
            "storage-unavailable",
            "storage-error",
            "forbidden",
            "conflict",
        ] {
            let calls = AtomicUsize::new(0);
            let until = Instant::now() + Duration::from_secs(1);
            let wait = AuthorityWait::new(Duration::from_secs(1));
            let stop = CancellationToken::new();
            let result: Result<()> = read_busy(until, &wait, stop.cancelled(), || {
                calls.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Err(Error::new(code, "Exact original refusal")))
            })
            .await;
            assert_eq!(result.unwrap_err().code, code);
            assert_eq!(calls.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn provenance_cancellation_after_busy_cannot_issue_an_origin() {
        let stop = CancellationToken::new();
        let calls = AtomicUsize::new(0);
        let until = Instant::now() + Duration::from_secs(1);
        let wait = AuthorityWait::new(Duration::from_secs(1));
        let result: Result<()> = read_busy(until, &wait, stop.cancelled(), || {
            calls.fetch_add(1, Ordering::SeqCst);
            stop.cancel();
            std::future::ready(Err(Error::new("authority-busy", "Original exclusion")))
        })
        .await;
        assert_eq!(result.unwrap_err().code, "forbidden");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
