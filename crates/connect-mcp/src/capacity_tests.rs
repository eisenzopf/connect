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

#[tokio::test]
async fn raw_shared_sockets_refuse_before_auth_expire_and_join_auxiliary_drain() {
    use connect_protocol::{Limits, ResourcePoolKind};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    struct CountDeny(AtomicUsize);
    #[async_trait::async_trait]
    impl McpAuthenticator for CountDeny {
        async fn authenticate(
            &self,
            _: &axum::http::HeaderMap,
        ) -> connect_protocol::Result<connect_service::tasks::TaskInvocation> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(Error::forbidden())
        }
    }
    tokio::time::timeout(Duration::from_secs(50), async {
        let directory = tempfile::tempdir().unwrap();
        let service = Service::new(Arc::new(connect_store::Store::initialize(
            &directory.path().join("store.sqlite3"), Limits { connections: 1, ..Default::default() }).unwrap()));
        let pool = McpCapacity::with_connection_limit(1).unwrap();
        let stop = tokio_util::sync::CancellationToken::new();
        let _connections = pool.observe_connections(&service, stop.clone()).unwrap();
        let _requests = pool.observe_requests(&service, stop).unwrap();
        let auth = Arc::new(CountDeny(AtomicUsize::new(0)));
        let primary = pool.start_primary(service.clone(), auth.clone(), "127.0.0.1:0".parse().unwrap()).await.unwrap();
        let auxiliary = pool.start_auxiliary(service.clone(), auth.clone(), "127.0.0.1:0".parse().unwrap()).await.unwrap();
        let sample = |kind| service.readiness.resource_pool_snapshot().unwrap().into_iter()
            .find(|pool| pool.kind == kind).unwrap().usage.unwrap();
        let mut abandoned = tokio::net::TcpStream::connect(primary.address).await.unwrap();
        abandoned.write_all(b"G").await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            while sample(ResourcePoolKind::McpConnections).used != 1 { tokio::task::yield_now().await; }
        }).await.unwrap();
        let mut rejected = tokio::net::TcpStream::connect(auxiliary.address).await.unwrap();
        let mut byte = [0u8; 1];
        let value = tokio::time::timeout(Duration::from_secs(3), rejected.read(&mut byte)).await.unwrap();
        assert!(matches!(value, Ok(0) | Err(_)));
        assert_eq!(sample(ResourcePoolKind::McpConnections).rejected, 1);
        assert_eq!(sample(ResourcePoolKind::McpRequests).used, 0);
        assert_eq!(auth.0.load(Ordering::SeqCst), 0);
        let value = tokio::time::timeout(Duration::from_secs(35), abandoned.read(&mut byte)).await.unwrap();
        assert!(matches!(value, Ok(0) | Err(_)));
        tokio::time::timeout(Duration::from_secs(3), async {
            while sample(ResourcePoolKind::McpConnections).used != 0 { tokio::task::yield_now().await; }
        }).await.unwrap();
        let mut admitted = tokio::net::TcpStream::connect(primary.address).await.unwrap();
        admitted.write_all(format!("POST /mcp HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Length: 2\r\n\r\n{{}}", primary.address).as_bytes()).await.unwrap();
        let mut response = Vec::new();
        tokio::time::timeout(Duration::from_secs(3), admitted.read_to_end(&mut response)).await.unwrap().unwrap();
        assert!(response.starts_with(b"HTTP/1.1 401"));
        assert_eq!(auth.0.load(Ordering::SeqCst), 1);
        tokio::time::timeout(Duration::from_secs(3), async {
            while sample(ResourcePoolKind::McpConnections).used != 0 { tokio::task::yield_now().await; }
        }).await.unwrap();
        let mut held = tokio::net::TcpStream::connect(auxiliary.address).await.unwrap();
        held.write_all(b"G").await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            while sample(ResourcePoolKind::McpConnections).used != 1 { tokio::task::yield_now().await; }
        }).await.unwrap();
        let primary_address = primary.address; let auxiliary_address = auxiliary.address;
        primary.shutdown().await.unwrap();
        assert_eq!(sample(ResourcePoolKind::McpConnections).used, 1, "Primary shutdown must not release an auxiliary socket slot");
        auxiliary.shutdown().await.unwrap();
        let value = tokio::time::timeout(Duration::from_secs(2), held.read(&mut byte)).await.unwrap();
        assert!(matches!(value, Ok(0) | Err(_)));
        assert_eq!(sample(ResourcePoolKind::McpConnections).used, 0);
        assert_eq!(auth.0.load(Ordering::SeqCst), 1);
        drop(tokio::net::TcpListener::bind(primary_address).await.unwrap());
        drop(tokio::net::TcpListener::bind(auxiliary_address).await.unwrap());
    }).await.unwrap();
}

#[tokio::test]
async fn actual_trickling_socket_cannot_extend_absolute_lifetime_or_release_its_permit_early() {
    use axum::serve::Listener;
    use connect_protocol::ResourcePoolKind;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    tokio::time::timeout(Duration::from_secs(130), async {
        let directory = tempfile::tempdir().unwrap();
        let service = Service::new(Arc::new(connect_store::Store::initialize(&directory.path().join("store.sqlite3"), Default::default()).unwrap()));
        let pool = McpCapacity::with_connection_limit(1).unwrap();
        let stop = tokio_util::sync::CancellationToken::new();
        let _observer = pool.observe_connections(&service, stop.clone()).unwrap();
        let raw = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = raw.local_addr().unwrap();
        let mut listener = crate::socket_io::BoundedListener { listener: raw, capacity: pool.connections.clone(), cancellation: stop.clone() };
        let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
        let (mut original, _) = listener.accept().await;
        let finished = tokio_util::sync::CancellationToken::new();
        let done = finished.clone();
        let writer = tokio::spawn(async move {
            loop {
                if client.write_all(b"x").await.is_err() { break; }
                tokio::select! { _ = done.cancelled() => break, _ = tokio::time::sleep(Duration::from_secs(5)) => {} }
            }
        });
        let started = tokio::time::Instant::now();
        let mut byte = [0u8; 1]; let mut received = 0;
        loop {
            match original.read(&mut byte).await {
                Ok(1) => received += 1,
                Err(error) => { assert_eq!(error.kind(), io::ErrorKind::TimedOut); break; },
                _ => panic!("Original trickling socket closed before its fixed lifetime"),
            }
        }
        assert!(started.elapsed() >= Duration::from_secs(115) && started.elapsed() <= Duration::from_secs(125));
        assert!(received >= 20);
        let used = || service.readiness.resource_pool_snapshot().unwrap().into_iter().find(|p| p.kind == ResourcePoolKind::McpConnections).unwrap().usage.unwrap().used;
        assert_eq!(used(), 1, "A timeout does not release a still-owned socket");
        drop(original);
        assert_eq!(used(), 0);
        finished.cancel(); writer.await.unwrap();
    }).await.unwrap();
}

#[tokio::test]
async fn explicit_pool_cannot_exceed_current_service_connection_configuration() {
    let directory = tempfile::tempdir().unwrap();
    let service = Service::new(Arc::new(
        connect_store::Store::initialize(
            &directory.path().join("store.sqlite3"),
            connect_protocol::Limits {
                connections: 1,
                ..Default::default()
            },
        )
        .unwrap(),
    ));
    assert!(McpCapacity::with_connection_limit(0).is_err());
    assert!(McpCapacity::with_connection_limit(usize::MAX).is_err());
    let before = service.readiness.snapshot();
    let result = McpCapacity::default()
        .start_primary(
            service.clone(),
            Arc::new(Deny),
            "127.0.0.1:0".parse().unwrap(),
        )
        .await;
    assert_eq!(result.err().unwrap().kind(), io::ErrorKind::InvalidInput);
    assert_eq!(service.readiness.snapshot(), before);
}
