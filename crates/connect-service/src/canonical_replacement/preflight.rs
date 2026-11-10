//! Keep the original job through metadata contention without repeating begun IO.
use connect_protocol::{Error, Result};
use std::{future::Future, time::Duration};
use tokio_util::sync::CancellationToken;

fn transient(error: &Error) -> bool {
    matches!(
        error.code.as_str(),
        "authority-busy" | "storage-unavailable"
    )
}
async fn pause(stop: &CancellationToken) -> Result<()> {
    tokio::select! {
        biased;
        _ = stop.cancelled() => Err(Error::forbidden()),
        _ = tokio::time::sleep(Duration::from_millis(50)) => Ok(()),
    }
}
pub(super) async fn run<F, P, C, G>(
    stop: &CancellationToken,
    mut prepare: F,
    mut unstarted: C,
) -> Result<()>
where
    F: FnMut() -> P,
    P: Future<Output = Result<()>>,
    C: FnMut() -> G,
    G: Future<Output = Result<bool>>,
{
    loop {
        if stop.is_cancelled() {
            return Err(Error::forbidden());
        }
        let error = match prepare().await {
            Ok(()) => return Ok(()),
            Err(error) if transient(&error) => error,
            Err(error) => return Err(error),
        };
        // Only reads may repeat while the original's phase is uncertain. The
        // Store checks current authority, expiry and the exact original intent,
        // and refuses whole-preflight repetition once any reservation/effect
        // exists. There is no fresh operation ID, credential or Core attempt.
        loop {
            pause(stop).await?;
            match unstarted().await {
                Ok(true) => break,
                Ok(false) => return Err(error),
                Err(error) if transient(&error) => continue,
                Err(error) => return Err(error),
            }
        }
    }
}

// The outer preflight runner deliberately refuses replay once a reservation
// exists. This narrower loop owns that SAME original opaque reservation and
// retries only its current resolution/atomic not-yet-begun Store admission.
// The caller puts Core preparation strictly AFTER this returns success.
pub(super) async fn reserved<T, F, P>(
    stop: &CancellationToken,
    deadline: tokio::time::Instant,
    mut begin: F,
) -> Result<T>
where
    F: FnMut() -> P,
    P: Future<Output = Result<T>>,
{
    loop {
        if stop.is_cancelled() {
            return Err(Error::forbidden());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(Error::new(
                "authority-busy",
                "Original replacement begin cutoff expired",
            ));
        }
        let error = match begin().await {
            Ok(value) => return Ok(value),
            Err(error) if transient(&error) => error,
            Err(error) => return Err(error),
        };
        tokio::select! {
            biased;
            _ = stop.cancelled() => return Err(Error::forbidden()),
            _ = tokio::time::sleep_until(deadline.min(tokio::time::Instant::now()+Duration::from_millis(20))) => {}
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn busy_phase_reads_do_not_repeat_preparation_until_original_is_known_unstarted() {
        let calls = AtomicUsize::new(0);
        let reads = AtomicUsize::new(0);
        run(
            &CancellationToken::new(),
            || async {
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err(Error::new("authority-busy", "original preflight busy"))
                } else {
                    Ok(())
                }
            },
            || async {
                assert_eq!(calls.load(Ordering::SeqCst), 1);
                if reads.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err(Error::new(
                        "authority-busy",
                        "original phase not yet readable",
                    ))
                } else {
                    Ok(true)
                }
            },
        )
        .await
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(reads.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_reserved_or_effectful_original_never_repeats_preparation() {
        let calls = AtomicUsize::new(0);
        let error = run(
            &CancellationToken::new(),
            || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Err(Error::new(
                    "storage-unavailable",
                    "metadata capture uncertain",
                ))
            },
            || async { Ok(false) },
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "storage-unavailable");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn current_refusal_propagates_and_shutdown_stops_unreadable_phase_without_replay() {
        let calls = AtomicUsize::new(0);
        let error = run(
            &CancellationToken::new(),
            || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Err(Error::new("authority-busy", "preflight busy"))
            },
            || async { Err(Error::forbidden()) },
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "forbidden");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let stop = CancellationToken::new();
        let cancelling = async {
            tokio::time::sleep(Duration::from_millis(75)).await;
            stop.cancel();
        };
        let work = run(
            &stop,
            || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Err(Error::new("authority-busy", "preflight busy"))
            },
            || async { Err(Error::new("authority-busy", "phase busy")) },
        );
        let (result, ()) = tokio::join!(work, cancelling);
        assert_eq!(result.unwrap_err().code, "forbidden");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}

#[cfg(test)]
mod reserved_tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    #[tokio::test]
    async fn original_reserved_begin_retries_without_recreating_reservation_or_holding_iteration_guards(
    ) {
        struct Interval(Arc<AtomicUsize>);
        impl Drop for Interval {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }
        let reservation = Arc::new("one-original-reservation");
        let identity = Arc::as_ptr(&reservation);
        let held = Arc::new(AtomicUsize::new(0));
        let calls = AtomicUsize::new(0);
        let actual = reserved(
            &CancellationToken::new(),
            tokio::time::Instant::now() + Duration::from_secs(1),
            || async {
                assert_eq!(Arc::as_ptr(&reservation), identity);
                assert_eq!(
                    held.fetch_add(1, Ordering::SeqCst),
                    0,
                    "previous refused Read/coordinator must drop before retry"
                );
                let _interval = Interval(held.clone());
                match calls.fetch_add(1, Ordering::SeqCst) {
                    0 => Err(Error::new(
                        "authority-busy",
                        "original reserved begin blocked",
                    )),
                    1 => Err(Error::storage()),
                    _ => Ok("same-original-attempt"),
                }
            },
        )
        .await
        .unwrap();
        assert_eq!(actual, "same-original-attempt");
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert_eq!(held.load(Ordering::SeqCst), 0);
        assert_eq!(Arc::strong_count(&reservation), 1);
    }
    #[tokio::test]
    async fn reserved_begin_preserves_permanent_refusal_and_stops_at_original_cutoff_or_cancel() {
        let calls = AtomicUsize::new(0);
        let error = reserved(
            &CancellationToken::new(),
            tokio::time::Instant::now() + Duration::from_secs(1),
            || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Err::<(), _>(Error::forbidden())
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "forbidden");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let error = reserved(
            &CancellationToken::new(),
            tokio::time::Instant::now(),
            || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "authority-busy");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "expired cutoff must not call begin"
        );
        let stop = CancellationToken::new();
        let error = reserved(
            &stop,
            tokio::time::Instant::now() + Duration::from_secs(1),
            || async {
                calls.fetch_add(1, Ordering::SeqCst);
                stop.cancel();
                Err::<(), _>(Error::new("authority-busy", "same original blocked"))
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "forbidden");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
