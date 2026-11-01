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
