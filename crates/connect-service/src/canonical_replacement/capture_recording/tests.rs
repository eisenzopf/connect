//! Production recorder and retained blocking-call custody; no synthetic movement grant.
use super::*;
use connect_store::{AuthorityMode, AuthorityScope, Store};
use std::sync::{atomic::AtomicUsize, mpsc};
use tokio::sync::oneshot;

struct Release(Option<mpsc::Sender<()>>);
impl Release {
    fn release(mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}
impl Drop for Release {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}
fn store() -> (tempfile::TempDir, Arc<Store>) {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(
        Store::initialize(&directory.path().join("state.sqlite3"), Default::default()).unwrap(),
    );
    (directory, store)
}
fn scope(mode: AuthorityMode) -> AuthorityScope {
    AuthorityScope::identity("known-capture-recorder", mode).unwrap()
}

#[tokio::test]
async fn same_capture_serializes_store_calls_and_followers_still_recheck() {
    let (_directory, store) = store();
    let recorder = Arc::new(Recorder::default());
    let calls = Arc::new(AtomicUsize::new(0));
    let (entered, ready) = oneshot::channel();
    let (release, blocked) = mpsc::channel();
    let release = Release(Some(release));
    let first_recorder = recorder.clone();
    let first_store = store.clone();
    let first_calls = calls.clone();
    let first = tokio::spawn(async move {
        let mut entered = Some(entered);
        let mut blocked = Some(blocked);
        first_recorder
            .record(&CancellationToken::new(), || {
                let store = first_store.clone();
                let calls = first_calls.clone();
                let entered = entered.take().unwrap();
                let blocked = blocked.take().unwrap();
                move || {
                    calls.fetch_add(1, Ordering::Relaxed);
                    let _read = store.try_acquire_authority(&[scope(AuthorityMode::Read)])?;
                    let _ = entered.send(());
                    blocked
                        .recv_timeout(Duration::from_secs(2))
                        .map_err(|_| Error::storage())?;
                    Ok(())
                }
            })
            .await
    });
    ready.await.unwrap();
    let follower_recorder = recorder.clone();
    let follower_store = store.clone();
    let follower_calls = calls.clone();
    let mut follower = tokio::spawn(async move {
        follower_recorder
            .record(&CancellationToken::new(), || {
                let store = follower_store.clone();
                let calls = follower_calls.clone();
                move || {
                    calls.fetch_add(1, Ordering::Relaxed);
                    let _writer = store.try_acquire_authority(&[scope(AuthorityMode::Write)])?;
                    Ok(())
                }
            })
            .await
    });
    let early = tokio::select! { biased;
        result = &mut follower => Some(result),
        _ = tokio::time::sleep(Duration::from_millis(10)) => None,
    };
    let calls_while_held = calls.load(Ordering::Relaxed);
    release.release();
    let first_result = first.await.unwrap();
    let follower_result = match early {
        Some(result) => result.unwrap(),
        None => follower.await.unwrap(),
    };
    // A previous successful record must never turn a later refusal into success.
    let refused = recorder
        .record(&CancellationToken::new(), || {
            let calls = calls.clone();
            move || {
                calls.fetch_add(1, Ordering::Relaxed);
                Err::<(), _>(Error::forbidden())
            }
        })
        .await;
    assert!(first_result.is_ok() && follower_result.is_ok());
    assert_eq!(
        calls_while_held, 1,
        "a follower entered Store while the original recorder ran"
    );
    assert_eq!(
        calls.load(Ordering::Relaxed),
        3,
        "a recorded result was reused as authority"
    );
    assert_eq!(refused.unwrap_err().code, "forbidden");
}

#[tokio::test]
async fn cancelled_async_owner_keeps_running_store_custody_until_return_or_panic() {
    for panic_in_store in [false, true] {
        let (_directory, store) = store();
        let recorder = Arc::new(Recorder::default());
        let owner = recorder
            .acquire(
                &CancellationToken::new(),
                tokio::time::Instant::now() + Duration::from_secs(1),
            )
            .await
            .unwrap();
        let running = owner.clone();
        let async_owner = tokio::spawn(async move {
            let _owner = owner;
            std::future::pending::<()>().await;
        });
        let (entered, ready) = oneshot::channel();
        let (release, blocked) = mpsc::channel();
        let release = Release(Some(release));
        let original_store = store.clone();
        // This is the same call wrapper used by both retrying recorders and
        // stop_unactivated. The test keeps the actual blocking join separately
        // so cancelling the outer owner cannot hide its exit or its panic.
        let actual_call = tokio::spawn(store_call(running.call(move || {
            let _read = original_store.try_acquire_authority(&[scope(AuthorityMode::Read)])?;
            let _ = entered.send(());
            blocked
                .recv_timeout(Duration::from_secs(2))
                .map_err(|_| Error::storage())?;
            assert!(!panic_in_store, "bounded recorder panic control");
            Ok(())
        })));
        ready.await.unwrap();
        async_owner.abort();
        let cancelled = async_owner.await.unwrap_err().is_cancelled();
        let cleanup_still_refused = recorder.try_acquire().is_none();
        release.release();
        let original_result = actual_call.await.unwrap();
        let follower = recorder
            .record(&CancellationToken::new(), || {
                let store = store.clone();
                move || {
                    let _writer = store.try_acquire_authority(&[scope(AuthorityMode::Write)])?;
                    Ok(())
                }
            })
            .await;
        assert!(cancelled && cleanup_still_refused);
        assert_eq!(original_result.is_err(), panic_in_store);
        assert!(
            follower.is_ok(),
            "cancel/panic stranded the exact capture recorder"
        );
    }
}

#[tokio::test]
async fn retry_owner_survives_sleep_without_retaining_store_authority() {
    let (_directory, store) = store();
    let recorder = Arc::new(Recorder::default());
    let calls = Arc::new(AtomicUsize::new(0));
    let (first, first_done) = oneshot::channel();
    let (release, blocked) = mpsc::channel();
    let release = Release(Some(release));
    let original_recorder = recorder.clone();
    let original_store = store.clone();
    let original_calls = calls.clone();
    let task = tokio::spawn(async move {
        let mut first = Some(first);
        let mut blocked = Some(blocked);
        original_recorder
            .record(&CancellationToken::new(), || {
                let store = original_store.clone();
                let calls = original_calls.clone();
                let first = first.take();
                let blocked = if first.is_none() {
                    blocked.take()
                } else {
                    None
                };
                move || {
                    let attempt = calls.fetch_add(1, Ordering::Relaxed);
                    if let Some(first) = first {
                        let read = store.try_acquire_authority(&[scope(AuthorityMode::Read)])?;
                        drop(read);
                        let _ = first.send(());
                        return Err(Error::new("authority-busy", "bounded first attempt"));
                    }
                    assert_eq!(attempt, 1);
                    blocked
                        .unwrap()
                        .recv_timeout(Duration::from_secs(2))
                        .map_err(|_| Error::storage())?;
                    let _read = store.try_acquire_authority(&[scope(AuthorityMode::Read)])?;
                    Ok(())
                }
            })
            .await
    });
    first_done.await.unwrap();
    let original_owner_retained = recorder.try_acquire().is_none();
    let writer = store.try_acquire_authority(&[scope(AuthorityMode::Write)]);
    let store_guard_released = writer.is_ok();
    drop(writer);
    release.release();
    let result = task.await.unwrap();
    assert!(result.is_ok() && original_owner_retained && store_guard_released);
    assert_eq!(calls.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn waiting_for_recorder_consumes_original_deadline_without_a_new_attempt() {
    let recorder = Arc::new(Recorder::default());
    let owner = recorder.try_acquire().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let until = tokio::time::Instant::now() + Duration::from_millis(30);
    let follower_recorder = recorder.clone();
    let follower_calls = calls.clone();
    let follower = tokio::spawn(async move {
        follower_recorder
            .record_until(&CancellationToken::new(), until, || {
                let calls = follower_calls.clone();
                move || {
                    calls.fetch_add(1, Ordering::Relaxed);
                    Ok(())
                }
            })
            .await
    });
    // Releasing after the absolute cut must not start a fresh five seconds.
    tokio::time::sleep_until(until + Duration::from_millis(5)).await;
    drop(owner);
    let result = follower.await.unwrap();
    assert_eq!(result.unwrap_err().code, "authority-busy");
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn stopped_waiter_does_not_block_a_different_capture_or_replace_original_owner() {
    let recorder = Arc::new(Recorder::default());
    let owner = recorder.try_acquire().unwrap();
    let stop = CancellationToken::new();
    let waiter_stop = stop.clone();
    let waiting = recorder.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let waiting_calls = calls.clone();
    let waiter = tokio::spawn(async move {
        waiting
            .record(&waiter_stop, || {
                let calls = waiting_calls.clone();
                move || {
                    calls.fetch_add(1, Ordering::Relaxed);
                    Ok(())
                }
            })
            .await
    });
    let separate = Arc::new(Recorder::default());
    let unrelated = separate
        .record(&CancellationToken::new(), || || Ok(()))
        .await;
    stop.cancel();
    let result = waiter.await.unwrap();
    let original_still_held = recorder.try_acquire().is_none();
    drop(owner);
    assert!(unrelated.is_ok() && original_still_held);
    assert_eq!(result.unwrap_err().code, "forbidden");
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn recorder_release_wakes_all_original_followers_without_lost_notifications() {
    let recorder = Arc::new(Recorder::default());
    let owner = recorder.try_acquire().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut followers = Vec::new();
    for _ in 0..16 {
        let recorder = recorder.clone();
        let calls = calls.clone();
        followers.push(tokio::spawn(async move {
            recorder
                .record(&CancellationToken::new(), || {
                    let calls = calls.clone();
                    move || {
                        calls.fetch_add(1, Ordering::Relaxed);
                        Ok(())
                    }
                })
                .await
        }));
    }
    tokio::task::yield_now().await;
    drop(owner);
    let mut results = Vec::new();
    for follower in followers {
        results.push(follower.await.unwrap());
    }
    assert!(results.into_iter().all(|result| result.is_ok()));
    assert_eq!(calls.load(Ordering::Relaxed), 16);
}

#[tokio::test]
async fn cancelled_cleanup_waits_for_retry_owner_then_attempts_original_settlement() {
    let (_directory, store) = store();
    let recorder = Arc::new(Recorder::default());
    let owner = recorder
        .acquire(
            &CancellationToken::new(),
            tokio::time::Instant::now() + Duration::from_secs(1),
        )
        .await
        .unwrap();
    let original_store = store.clone();
    let refused = store_call(owner.clone().call(move || {
        let read = original_store.try_acquire_authority(&[scope(AuthorityMode::Read)])?;
        drop(read);
        Err::<(), _>(Error::new("authority-busy", "bounded original retry"))
    }))
    .await;
    // Hold the exact production owner between its actual Store attempts. The
    // gate fixes the cancellation ordering without racing the20ms retry timer.
    let (retire, retired) = oneshot::channel();
    let retry_owner = tokio::spawn(async move {
        let _owner = owner;
        let _ = retired.await;
    });
    let stop = CancellationToken::new();
    stop.cancel();
    let calls = Arc::new(AtomicUsize::new(0));
    let premature_calls = calls.clone();
    let prematurely_finished = recorder
        .cleanup_attempt(&stop, move || {
            premature_calls.fetch_add(1, Ordering::Relaxed);
            Ok(())
        })
        .await;
    let before_release = calls.load(Ordering::Relaxed);
    let _ = retire.send(());
    retry_owner.await.unwrap();
    let final_calls = calls.clone();
    let after_release = recorder
        .cleanup_attempt(&stop, move || {
            final_calls.fetch_add(1, Ordering::Relaxed);
            let _writer = store.try_acquire_authority(&[scope(AuthorityMode::Write)])?;
            // Original cleanup leaves its retry loop after this real attempt when
            // stopping, even if the historical method still refuses settlement.
            Err(Error::forbidden())
        })
        .await;
    assert_eq!(refused.unwrap_err().code, "authority-busy");
    assert!(
        !prematurely_finished,
        "stop skipped the mandatory final Store attempt"
    );
    assert_eq!(
        before_release, 0,
        "cleanup overlapped the prior retry owner"
    );
    assert!(after_release);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}
