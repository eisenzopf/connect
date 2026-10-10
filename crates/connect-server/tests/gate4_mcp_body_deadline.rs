//! Incomplete authenticated bodies release admission at the whole-request deadline.
#![allow(clippy::unwrap_used)]
#[path = "support/mcp_fixture.rs"]
mod fixture;
#[path = "support/mcp_client.rs"]
mod mcp;
use connect_mcp::{FixedBindingAuth, RunningMcp};
use serde_json::json;
use std::{sync::Arc, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stalled_body_times_out_without_a_parsed_tool_and_next_request_succeeds() {
    let f = fixture::Fixture::start().await;
    let listener = RunningMcp::start_local(
        f.server.service.clone(),
        Arc::new(FixedBindingAuth::new(f.server.service.clone(), f.binding.clone()).unwrap()),
        "127.0.0.1:0".parse().unwrap(),
    )
    .await
    .unwrap();
    let mut socket = tokio::net::TcpStream::connect(listener.address)
        .await
        .unwrap();
    let headers = format!(
        "POST /mcp HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nAccept: application/json, text/event-stream\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n1\r\n{{\r\n",
        listener.address, f.issued.expose_secret(),
    );
    socket.write_all(headers.as_bytes()).await.unwrap();
    let mut response = vec![0; 4096];
    let read = tokio::time::timeout(Duration::from_secs(11), socket.read(&mut response))
        .await
        .unwrap()
        .unwrap();
    assert!(response[..read].starts_with(b"HTTP/1.1 504"));
    assert!(!String::from_utf8_lossy(&response[..read]).contains(f.issued.expose_secret()));
    drop(socket);
    let client = mcp::client_from_grant(listener.address, f.issued.expose_secret()).await;
    let status = mcp::success(mcp::call(&client, "connect_task_status", json!({})).await);
    assert_eq!(status["data"]["id"], f.task.id);
    client.cancel().await.unwrap();
    listener.shutdown().await.unwrap();
    f.close().await;
}
