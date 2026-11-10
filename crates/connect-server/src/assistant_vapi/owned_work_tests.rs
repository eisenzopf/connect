//! Production owned-work boundary: pointer-sized spawn argument and same task.
use super::*;
use std::sync::atomic::AtomicUsize;

#[tokio::test]
async fn owned_context_work_boxes_large_pending_future_before_spawn_and_joins_original() {
    let bytes = [0x5au8; 64 * 1024];
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let (started, began) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let work = async move {
        started.send(()).map_err(|_| Error::forbidden())?;
        released.await.map_err(|_| Error::forbidden())?;
        // Required after suspension, so the actual future retains this array.
        assert!(std::hint::black_box(&bytes)
            .iter()
            .all(|value| *value == 0x5a));
        assert_eq!(observed.fetch_add(1, Ordering::AcqRel), 0);
        Ok(())
    };
    let original_size = std::mem::size_of_val(&work);
    assert!(original_size >= 64 * 1024);
    let boxed = owned_context_work(work);
    assert!(std::mem::size_of_val(&boxed) <= 2 * std::mem::size_of::<usize>());
    let task = tokio::spawn(boxed);
    began.await.unwrap();
    assert!(!task.is_finished());
    release.send(()).unwrap();
    let result = task.await;
    assert!(result.is_ok(), "original task must be joined");
    result.unwrap().unwrap();
    assert_eq!(calls.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn owned_context_work_cancellation_keeps_original_cleanup_and_result() {
    struct Owned(Arc<AtomicUsize>);
    impl Drop for Owned {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::AcqRel);
        }
    }
    let drops = Arc::new(AtomicUsize::new(0));
    let owner = Owned(drops.clone());
    let stop = CancellationToken::new();
    let original = stop.clone();
    let (started, began) = oneshot::channel();
    let task = tokio::spawn(owned_context_work(async move {
        let _owner = owner;
        started.send(()).map_err(|_| Error::forbidden())?;
        original.cancelled().await;
        Err(problem_error("original-context-cancelled"))
    }));
    began.await.unwrap();
    assert_eq!(drops.load(Ordering::Acquire), 0);
    stop.cancel();
    let result = task.await;
    assert!(
        result.is_ok(),
        "cancellation must join, not abort/detach task"
    );
    assert_eq!(
        result.unwrap().unwrap_err().code,
        "original-context-cancelled"
    );
    assert_eq!(drops.load(Ordering::Acquire), 1);
}
