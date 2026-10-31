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

#[tokio::test]
async fn request_observer_counts_actual_shared_http_overload_and_releases_slots() {
    use connect_protocol::{ResourcePoolKind, ResourceSampleState};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    struct HeldAuth {
        entered: AtomicUsize,
        release: Semaphore,
    }
    #[async_trait::async_trait]
    impl McpAuthenticator for HeldAuth {
        async fn authenticate(
            &self,
            _: &axum::http::HeaderMap,
        ) -> connect_protocol::Result<connect_service::tasks::TaskInvocation> {
            self.entered.fetch_add(1, Ordering::SeqCst);
            let _permit = self.release.acquire().await.unwrap();
            Err(Error::forbidden())
        }
    }
    async fn exchange(address: SocketAddr) -> Vec<u8> {
        let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
        let request = format!("POST /mcp HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nContent-Length: 2\r\n\r\n{{}}");
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut response = Vec::new();
        stream.take(4096).read_to_end(&mut response).await.unwrap();
        response
    }
    tokio::time::timeout(Duration::from_secs(15), async {
        let directory = tempfile::tempdir().unwrap();
        let service = Service::new(Arc::new(
            connect_store::Store::initialize(
                &directory.path().join("store.sqlite3"),
                Default::default(),
            )
            .unwrap(),
        ));
        let pool = McpCapacity::default();
        let stop = tokio_util::sync::CancellationToken::new();
        let observer = pool.observe_requests(&service, stop.clone()).unwrap();
        let auth = Arc::new(HeldAuth {
            entered: AtomicUsize::new(0),
            release: Semaphore::new(0),
        });
        let primary = pool
            .start_primary(
                service.clone(),
                auth.clone(),
                "127.0.0.1:0".parse().unwrap(),
            )
            .await
            .unwrap();
        let auxiliary = pool
            .start_auxiliary(
                service.clone(),
                auth.clone(),
                "127.0.0.1:0".parse().unwrap(),
            )
            .await
            .unwrap();
        let mut requests = Vec::new();
        for index in 0..MAX_MCP_REQUESTS {
            let address = if index % 2 == 0 {
                primary.address
            } else {
                auxiliary.address
            };
            requests.push(tokio::spawn(exchange(address)));
        }
        tokio::time::timeout(Duration::from_secs(2), async {
            while auth.entered.load(Ordering::SeqCst) < MAX_MCP_REQUESTS {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let sample = || {
            service
                .readiness
                .resource_pool_snapshot()
                .unwrap()
                .into_iter()
                .find(|pool| pool.kind == ResourcePoolKind::McpRequests)
                .unwrap()
        };
        assert_eq!(sample().usage.unwrap().used, MAX_MCP_REQUESTS as u32);
        let refused = exchange(auxiliary.address).await;
        assert!(refused.starts_with(b"HTTP/1.1 503"));
        assert_eq!(auth.entered.load(Ordering::SeqCst), MAX_MCP_REQUESTS);
        assert_eq!(sample().usage.unwrap().rejected, 1);
        auth.release.add_permits(MAX_MCP_REQUESTS);
        for request in requests {
            assert!(request.await.unwrap().starts_with(b"HTTP/1.1 401"));
        }
        assert_eq!(sample().usage.unwrap().used, 0);
        assert!(exchange(primary.address).await.starts_with(b"HTTP/1.1 401"));
        let usage = sample().usage.unwrap();
        assert_eq!(usage.used, 0);
        assert_eq!(usage.accepted, MAX_MCP_REQUESTS as u64 + 1);
        stop.cancel();
        assert!(sample().usage.unwrap().draining);
        primary.shutdown().await.unwrap();
        auxiliary.shutdown().await.unwrap();
        drop(observer);
        assert_eq!(sample().state, ResourceSampleState::Retired);
    })
    .await
    .unwrap();
}
