//! Never park the role coordinator behind an authority reader that needs it.
use connect_protocol::{Error, Result};
use std::{future::Future, time::Duration};
use tokio::sync::{Mutex, MutexGuard};
use tokio_util::sync::CancellationToken;

fn busy() -> Error {
    Error::new("authority-busy", "Replacement projection metadata is busy")
}

/// Only fresh, effect-free metadata belongs in this closure. A refused read
/// releases the coordinator and every partial reader before the next poll.
pub(super) async fn read<'a, T, F, Fut>(
    roles: &'a Mutex<()>,
    stop: &CancellationToken,
    until: tokio::time::Instant,
    mut current: F,
) -> Result<(MutexGuard<'a, ()>, T)>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    loop {
        if stop.is_cancelled() {
            return Err(Error::forbidden());
        }
        if tokio::time::Instant::now() >= until {
            return Err(busy());
        }
        let guard = tokio::select! { biased;
            _ = stop.cancelled() => return Err(Error::forbidden()),
            _ = tokio::time::sleep_until(until) => return Err(busy()),
            guard = roles.lock() => guard,
        };
        if stop.is_cancelled() {
            return Err(Error::forbidden());
        }
        if tokio::time::Instant::now() >= until {
            return Err(busy());
        }
        match current().await {
            Ok(value) => {
                if stop.is_cancelled() {
                    return Err(Error::forbidden());
                }
                if tokio::time::Instant::now() >= until {
                    return Err(busy());
                }
                return Ok((guard, value));
            }
            Err(error)
                if matches!(
                    error.code.as_str(),
                    "authority-busy" | "storage-unavailable"
                ) =>
            {
                drop(guard);
                tokio::select! { biased;
                    _ = stop.cancelled() => return Err(Error::forbidden()),
                    _ = tokio::time::sleep_until(until) => return Err(error),
                    _ = tokio::time::sleep(Duration::from_millis(20)) => {},
                }
            }
            Err(error) => return Err(error),
        }
    }
}

/// The mutation has returned and its immutable result has been captured. Its
/// historical recording grants no further IO and must let admitted callers
/// drain their readers through the coordinator before acquiring a writer.
pub(super) async fn record<T>(
    roles: MutexGuard<'_, ()>,
    captured: impl Future<Output = Result<T>>,
) -> Result<T> {
    drop(roles);
    captured.await
}

#[cfg(test)]
mod tests {
    use super::*;
    use connect_store::{AuthorityMode, AuthorityScope, AuthorityWait, Store};
    use std::sync::Arc;

    #[tokio::test]
    async fn captured_recording_drains_an_admitted_reader_waiting_for_the_coordinator() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(
            Store::initialize(&dir.path().join("state.sqlite3"), Default::default()).unwrap(),
        );
        let roles = Arc::new(Mutex::new(()));
        let held = roles.lock().await;
        let reader = store
            .acquire_authority(
                &[AuthorityScope::identity("member", AuthorityMode::Read).unwrap()],
                &AuthorityWait::new(Duration::from_secs(1)),
            )
            .unwrap();
        let queued = roles.clone();
        let (started, ready) = tokio::sync::oneshot::channel();
        let caller = tokio::spawn(async move {
            started.send(()).unwrap();
            let _role = queued.lock().await;
            drop(reader);
        });
        ready.await.unwrap();
        // Queue the reader's coordinator request before recording needs the
        // conflicting kernel writer. Retaining `held` through this await makes
        // the real Store authority wait fail; increasing a timeout cannot help.
        tokio::task::yield_now().await;
        let recorded = record(held, async move {
            tokio::task::spawn_blocking(move || {
                store.acquire_authority(
                    &[AuthorityScope::identity("member", AuthorityMode::Write).unwrap()],
                    &AuthorityWait::new(Duration::from_millis(250)),
                )
            })
            .await
            .unwrap()
        })
        .await
        .unwrap();
        drop(recorded);
        caller.await.unwrap();
    }

    #[tokio::test]
    async fn busy_metadata_releases_the_coordinator_before_the_next_read() {
        let roles = Mutex::new(());
        let stop = CancellationToken::new();
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let work = read(
            &roles,
            &stop,
            tokio::time::Instant::now() + Duration::from_secs(1),
            || async {
                assert!(roles.try_lock().is_err());
                if calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                    Err(busy())
                } else {
                    Ok(17)
                }
            },
        );
        let observer = async {
            tokio::task::yield_now().await;
            let _available = tokio::time::timeout(Duration::from_millis(10), roles.lock())
                .await
                .unwrap();
            assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        };
        let (result, ()) = tokio::join!(work, observer);
        let (guard, value) = result.unwrap();
        assert_eq!(value, 17);
        drop(guard);
    }

    #[tokio::test]
    async fn refusal_cancellation_and_original_cutoff_never_lend_a_projection() {
        let roles = Mutex::new(());
        let stop = CancellationToken::new();
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let result = read(
            &roles,
            &stop,
            tokio::time::Instant::now() + Duration::from_secs(1),
            || async {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Err::<(), _>(Error::forbidden())
            },
        )
        .await;
        assert_eq!(result.err().unwrap().code, "forbidden");
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(roles.try_lock().is_ok());
        stop.cancel();
        let result = read(
            &roles,
            &stop,
            tokio::time::Instant::now() + Duration::from_secs(1),
            || async {
                panic!("cancelled projection read was entered");
                #[allow(unreachable_code)]
                Ok(())
            },
        )
        .await;
        assert_eq!(result.err().unwrap().code, "forbidden");
        let result = read(
            &roles,
            &CancellationToken::new(),
            tokio::time::Instant::now(),
            || async {
                panic!("expired projection read was entered");
                #[allow(unreachable_code)]
                Ok(())
            },
        )
        .await;
        assert_eq!(result.err().unwrap().code, "authority-busy");
    }
}
