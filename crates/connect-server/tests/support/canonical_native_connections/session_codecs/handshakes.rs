//! Fixture peers independently drive each original exchange to completion.
use futures_util::FutureExt;
use std::{future::Future, pin::Pin};
pub(super) type OriginalHandshake<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;
pub(super) async fn collect_original_handshakes<T>(
    work: Vec<OriginalHandshake<'_, T>>,
) -> Vec<std::thread::Result<T>> {
    futures_util::future::join_all(
        work.into_iter()
            .map(|work| std::panic::AssertUnwindSafe(work).catch_unwind()),
    )
    .await
}

#[tokio::test]
async fn independent_codec_peers_answer_reverse_offer_order_and_release_actual_cid_writer() {
    use connect_store::{AuthorityMode, AuthorityScope, Store};
    use std::{sync::Arc, time::Duration};
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(
        Store::initialize(
            &directory.path().join("fixture.sqlite3"),
            Default::default(),
        )
        .unwrap(),
    );
    let scope = |mode| AuthorityScope::conversation("codec-peer-order", mode).unwrap();
    let first = store
        .try_acquire_authority(&[scope(AuthorityMode::Read)])
        .unwrap();
    let (offer0, received0) = tokio::sync::oneshot::channel();
    let (offer1, received1) = tokio::sync::oneshot::channel();
    let (answer0, answered0) = tokio::sync::oneshot::channel();
    let (answer1, answered1) = tokio::sync::oneshot::channel();
    let db = store.clone();
    let original = tokio::spawn(async move {
        // Actual peer1 owns its broad read until the original answer. Peer0
        // cannot offer until its real same-CID mutation writer can progress.
        offer1.send(()).map_err(|_| "peer1 receive disappeared")?;
        answered1
            .await
            .map_err(|_| "peer1 original answer disappeared")?;
        drop(first);
        let writer = db
            .try_acquire_authority(&[scope(AuthorityMode::Write)])
            .map_err(|_| "same CID writer did not drain")?;
        offer0.send(()).map_err(|_| "peer0 receive disappeared")?;
        answered0
            .await
            .map_err(|_| "peer0 original answer disappeared")?;
        drop(writer);
        Ok::<_, &'static str>(())
    });
    let work: Vec<OriginalHandshake<'_, usize>> = vec![
        Box::pin(async move {
            received0.await.unwrap();
            answer0.send(()).unwrap();
            0
        }),
        Box::pin(async move {
            received1.await.unwrap();
            answer1.send(()).unwrap();
            1
        }),
    ];
    let completed =
        tokio::time::timeout(Duration::from_secs(1), collect_original_handshakes(work)).await;
    // On timeout the dropped original handshakes close their answer channels;
    // the same original server task still joins before an assertion reports it.
    let joined = original.await;
    assert!(
        joined.unwrap().is_ok(),
        "original offers/answers must naturally join"
    );
    let result = completed.expect("source-first sequential await strands the opposite peer");
    assert_eq!(
        result.into_iter().map(Result::unwrap).collect::<Vec<_>>(),
        vec![0, 1]
    );
    assert!(store
        .try_acquire_authority(&[scope(AuthorityMode::Read)])
        .is_ok());
}

#[tokio::test]
async fn failed_codec_handshake_keeps_other_original_handshake_owned_until_completion() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    let finished = Arc::new(AtomicBool::new(false));
    let observed = finished.clone();
    let work: Vec<OriginalHandshake<'_, usize>> = vec![
        Box::pin(async { panic!("original-codec-handshake-panic") }),
        Box::pin(async move {
            tokio::task::yield_now().await;
            observed.store(true, Ordering::Release);
            1
        }),
    ];
    let result = collect_original_handshakes(work).await;
    assert!(
        finished.load(Ordering::Acquire),
        "original sibling must settle despite first panic"
    );
    assert_eq!(result.len(), 2);
    assert_eq!(
        result[0].as_ref().unwrap_err().downcast_ref::<&str>(),
        Some(&"original-codec-handshake-panic")
    );
    assert_eq!(*result[1].as_ref().unwrap(), 1);
}
