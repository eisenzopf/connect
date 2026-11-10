use super::*;
use crate::canonical_runtime::Runtime;
use connect_store::{AuthorityMode, AuthorityScope, AuthorityWait, Store};
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

fn fixture() -> (tempfile::TempDir, Arc<Store>, Arc<Runtime>) {
    let directory = tempfile::tempdir().expect("isolated codec admission fixture");
    let store = Arc::new(
        Store::initialize(&directory.path().join("state.sqlite3"), Default::default())
            .expect("store"),
    );
    let runtime = Runtime::new(store.clone()).expect("production runtime");
    (directory, store, runtime)
}
fn scope(mode: AuthorityMode) -> AuthorityScope {
    AuthorityScope::conversation("codec-admission-fixture", mode).expect("exact CID")
}
async fn drain(runtime: &Runtime) {
    runtime.stop.cancel();
    assert!(
        runtime
            .drain_control_polls(tokio::time::Instant::now() + Duration::from_secs(1))
            .await,
        "original blocking calls must join normally"
    );
}

#[tokio::test]
async fn original_codec_admission_yields_current_thread_for_real_reader_drain() {
    // Default tokio::test is current-thread: a synchronous five-ms writer wait
    // here would prevent the only task able to drop its real reader from polling.
    let (_directory, store, runtime) = fixture();
    let read = store
        .try_acquire_authority(&[scope(AuthorityMode::Read)])
        .expect("real reader");
    let (entered, observed) = tokio::sync::oneshot::channel();
    let reader = tokio::spawn(async move {
        observed.await.expect("actual admission entered");
        drop(read);
    });
    let slot = Arc::new(Slot::default());
    let pending = slot.claim().expect("one original claim");
    let db = store.clone();
    let outcome = runtime
        .owned_store_call(move || {
            pending.run(|| {
                entered.send(()).expect("original async reader still owned");
                db.acquire_authority(
                    &[scope(AuthorityMode::Write)],
                    &AuthorityWait::new(Duration::from_millis(5)),
                )
            })
        })
        .await;
    reader.await.expect("reader owner joined");
    assert!(
        outcome.is_ok(),
        "same five-ms writer must progress while async reader drains: {outcome:?}"
    );
    let original = slot.take_when_done().await.expect("same original writer");
    assert!(store.authority_covers(&original, &[scope(AuthorityMode::Write)]));
    assert!(
        slot.claim().is_err(),
        "original admission cannot be invoked twice"
    );
    assert!(
        slot.take_when_done().await.is_none(),
        "opaque outcome consumed once"
    );
    drop(original);
    drain(&runtime).await;
    assert!(
        store
            .try_acquire_authority(&[scope(AuthorityMode::Read)])
            .is_ok(),
        "writer released after original outcome consumption"
    );
}

#[tokio::test]
async fn cancelled_codec_waiter_retains_original_reservation_until_join_and_capture() {
    let (_directory, store, runtime) = fixture();
    let read = store
        .try_acquire_authority(&[scope(AuthorityMode::Read)])
        .expect("real reader");
    let slot = Arc::new(Slot::default());
    let pending = slot.claim().expect("original claim");
    let calls = Arc::new(AtomicUsize::new(0));
    let original_calls = calls.clone();
    let original_identity = Arc::new(());
    let identity = original_identity.clone();
    let (entered, observed) = tokio::sync::oneshot::channel();
    let (release, proceed) = std::sync::mpsc::channel();
    let db = store.clone();
    let host = runtime.clone();
    let waiter = tokio::spawn(async move {
        host.owned_store_call(move || {
            pending.run(|| {
                original_calls.fetch_add(1, Ordering::AcqRel);
                entered.send(()).expect("original waiter alive at entry");
                // Test barrier BEFORE the unchanged five-ms authority budget.
                proceed
                    .recv_timeout(Duration::from_secs(1))
                    .map_err(|_| Error::storage())?;
                let writer = db.acquire_authority(
                    &[scope(AuthorityMode::Write)],
                    &AuthorityWait::new(Duration::from_millis(5)),
                )?;
                Ok((identity, writer))
            })
        })
        .await
    });
    observed.await.expect("original blocking callback entered");
    waiter.abort();
    assert!(waiter
        .await
        .expect_err("cancel only async waiter")
        .is_cancelled());
    assert!(
        slot.claim().is_err(),
        "cancel cannot reopen original admission"
    );
    drop(read);
    release.send(()).expect("release same callback");
    let (captured_identity, writer) =
        tokio::time::timeout(Duration::from_secs(1), slot.take_when_done())
            .await
            .expect("original callback settles")
            .expect("late original reservation retained");
    assert!(
        Arc::ptr_eq(&captured_identity, &original_identity),
        "capture exact original outcome after waiter was dropped"
    );
    assert!(store.authority_covers(&writer, &[scope(AuthorityMode::Write)]));
    assert_eq!(
        calls.load(Ordering::Acquire),
        1,
        "no second admission or effect"
    );
    drop(writer);
    drain(&runtime).await;
    assert!(slot.take_when_done().await.is_none());
    assert!(store
        .try_acquire_authority(&[scope(AuthorityMode::Read)])
        .is_ok());
}

#[tokio::test]
async fn refused_codec_submission_settles_claim_without_creating_reservation() {
    let (_directory, _store, runtime) = fixture();
    drain(&runtime).await; // Existing owned-call registry is closed.
    let slot = Arc::new(Slot::<()>::default());
    let pending = slot.claim().expect("original claim");
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    assert!(runtime
        .owned_store_call(move || pending.run(|| {
            count.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }))
        .await
        .is_err());
    assert!(
        tokio::time::timeout(Duration::from_secs(1), slot.take_when_done())
            .await
            .expect("rejected submission releases pending custody")
            .is_none()
    );
    assert_eq!(calls.load(Ordering::Acquire), 0);
    assert!(slot.claim().is_err());
}
