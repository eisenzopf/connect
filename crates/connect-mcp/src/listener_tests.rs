//! Actual owned listeners exercise abort-before-poll and independent teardown.
#![allow(clippy::unwrap_used)]
use super::*;
use connect_protocol::{Error, ReadinessState};
use std::future::Future;
struct Deny;
#[async_trait::async_trait]
impl McpAuthenticator for Deny {
    async fn authenticate(
        &self,
        _: &axum::http::HeaderMap,
    ) -> connect_protocol::Result<connect_service::tasks::TaskInvocation> {
        Err(Error::forbidden())
    }
}
fn fixture() -> (tempfile::TempDir, Arc<Service>) {
    let directory = tempfile::tempdir().unwrap();
    let store = connect_store::Store::initialize(
        &directory.path().join("fixture.sqlite3"),
        Default::default(),
    )
    .unwrap();
    (directory, Service::new(Arc::new(store)))
}
#[tokio::test]
async fn abort_before_first_poll_observes_failed_listener_and_wakes_supervision() {
    let (_directory, service) = fixture();
    let listener = RunningMcp::start_local(
        service.clone(),
        Arc::new(Deny),
        "127.0.0.1:0".parse().unwrap(),
    )
    .await
    .unwrap();
    listener.handle.as_ref().unwrap().abort();
    tokio::time::timeout(Duration::from_secs(1), service.readiness.failed())
        .await
        .unwrap();
    let state = service.readiness.snapshot();
    assert_eq!(state.state, ReadinessState::Unavailable);
    assert_eq!(state.components[0].reason, "worker-exited");
    assert!(state.components[0].configured);
    assert!(listener.shutdown().await.is_err());
}
#[tokio::test]
async fn normal_auxiliary_shutdown_preserves_live_primary_readiness() {
    let (_directory, service) = fixture();
    let main = RunningMcp::start_local(
        service.clone(),
        Arc::new(Deny),
        "127.0.0.1:0".parse().unwrap(),
    )
    .await
    .unwrap();
    let auxiliary = RunningMcp::start_local_auxiliary(
        service.clone(),
        Arc::new(Deny),
        "127.0.0.1:0".parse().unwrap(),
    )
    .await
    .unwrap();
    auxiliary.shutdown().await.unwrap();
    assert_eq!(service.readiness.snapshot().state, ReadinessState::Ready);
    main.shutdown().await.unwrap();
    let state = service.readiness.snapshot();
    assert_eq!(state.state, ReadinessState::Unavailable);
    assert_eq!(state.components[0].reason, "listener-stopped");
    assert!(
        tokio::time::timeout(Duration::from_millis(20), service.readiness.failed())
            .await
            .is_err()
    );
}
#[tokio::test]
async fn dropping_primary_without_explicit_shutdown_never_leaves_ready_observation() {
    let (_directory, service) = fixture();
    let listener = RunningMcp::start_local(
        service.clone(),
        Arc::new(Deny),
        "127.0.0.1:0".parse().unwrap(),
    )
    .await
    .unwrap();
    drop(listener);
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if service.readiness.snapshot().state == ReadinessState::Unavailable {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        service.readiness.snapshot().components[0].reason,
        "listener-stopped"
    );
}
#[tokio::test]
async fn cancelling_pending_shutdown_aborts_owned_listener_and_retains_honest_readiness() {
    let (_directory, service) = fixture();
    let listener = RunningMcp::start_local(
        service.clone(),
        Arc::new(Deny),
        "127.0.0.1:0".parse().unwrap(),
    )
    .await
    .unwrap();
    let address = listener.address;
    let mut shutdown = Box::pin(listener.shutdown());
    std::future::poll_fn(|cx| {
        assert!(shutdown.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    let state = service.readiness.snapshot();
    assert_eq!(state.state, ReadinessState::Degraded);
    assert_eq!(state.components[0].reason, "listener-draining");
    drop(shutdown);
    tokio::time::timeout(Duration::from_secs(1), async {
        while service.readiness.snapshot().state != ReadinessState::Unavailable {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        service.readiness.snapshot().components[0].reason,
        "listener-stopped"
    );
    assert!(tokio::net::TcpStream::connect(address).await.is_err());
}
