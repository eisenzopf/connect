//! Actual Runtime task ownership/drain and production sequencing helper only.
//! The paired Store control supplies real uncertain-Start/history semantics;
//! these closures do not manufacture an integrated B/Closed/restore receipt.
use super::*;
use std::sync::atomic::AtomicUsize;

#[tokio::test]
async fn uncertain_start_settlement_precedes_public_abort_and_original_owned_task_drains() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(
        connect_store::Store::initialize(
            &directory.path().join("state.sqlite3"),
            Default::default(),
        )
        .unwrap(),
    );
    let service = crate::Service::new(store.clone());
    let runtime = Runtime::new(store).unwrap();
    assert!(service.canonical_runtime.set(runtime.clone()).is_ok());
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let public_calls = Arc::new(AtomicUsize::new(0));
    let settled = Arc::new(AtomicBool::new(false));
    let (sent, received) = tokio::sync::oneshot::channel();
    let original_entered = entered.clone();
    let original_release = release.clone();
    let calls = public_calls.clone();
    let original_settled = settled.clone();
    runtime.work.lock().unwrap().spawn(async move {
        let result = before_public_abort(
            async {
                original_entered.notify_one();
                original_release.notified().await;
                original_settled.store(true, Ordering::Release);
                Ok(())
            },
            async {
                assert!(original_settled.load(Ordering::Acquire));
                calls.fetch_add(1, Ordering::AcqRel);
                Ok(())
            },
        )
        .await;
        let _ = sent.send(result);
    });
    let entered = tokio::time::timeout(Duration::from_secs(1), entered.notified()).await;
    let pending_calls = public_calls.load(Ordering::Acquire);
    let pending_owners = runtime.work.lock().unwrap().len();
    release.notify_one();
    let result = tokio::time::timeout(Duration::from_secs(1), received).await;
    // The real production drain consumes and joins this SAME original JoinSet,
    // not an inferred count or a detached task. Release precedes every assertion.
    let drain = service.drain_canonical_runtime().await;
    assert!(
        entered.is_ok(),
        "original settlement did not enter before cleanup"
    );
    assert_eq!(pending_calls, 0);
    assert_eq!(pending_owners, 1);
    result.unwrap().unwrap().unwrap();
    assert!(drain.is_ok(), "original task did not drain: {drain:?}");
    assert!(runtime.work.lock().unwrap().is_empty());
    assert_eq!(public_calls.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn uncertain_start_refusal_never_polls_public_abort_or_relabels_its_error() {
    // Shared production precondition with real local mutexes. A known Started
    // must skip even a simultaneously unavailable stale failed capture.
    let capture = Arc::new(());
    let started = Mutex::new(Some(()));
    let failed = Mutex::new(Some(capture.clone()));
    {
        let _held_failed = failed.lock().unwrap();
        assert!(uncertain_start_only(&started, &failed).unwrap().is_none());
    }
    *started.lock().unwrap() = None;
    assert!(Arc::ptr_eq(
        &uncertain_start_only(&started, &failed).unwrap().unwrap(),
        &capture
    ));
    *failed.lock().unwrap() = None;
    assert!(uncertain_start_only(&started, &failed).unwrap().is_none());
    for code in [
        "reconciliation-required",
        "forbidden",
        "storage-unavailable",
    ] {
        let public_calls = AtomicUsize::new(0);
        let result = before_public_abort(
            async { Err(Error::new(code, "controlled terminal settlement refusal")) },
            async {
                public_calls.fetch_add(1, Ordering::AcqRel);
                Ok(())
            },
        )
        .await;
        assert_eq!(result.unwrap_err().code, code);
        assert_eq!(public_calls.load(Ordering::Acquire), 0);
    }
    let public_calls = AtomicUsize::new(0);
    let result = before_public_abort(async { Ok(()) }, async {
        public_calls.fetch_add(1, Ordering::AcqRel);
        Err(Error::new(
            "cleanup-incomplete",
            "actual close remains separately required",
        ))
    })
    .await;
    assert_eq!(result.unwrap_err().code, "cleanup-incomplete");
    assert_eq!(public_calls.load(Ordering::Acquire), 1);
}
