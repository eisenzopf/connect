//! Shutdown cancels admitted body decoding before a tool context exists.
#![allow(clippy::unwrap_used)]
#[path = "support/mcp_fixture.rs"]
mod fixture;
use async_trait::async_trait;
use connect_mcp::{FixedBindingAuth, McpAuthenticator, RunningMcp};
use connect_protocol::Error;
use connect_service::tasks::TaskInvocation;
use std::{sync::Arc, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct ObservedAuth {
    inner: FixedBindingAuth,
    entered: tokio::sync::Notify,
}
#[async_trait]
impl McpAuthenticator for ObservedAuth {
    async fn authenticate(&self, headers: &axum::http::HeaderMap) -> Result<TaskInvocation, Error> {
        let result = self.inner.authenticate(headers).await;
        self.entered.notify_one();
        result
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stalled_authenticated_chunked_body_is_cancelled_and_connection_is_drained() {
    let f = fixture::Fixture::start().await;
    let auth = Arc::new(ObservedAuth {
        inner: FixedBindingAuth::new(f.server.service.clone(), f.binding.clone()).unwrap(),
        entered: tokio::sync::Notify::new(),
    });
    let listener = RunningMcp::start_local(
        f.server.service.clone(),
        auth.clone(),
        "127.0.0.1:0".parse().unwrap(),
    )
    .await
    .unwrap();
    let mut socket = tokio::net::TcpStream::connect(listener.address)
        .await
        .unwrap();
    let header = format!("POST /mcp HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nAccept: application/json, text/event-stream\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n1\r\n{{\r\n", listener.address, f.issued.expose_secret());
    socket.write_all(header.as_bytes()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), auth.entered.notified())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), listener.shutdown())
        .await
        .unwrap()
        .unwrap();
    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(2), socket.read_to_end(&mut response))
        .await
        .unwrap()
        .unwrap();
    assert!(response.starts_with(b"HTTP/1.1 503"));
    assert!(!String::from_utf8_lossy(&response).contains(f.issued.expose_secret()));
    f.close().await;
}
