//! Fixture-only joins of independent original peers. Never timeout/drop a close
//! future, retry a close, or renew the original programme qualification cutoff.
use super::*;
use futures_util::{future::join_all, FutureExt};
use std::{future::Future, pin::Pin};

type Attempt<'a> = Pin<Box<dyn Future<Output = std::result::Result<(), String>> + 'a>>;
pub(super) struct OriginalClose<'a> {
    kind: &'static str,
    index: usize,
    attempt: Attempt<'a>,
}
impl<'a> OriginalClose<'a> {
    pub(super) fn new(kind: &'static str, index: usize, attempt: Attempt<'a>) -> Self {
        Self {
            kind,
            index,
            attempt,
        }
    }
}
pub(super) struct Observation {
    pub kind: &'static str,
    pub index: usize,
    pub entered_us: u128,
    pub returned_us: u128,
    pub ok: bool,
    pub original_cutoff_us: Option<u128>,
}
pub(super) struct Closed {
    pub failures: Vec<String>,
    pub observations: Vec<Observation>,
}
pub(super) async fn join_originals(
    originals: Vec<OriginalClose<'_>>,
    began: Instant,
    until: Option<Instant>,
) -> Closed {
    // Await all exact original futures, including on an early error/panic.
    // Each callback retains its existing transport/request cleanup bounds.
    let results = join_all(originals.into_iter().map(|original| async move {
        let entered = Instant::now();
        let result = match AssertUnwindSafe(original.attempt).catch_unwind().await {
            Ok(result) => result,
            Err(panic) => Err(panic_text(panic)),
        };
        let returned = Instant::now();
        let observation = Observation {
            kind: original.kind,
            index: original.index,
            entered_us: entered.saturating_duration_since(began).as_micros(),
            returned_us: returned.saturating_duration_since(began).as_micros(),
            ok: result.is_ok(),
            original_cutoff_us: until.map(|at| at.saturating_duration_since(began).as_micros()),
        };
        let mut failures = Vec::new();
        if let Err(error) = result {
            failures.push(format!(
                "original peer close kind={} index={}: {error}",
                original.kind, original.index
            ));
        }
        if until.is_some_and(|until| returned > until) {
            failures.push(format!(
                "original peer close kind={} index={} returned beyond original programme cutoff",
                original.kind, original.index
            ));
        }
        (observation, failures)
    }))
    .await;
    let mut closed = Closed {
        failures: Vec::new(),
        observations: Vec::with_capacity(results.len()),
    };
    for (observation, failures) in results {
        closed.observations.push(observation);
        closed.failures.extend(failures);
    }
    closed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[tokio::test]
    async fn all_eight_original_closes_enter_before_any_protocol_drain_is_released() {
        let entered = Arc::new(AtomicUsize::new(0));
        let completed = Arc::new(AtomicUsize::new(0));
        let release = CancellationToken::new();
        let began = Instant::now();
        let until = began + Duration::from_secs(5);
        let attempts = (0..8)
            .map(|index| {
                let entered = entered.clone();
                let completed = completed.clone();
                let release = release.clone();
                OriginalClose::new(
                    if index < 4 { "target" } else { "sdk" },
                    index % 4,
                    Box::pin(async move {
                        entered.fetch_add(1, Ordering::SeqCst);
                        release.cancelled().await;
                        completed.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    }),
                )
            })
            .collect();
        let mut work = Box::pin(join_originals(attempts, began, Some(until)));
        let first = futures_util::poll!(work.as_mut());
        let before_release = entered.load(Ordering::SeqCst);
        release.cancel();
        let closed = match first {
            std::task::Poll::Ready(closed) => closed,
            std::task::Poll::Pending => work.await,
        };
        // Assertions occur only after the gate is released and all callbacks
        // have returned. A serial implementation enters only its first peer.
        assert_eq!(before_release, 8);
        assert_eq!(completed.load(Ordering::SeqCst), 8);
        assert!(closed.failures.is_empty(), "{:?}", closed.failures);
        assert_eq!(closed.observations.len(), 8);
        for (index, value) in closed.observations.iter().enumerate() {
            assert_eq!(
                (value.kind, value.index),
                (if index < 4 { "target" } else { "sdk" }, index % 4)
            );
            assert!(value.ok);
            assert_eq!(
                value.original_cutoff_us,
                Some(until.duration_since(began).as_micros())
            );
        }
    }

    #[tokio::test]
    async fn early_close_errors_and_expired_cutoff_still_join_every_original_without_renewal() {
        for panic_first in [false, true] {
            let entered = Arc::new(AtomicUsize::new(0));
            let completed = Arc::new(AtomicUsize::new(0));
            let release = CancellationToken::new();
            let began = Instant::now();
            let until = began + Duration::from_secs(5);
            let attempts = (0..8)
                .map(|index| {
                    let entered = entered.clone();
                    let completed = completed.clone();
                    let release = release.clone();
                    OriginalClose::new(
                        "controlled",
                        index,
                        Box::pin(async move {
                            entered.fetch_add(1, Ordering::SeqCst);
                            if index == 0 {
                                completed.fetch_add(1, Ordering::SeqCst);
                                if panic_first {
                                    panic!("original controlled close panic");
                                }
                                return Err("original controlled close error".into());
                            }
                            release.cancelled().await;
                            completed.fetch_add(1, Ordering::SeqCst);
                            Ok(())
                        }),
                    )
                })
                .collect();
            let mut work = Box::pin(join_originals(attempts, began, Some(until)));
            let first = futures_util::poll!(work.as_mut());
            let before_release = entered.load(Ordering::SeqCst);
            release.cancel();
            let closed = match first {
                std::task::Poll::Ready(closed) => closed,
                std::task::Poll::Pending => work.await,
            };
            assert_eq!(before_release, 8);
            assert_eq!(completed.load(Ordering::SeqCst), 8);
            assert_eq!(closed.observations.len(), 8);
            assert_eq!(closed.failures.len(), 1);
            assert!(closed.failures[0].contains("kind=controlled index=0"));
            assert!(closed.failures[0].contains(if panic_first {
                "original controlled close panic"
            } else {
                "original controlled close error"
            }));
        }
        // Expiry does not skip an original cleanup callback or mint a new
        // relative timeout. All successful late callbacks remain disqualified.
        let began = Instant::now() - Duration::from_secs(1);
        let until = began;
        let completed = Arc::new(AtomicUsize::new(0));
        let attempts = (0..8)
            .map(|index| {
                let completed = completed.clone();
                OriginalClose::new(
                    "expired",
                    index,
                    Box::pin(async move {
                        completed.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    }),
                )
            })
            .collect();
        let closed = join_originals(attempts, began, Some(until)).await;
        assert_eq!(completed.load(Ordering::SeqCst), 8);
        assert_eq!(closed.failures.len(), 8);
        assert!(closed
            .failures
            .iter()
            .all(|error| error.contains("beyond original programme cutoff")));
        assert!(closed
            .observations
            .iter()
            .all(|value| value.ok && value.original_cutoff_us == Some(0)));
    }
}
