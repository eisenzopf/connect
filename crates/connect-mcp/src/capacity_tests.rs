//! Real sockets prove admission and slot recovery, including cancelled shutdown.
#![allow(clippy::unwrap_used)]
use super::*;
use connect_protocol::Error;
use std::{future::Future, time::Duration};
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
#[tokio::test]
async fn shared_listener_limit_counts_starting_primary_and_auxiliary_until_exit() {
    let directory = tempfile::tempdir().unwrap();
    let service = Service::new(Arc::new(
        connect_store::Store::initialize(
            &directory.path().join("store.sqlite3"),
            Default::default(),
        )
        .unwrap(),
    ));
    let pool = McpCapacity::default();
    let address = "127.0.0.1:0".parse().unwrap();
    let mut endpoints = vec![pool
        .start_primary(service.clone(), Arc::new(Deny), address)
        .await
        .unwrap()];
    for _ in 1..MAX_MCP_LISTENERS {
        endpoints.push(
            pool.clone()
                .start_auxiliary(service.clone(), Arc::new(Deny), address)
                .await
                .unwrap(),
        );
    }
    assert_eq!(pool.listeners.available_permits(), 0);
    let before = service.readiness.snapshot();
    let rejected = pool
        .start_auxiliary(service.clone(), Arc::new(Deny), address)
        .await;
    assert_eq!(rejected.err().unwrap().kind(), io::ErrorKind::WouldBlock);
    assert_eq!(service.readiness.snapshot(), before);
    let stopped = endpoints.pop().unwrap();
    let old_address = stopped.address;
    let mut shutdown = Box::pin(stopped.shutdown());
    std::future::poll_fn(|cx| {
        assert!(shutdown.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    assert_eq!(
        pool.listeners.available_permits(),
        0,
        "pending shutdown released its slot early"
    );
    drop(shutdown);
    tokio::time::timeout(Duration::from_secs(1), async {
        while pool.listeners.available_permits() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(tokio::net::TcpStream::connect(old_address).await.is_err());
    // Binding fails after slot admission. The slot must return for the next start.
    let occupied = tokio::net::TcpListener::bind(address).await.unwrap();
    assert_eq!(
        pool.start_auxiliary(
            service.clone(),
            Arc::new(Deny),
            occupied.local_addr().unwrap()
        )
        .await
        .err()
        .unwrap()
        .kind(),
        io::ErrorKind::AddrInUse
    );
    assert_eq!(pool.listeners.available_permits(), 1);
    endpoints.push(
        pool.start_auxiliary(service.clone(), Arc::new(Deny), address)
            .await
            .unwrap(),
    );
    assert_eq!(pool.listeners.available_permits(), 0);
    // Failed starts and auxiliary teardown never change the live main endpoint.
    assert_eq!(service.readiness.snapshot(), before);
    for endpoint in endpoints {
        endpoint.shutdown().await.unwrap();
    }
    assert_eq!(pool.listeners.available_permits(), MAX_MCP_LISTENERS);
}
