//! Original native control ownership, bounded by the existing runtime capacity.
use super::*;
use std::future::Future;

impl Runtime {
    pub(crate) fn spawn_native_control(
        &self,
        worker: impl Future<Output = ()> + Send + 'static,
    ) -> Result<()> {
        let mut work = self.work.lock().map_err(|_| Error::storage())?;
        while work.try_join_next().is_some() {}
        if self.stop.is_cancelled()
            || work.len() >= self.media.store.limits().connections.saturating_mul(2)
        {
            return Err(Error::new("busy", "Native control workers are at capacity"));
        }
        work.spawn(worker);
        Ok(())
    }
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    // A single async worker makes executor exclusion observable without load
    // tuning. The production launcher and real Store mutex/query are used;
    // this is scheduler/custody coverage, not a signed or decoded-media pass.
    #[tokio::test]
    async fn native_control_sql_wait_preserves_executor_progress_and_original_join() {
        let directory = tempfile::tempdir().expect("isolated fixture");
        let store = Arc::new(
            connect_store::Store::initialize(
                &directory.path().join("state.sqlite3"),
                Default::default(),
            )
            .expect("store"),
        );
        let identity = store
            .create_identity("control-scheduler")
            .expect("identity");
        let credential = store
            .issue(
                &identity.id,
                connect_store::Scope::operator(),
                chrono::Utc::now().timestamp() + 60,
            )
            .expect("credential");
        let runtime = Runtime::new(store.clone()).expect("production runtime");
        let entered = Arc::new(AtomicBool::new(false));
        let heartbeat = Arc::new(AtomicBool::new(false));
        let released = Arc::new(AtomicBool::new(false));
        let completed = Arc::new(AtomicBool::new(false));
        let (locked_tx, locked_rx) = tokio::sync::oneshot::channel();
        let held_store = store.clone();
        let held_entered = entered.clone();
        let held_heartbeat = heartbeat.clone();
        let held_released = released.clone();
        // This independent owner always releases SQL, even if the async
        // executor cannot poll its timer or observe cancellation.
        let holder = std::thread::spawn(move || {
            held_store
                .with_primary_sql_held_for_test(|| {
                    let _ = locked_tx.send(());
                    let until = Instant::now() + Duration::from_secs(2);
                    while !held_entered.load(Ordering::Acquire) && Instant::now() < until {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    let saw_query = held_entered.load(Ordering::Acquire);
                    let until = Instant::now() + Duration::from_millis(300);
                    while Instant::now() < until {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    let progressed = held_heartbeat.load(Ordering::Acquire);
                    held_released.store(true, Ordering::Release);
                    (saw_query, progressed)
                })
                .expect("original SQL owner")
        });
        locked_rx.await.expect("SQL held before dispatch");
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        let marked = completed.clone();
        runtime
            .spawn_native_control(async move {
                entered.store(true, Ordering::Release);
                let _ = started_tx.send(());
                let result = store
                    .credential_metadata(&credential.id)
                    .map(|metadata| metadata.actor.identity_id == identity.id);
                marked.store(true, Ordering::Release);
                let _ = result_tx.send(result);
            })
            .expect("one original control owner");
        started_rx.await.expect("original metadata query entered");
        tokio::time::sleep(Duration::from_millis(10)).await;
        let progressed_while_held = !released.load(Ordering::Acquire);
        let original_pending = !completed.load(Ordering::Acquire);
        let retained_owner = runtime.work.lock().expect("owned work").len() == 1;
        heartbeat.store(progressed_while_held, Ordering::Release);
        runtime.stop.cancel();
        let stop_preserved_owner = runtime.work.lock().expect("stopped owned work").len() == 1;
        let result = result_rx.await.expect("original read result");
        let (saw_query, timer_observed) = holder.join().expect("SQL owner joined");
        let mut work = std::mem::take(&mut *runtime.work.lock().expect("join original work"));
        let mut joined = 0;
        let mut normal = true;
        while let Some(result) = work.join_next().await {
            joined += 1;
            normal &= result.is_ok();
        }
        eprintln!("native control scheduler probe: query_entered={saw_query} timer_while_sql_held={timer_observed} original_pending={original_pending} retained_owner={retained_owner} stop_preserved_owner={stop_preserved_owner} joined={joined}");
        assert!(
            saw_query && matches!(result, Ok(true)),
            "actual original Store read succeeded"
        );
        assert!(
            normal && joined == 1 && retained_owner && stop_preserved_owner,
            "same original worker remained owned through stop and joined normally"
        );
        assert!(
            progressed_while_held && timer_observed && original_pending,
            "native control SQL wait occupied the async executor"
        );
    }
}
