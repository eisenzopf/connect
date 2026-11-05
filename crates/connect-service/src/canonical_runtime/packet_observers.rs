use super::*;
use rvoip_uctp::v2::reachability::PeerTransport;
use tokio_util::sync::CancellationToken;

#[tokio::test(flavor = "current_thread")]
async fn physical_cold_reopen_closes_its_pending_batch_without_async_custodian_progress() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(
        connect_store::Store::initialize(
            &directory.path().join("state.sqlite3"),
            Default::default(),
        )
        .unwrap(),
    );
    let runtime = Runtime::new(store.clone()).unwrap();
    assert!(runtime.session_codec_worker.lock().unwrap().is_none());
    let cutoff = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    let context =
        || NativeTransportContext::new(PeerTransport::Quic, CancellationToken::new(), cutoff, ());
    let first = runtime.physical(context()).unwrap();
    let original = Arc::downgrade(&first);
    drop(first);
    assert!(original.upgrade().is_none());
    // No yield and no async custodian exists. A synchronous admission that waits
    // for such a task would consume its original cutoff and fail here.
    let second = runtime.physical(context());
    let elapsed_ok = tokio::time::Instant::now() < cutoff;
    let success = second.is_ok();
    drop(second);
    let reaped = store.reap_idle_packet_observers().unwrap();
    runtime.stop.cancel();
    runtime
        .core
        .drain_connection_lifecycle_tasks()
        .await
        .unwrap();
    assert!(
        success && elapsed_ok,
        "cold physical reopen waited for an unavailable executor custodian"
    );
    assert!(reaped);
}
