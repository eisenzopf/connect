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
    let polls_joined = runtime
        .drain_control_polls(tokio::time::Instant::now() + Duration::from_secs(2))
        .await;
    eprintln!("native control scheduler probe: query_entered={saw_query} timer_while_sql_held={timer_observed} original_pending={original_pending} retained_owner={retained_owner} stop_preserved_owner={stop_preserved_owner} joined={joined}");
    assert!(
        saw_query && matches!(result, Ok(true)),
        "actual original Store read succeeded"
    );
    assert!(
        normal && joined == 1 && retained_owner && stop_preserved_owner && polls_joined,
        "same original worker remained owned through stop and joined normally"
    );
    assert!(
        progressed_while_held && timer_observed && original_pending,
        "native control SQL wait occupied the async executor"
    );
}

fn fixture() -> (tempfile::TempDir, Arc<connect_store::Store>, Arc<Runtime>) {
    let directory = tempfile::tempdir().expect("isolated control fixture");
    let store = Arc::new(
        connect_store::Store::initialize(
            &directory.path().join("state.sqlite3"),
            Default::default(),
        )
        .expect("store"),
    );
    let runtime = Runtime::new(store.clone()).expect("production runtime");
    (directory, store, runtime)
}
async fn join_controls(runtime: &Runtime) -> bool {
    let mut work = std::mem::take(&mut *runtime.work.lock().expect("owned work"));
    let mut normal = true;
    while let Some(result) = work.join_next().await {
        normal &= result.is_ok();
    }
    normal
}

#[test]
fn native_control_preserves_wakeups_deadline_and_nested_blocking_progress() {
    // A whole-future block_on in the only blocking thread would strand the
    // nested job. Each synchronous poll must release its thread on Pending.
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .expect("one blocking worker");
    executor.block_on(async {
        let (_directory, _store, runtime) = fixture();
        let (sent, result) = tokio::sync::oneshot::channel();
        let untouched = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let released = Arc::new(AtomicBool::new(false));
        let wake = Arc::new(Mutex::new(None::<Waker>));
        let (parked, parked_rx) = tokio::sync::oneshot::channel();
        let calls = untouched.clone();
        let release = released.clone();
        let original_wake = wake.clone();
        runtime
            .spawn_native_control(async move {
                let mut parked = Some(parked);
                std::future::poll_fn(|cx| {
                    calls.fetch_add(1, Ordering::AcqRel);
                    if release.load(Ordering::Acquire) {
                        return Poll::Ready(());
                    }
                    *original_wake.lock().expect("original waker") = Some(cx.waker().clone());
                    if let Some(parked) = parked.take() {
                        let _ = parked.send(());
                    }
                    Poll::Pending
                })
                .await;
                let mut polls = 0;
                std::future::poll_fn(|cx| {
                    polls += 1;
                    if polls <= 16 {
                        // Both wakes happen before this poll returns.
                        cx.waker().wake_by_ref();
                        cx.waker().wake_by_ref();
                        Poll::Pending
                    } else {
                        Poll::Ready(())
                    }
                })
                .await;
                let nested = tokio::task::spawn_blocking(|| 17u32).await;
                let deadline = tokio::time::Instant::now() + Duration::from_millis(30);
                let expired = tokio::time::timeout_at(deadline, std::future::pending::<()>())
                    .await
                    .is_err();
                let _ = sent.send((
                    polls,
                    nested,
                    expired,
                    tokio::time::Instant::now() >= deadline,
                ));
            })
            .expect("original worker");
        parked_rx.await.expect("original future parked");
        tokio::time::sleep(Duration::from_millis(20)).await;
        let polls_without_wake = untouched.load(Ordering::Acquire);
        released.store(true, Ordering::Release);
        wake.lock()
            .expect("original waker")
            .take()
            .expect("registered wake")
            .wake();
        let result = tokio::time::timeout(Duration::from_secs(2), result).await;
        runtime.stop.cancel();
        let parents = join_controls(&runtime).await;
        let children = runtime
            .drain_control_polls(tokio::time::Instant::now() + Duration::from_secs(2))
            .await;
        assert!(parents && children && !runtime.retirement_failed.load(Ordering::Acquire));
        let (polls, nested, expired, original_deadline) = result
            .expect("no pool starvation")
            .expect("original result");
        assert_eq!(polls, 17);
        assert_eq!(
            polls_without_wake, 1,
            "no polling without the original future's wake"
        );
        assert_eq!(nested.expect("nested blocking job joined"), 17);
        assert!(
            expired && original_deadline,
            "original timeout remains live"
        );
    });
}

#[tokio::test]
async fn cancelled_native_control_retains_running_sql_poll_through_drain() {
    let (_directory, store, runtime) = fixture();
    let identity = store
        .create_identity("cancelled-sql-owner")
        .expect("identity");
    let credential = store
        .issue(
            &identity.id,
            connect_store::Scope::operator(),
            chrono::Utc::now().timestamp() + 60,
        )
        .expect("credential");
    let (locked, ready) = tokio::sync::oneshot::channel();
    let entered = Arc::new(AtomicBool::new(false));
    let completed = Arc::new(AtomicBool::new(false));
    let holder_store = store.clone();
    let holder_entered = entered.clone();
    let holder = std::thread::spawn(move || {
        holder_store
            .with_primary_sql_held_for_test(|| {
                let _ = locked.send(());
                let cutoff = Instant::now() + Duration::from_secs(2);
                while !holder_entered.load(Ordering::Acquire) && Instant::now() < cutoff {
                    std::thread::sleep(Duration::from_millis(1));
                }
                let original_started = holder_entered.load(Ordering::Acquire);
                std::thread::sleep(Duration::from_millis(300));
                original_started
            })
            .expect("original SQL holder")
    });
    ready.await.expect("SQL held");
    let (sent, started) = tokio::sync::oneshot::channel();
    let finished = completed.clone();
    runtime
        .spawn_native_control(async move {
            entered.store(true, Ordering::Release);
            let _ = sent.send(());
            let observed = store
                .credential_metadata(&credential.id)
                .expect("original credential read");
            assert_eq!(observed.actor.identity_id, identity.id);
            finished.store(true, Ordering::Release);
        })
        .expect("original control");
    started.await.expect("real query entered");
    assert_eq!(runtime.control_polls.len(), Some(1));
    let extra = Arc::new(AtomicBool::new(false));
    let marked = extra.clone();
    assert!(
        runtime
            .control_polls
            .submit(1, move || {
                marked.store(true, Ordering::Release);
                Poll::Ready(())
            })
            .is_err(),
        "running original consumes capacity"
    );
    runtime.stop.cancel();
    runtime.work.lock().expect("cancel parent only").abort_all();
    assert!(
        !join_controls(&runtime).await,
        "parent cancellation is observed"
    );
    assert!(!completed.load(Ordering::Acquire));
    assert_eq!(
        runtime.control_polls.len(),
        Some(1),
        "running SQL still owned after parent joins"
    );
    let drain = runtime.drain_control_polls(tokio::time::Instant::now() + Duration::from_secs(2));
    tokio::pin!(drain);
    let premature = tokio::select! {
        result = &mut drain => Some(result),
        _ = tokio::time::sleep(Duration::from_millis(10)) => None,
    };
    let joined = match premature {
        Some(result) => result,
        None => drain.await,
    };
    let original_started = holder.join().expect("SQL holder joined");
    assert!(
        premature.is_none(),
        "drain cannot finish before original SQL returns"
    );
    assert!(joined && original_started && completed.load(Ordering::Acquire));
    assert_eq!(runtime.control_polls.len(), Some(0));
    assert!(
        !extra.load(Ordering::Acquire),
        "refused capacity has no execution"
    );
    assert!(
        runtime.control_polls.submit(1, || Poll::Ready(())).is_err(),
        "drained registry stays closed"
    );
    assert!(
        runtime.spawn_native_control(async {}).is_err(),
        "stopped runtime cannot dispatch"
    );
}

#[tokio::test]
async fn native_control_poll_panic_remains_a_failed_drain() {
    let (_directory, _store, runtime) = fixture();
    runtime
        .spawn_native_control(async { panic!("synthetic original control poll failure") })
        .expect("original worker");
    assert!(
        join_controls(&runtime).await,
        "outer custodian records the child failure"
    );
    runtime.stop.cancel();
    assert!(runtime.retirement_failed.load(Ordering::Acquire));
    assert!(
        !runtime
            .drain_control_polls(tokio::time::Instant::now() + Duration::from_secs(2))
            .await,
        "panic cannot become a clean shutdown"
    );
    assert_eq!(runtime.control_polls.len(), Some(0));
}
