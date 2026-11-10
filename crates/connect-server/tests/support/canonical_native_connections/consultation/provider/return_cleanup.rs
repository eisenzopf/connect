//! Failure-only fixture cleanup. Neither errors nor panics replace the original
//! return failure; each original owner is awaited before the caller resumes it.
use futures_util::FutureExt;
use std::{future::Future, panic::AssertUnwindSafe};

pub(super) async fn observe<E>(
    owner: &'static str,
    work: impl Future<Output = Result<(), E>>,
) -> bool {
    let outcome = AssertUnwindSafe(work).catch_unwind().await;
    let status = match &outcome {
        Ok(Ok(())) => "ok",
        Ok(Err(_)) => "error",
        Err(_) => "panic",
    };
    eprintln!("provider return failure cleanup: owner={owner} result={status}");
    matches!(outcome, Ok(Ok(())))
}

#[tokio::test]
async fn failed_cleanup_does_not_prevent_following_original_task_join_or_replace_primary_panic() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    let joined = Arc::new(AtomicBool::new(false));
    let completed = joined.clone();
    let (release, waiting) = tokio::sync::oneshot::channel();
    let worker = tokio::spawn(async move {
        waiting.await.unwrap();
        completed.store(true, Ordering::Release);
    });
    let original = Box::new(String::from("original return deadline"));
    let failed = AssertUnwindSafe(async {
        let first = observe("refused", async { Err::<(), _>(()) }).await;
        let second = observe("panicked", async {
            panic!("secondary cleanup panic");
            #[allow(unreachable_code)]
            Ok::<(), ()>(())
        })
        .await;
        release.send(()).unwrap();
        let third = observe("original-worker", async { worker.await }).await;
        assert!(!first && !second && third);
        std::panic::resume_unwind(original)
    })
    .catch_unwind()
    .await
    .unwrap_err();
    assert!(joined.load(Ordering::Acquire));
    assert_eq!(
        *failed.downcast::<String>().unwrap(),
        "original return deadline"
    );
}
