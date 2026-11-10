//! Same original codec receipt, actual file admission and no renewed rights.
use super::*;
use connect_store::{AuthorityMode, AuthorityScope};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
fn fixture() -> (tempfile::TempDir, Arc<Store>) {
    let dir = tempfile::tempdir().unwrap();
    let db =
        Arc::new(Store::initialize(&dir.path().join("state.sqlite3"), Default::default()).unwrap());
    (dir, db)
}
fn scope(mode: AuthorityMode) -> AuthorityScope {
    AuthorityScope::conversation("codec-original-read", mode).unwrap()
}
#[tokio::test]
async fn codec_current_retry_keeps_same_reserved_receipt_until_actual_cid_writer_releases() {
    let (_dir, store) = fixture();
    let original = Arc::new(());
    let slot = Arc::new(Slot::default());
    slot.claim().unwrap().run(|| Ok(original.clone())).unwrap();
    let writer = store
        .try_acquire_authority(&[scope(AuthorityMode::Write)])
        .unwrap();
    let (entered, observed) = tokio::sync::oneshot::channel();
    let mut entered = Some(entered);
    let owner = tokio::spawn(async move {
        observed.await.unwrap();
        drop(writer);
    });
    let calls = AtomicUsize::new(0);
    let stop = CancellationToken::new();
    let read = retry_original_codec_current(
        &stop,
        tokio::time::Instant::now() + Duration::from_secs(1),
        || true,
        || {
            calls.fetch_add(1, Ordering::AcqRel);
            slot.with_ready(|receipt| {
                assert!(Arc::ptr_eq(receipt, &original));
                let result = store
                    .try_acquire_authority(&[scope(AuthorityMode::Read)])
                    .map_err(current_codec_error);
                if let Some(entered) = entered.take() {
                    let _ = entered.send(());
                }
                result
            })
        },
    )
    .await;
    owner.await.unwrap();
    let read = read.expect("original current read progresses after real writer");
    assert!(calls.load(Ordering::Acquire) >= 2);
    assert!(slot.claim().is_err(), "no second reservation allowed");
    assert!(Arc::ptr_eq(
        &slot.take_when_done().await.unwrap(),
        &original
    ));
    assert!(slot.take_when_done().await.is_none());
    drop(read);
}
#[tokio::test]
async fn codec_current_retry_refuses_revoked_original_before_reacquiring_after_contention() {
    let (_dir, store) = fixture();
    let original = Arc::new(());
    let slot = Arc::new(Slot::default());
    slot.claim().unwrap().run(|| Ok(original.clone())).unwrap();
    let writer = store
        .try_acquire_authority(&[scope(AuthorityMode::Write)])
        .unwrap();
    let current = Arc::new(AtomicBool::new(true));
    let changed = current.clone();
    let (entered, observed) = tokio::sync::oneshot::channel();
    let mut entered = Some(entered);
    let owner = tokio::spawn(async move {
        observed.await.unwrap();
        changed.store(false, Ordering::Release);
        drop(writer);
    });
    let calls = AtomicUsize::new(0);
    let stop = CancellationToken::new();
    let result = retry_original_codec_current(
        &stop,
        tokio::time::Instant::now() + Duration::from_secs(1),
        || current.load(Ordering::Acquire),
        || {
            calls.fetch_add(1, Ordering::AcqRel);
            let result = slot.with_ready(|receipt| {
                assert!(Arc::ptr_eq(receipt, &original));
                store
                    .try_acquire_authority(&[scope(AuthorityMode::Read)])
                    .map_err(current_codec_error)
            });
            if let Some(entered) = entered.take() {
                let _ = entered.send(());
            }
            result
        },
    )
    .await;
    owner.await.unwrap();
    assert!(matches!(result, Err(DispatchError::Closed)));
    assert_eq!(calls.load(Ordering::Acquire), 1);
    assert!(Arc::ptr_eq(
        &slot.take_when_done().await.unwrap(),
        &original
    ));
    assert!(slot.claim().is_err());
}
