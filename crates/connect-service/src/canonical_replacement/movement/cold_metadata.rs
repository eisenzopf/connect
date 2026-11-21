//! Repeat only a pure planning turn, before one-shot staging or durable begin.
use connect_protocol::{Error, Result};
use std::{future::Future, pin::Pin, time::Duration};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

pub(super) type Planning<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

pub(super) async fn read<'a, T, F>(
    until: Instant,
    cancellation: [&CancellationToken; 2],
    mut current: F,
) -> Result<T>
where
    F: FnMut() -> Planning<'a, T>,
{
    loop {
        if cancellation.iter().any(|cancel| cancel.is_cancelled()) || Instant::now() >= until {
            return Err(Error::forbidden());
        }
        // The producer owns every read/role alias on its lexical stack. Join
        // it, and drop the completed future, before any retry pause. No mutation
        // or one-shot Core stage may be placed inside this planning callback.
        match current().await {
            Ok(value) => return Ok(value),
            Err(error) if error.code == "authority-busy" => {}
            Err(error) => return Err(error),
        }
        // Storage/SQLite errors, changed facts and currentness refusals never
        // enter this branch. The same original deadline and cancellation apply.
        tokio::select! { biased;
            _ = cancellation[0].cancelled() => return Err(Error::forbidden()),
            _ = cancellation[1].cancelled() => return Err(Error::forbidden()),
            _ = tokio::time::sleep_until(until) => return Err(Error::forbidden()),
            _ = tokio::time::sleep_until(until.min(Instant::now() + Duration::from_millis(2))) => {}
        }
    }
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::*;
    use connect_store::{AuthorityMode, AuthorityScope, Store};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    };

    struct SqlHolder {
        release: Option<std::sync::mpsc::Sender<()>>,
        thread: Option<std::thread::JoinHandle<()>>,
    }
    impl SqlHolder {
        async fn new(store: Arc<Store>) -> Self {
            let (release, receive) = std::sync::mpsc::channel();
            let (entered, ready) = tokio::sync::oneshot::channel();
            let thread = std::thread::spawn(move || {
                store
                    .with_primary_sql_held_for_test(|| {
                        entered.send(()).unwrap();
                        receive.recv_timeout(Duration::from_secs(2)).unwrap();
                    })
                    .unwrap();
            });
            ready.await.unwrap();
            Self {
                release: Some(release),
                thread: Some(thread),
            }
        }
        fn close(&mut self) {
            if let Some(release) = self.release.take() {
                let _ = release.send(());
            }
            if let Some(thread) = self.thread.take() {
                thread.join().unwrap();
            }
        }
    }
    impl Drop for SqlHolder {
        fn drop(&mut self) {
            self.close();
        }
    }

    #[tokio::test]
    async fn cold_planning_actual_sql_busy_drops_real_turn_then_preserves_same_metadata_once() {
        {
            let directory = tempfile::tempdir().unwrap();
            let store = Arc::new(
                Store::initialize(&directory.path().join("state.sqlite3"), Default::default())
                    .unwrap(),
            );
            let before = Store::inspect_snapshot(store.path()).unwrap();
            let roles = tokio::sync::Mutex::new(());
            let scope =
                AuthorityScope::identity("same-cold-planning-owner", AuthorityMode::Read).unwrap();
            let metadata = Arc::new(("same-operation", "same-attempt", "same-effect"));
            let original = Arc::as_ptr(&metadata);
            let weak_read = Mutex::new(None);
            let calls = AtomicUsize::new(0);
            let successes = AtomicUsize::new(0);
            let stop = CancellationToken::new();
            let cancel = CancellationToken::new();
            let until = Instant::now() + Duration::from_secs(1);
            let mut holder = SqlHolder::new(store.clone()).await;
            let mut pending = Box::pin(read(until, [&stop, &cancel], || {
                Box::pin(async {
                    let role = roles.lock().await;
                    let actual_read = store.try_acquire_authority(&[scope.clone()])?;
                    *weak_read.lock().unwrap() = Some(Arc::downgrade(&actual_read));
                    calls.fetch_add(1, Ordering::SeqCst);
                    // Actual Store SQL try-lock/SELECT, not a synthetic Busy.
                    store.try_primary_sql_read_for_test()?;
                    successes.fetch_add(1, Ordering::SeqCst);
                    Ok((role, actual_read, metadata.clone()))
                })
            }));
            tokio::select! { biased;
                result = &mut pending => panic!("SQL-held planning returned early: {}", result.is_ok()),
                _ = async { while calls.load(Ordering::SeqCst) == 0 { tokio::task::yield_now().await; } } => {}
            }
            assert_eq!(successes.load(Ordering::SeqCst), 0);
            assert!(roles.try_lock().is_ok(), "role must drop before Busy wait");
            assert_eq!(
                weak_read.lock().unwrap().as_ref().unwrap().strong_count(),
                0
            );
            let writer = store
                .try_acquire_authority(&[AuthorityScope::identity(
                    "same-cold-planning-owner",
                    AuthorityMode::Write,
                )
                .unwrap()])
                .unwrap();
            drop(writer);
            holder.close();
            let (role, actual_read, same) = pending.await.unwrap();
            assert!(calls.load(Ordering::SeqCst) >= 2);
            assert_eq!(Arc::as_ptr(&same), original, "retain the same metadata");
            drop(actual_read);
            drop(role);
            assert_eq!(successes.load(Ordering::SeqCst), 1);
            assert_eq!(Store::inspect_snapshot(store.path()).unwrap(), before);
        }
    }

    #[tokio::test]
    async fn cold_planning_sql_busy_original_cancel_and_cutoff_do_not_enter_next_stage() {
        for expire in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let store = Arc::new(
                Store::initialize(&directory.path().join("state.sqlite3"), Default::default())
                    .unwrap(),
            );
            let before = Store::inspect_snapshot(store.path()).unwrap();
            let mut holder = SqlHolder::new(store.clone()).await;
            let stop = CancellationToken::new();
            let cancel = CancellationToken::new();
            let calls = AtomicUsize::new(0);
            let until = Instant::now() + Duration::from_millis(if expire { 30 } else { 1_000 });
            let mut pending = Box::pin(read(until, [&stop, &cancel], || {
                Box::pin(async {
                    calls.fetch_add(1, Ordering::SeqCst);
                    store.try_primary_sql_read_for_test()
                })
            }));
            tokio::select! { biased;
                _ = &mut pending => panic!("actual SQL exclusion must defer"),
                _ = async { while calls.load(Ordering::SeqCst) == 0 { tokio::task::yield_now().await; } } => {}
            }
            if !expire {
                cancel.cancel();
            }
            let error = pending.await.unwrap_err();
            assert_eq!(error.code, "forbidden");
            if expire {
                assert!(Instant::now() >= until);
            }
            let finished = calls.load(Ordering::SeqCst);
            holder.close();
            assert_eq!(
                calls.load(Ordering::SeqCst),
                finished,
                "release never resurrects cancelled/expired work"
            );
            assert_eq!(Store::inspect_snapshot(store.path()).unwrap(), before);
        }
    }

    #[tokio::test]
    async fn cold_planning_sql_and_currentness_errors_are_terminal_without_replay() {
        for code in [
            "storage-unavailable",
            "forbidden",
            "reconciliation-required",
            "not-ready",
        ] {
            let calls = AtomicUsize::new(0);
            let stop = CancellationToken::new();
            let cancel = CancellationToken::new();
            let result = read::<(), _>(
                Instant::now() + Duration::from_secs(1),
                [&stop, &cancel],
                || {
                    Box::pin(async {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Err(Error::new(code, "unchanged terminal error"))
                    })
                },
            )
            .await;
            assert_eq!(result.unwrap_err().code, code);
            assert_eq!(calls.load(Ordering::SeqCst), 1);
        }
    }
}
