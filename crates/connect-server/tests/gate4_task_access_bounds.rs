//! Admission precedes body decoding; owned shutdown cancels admitted slow bodies.
#![allow(clippy::unwrap_used)]
#[path = "support/task_access.rs"]
mod access;
#[path = "support/mcp_fixture.rs"]
mod fixture;
use connect_protocol::tasks::InvocationKind;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn stalled(f: &fixture::Fixture) -> tokio::net::TcpStream {
    let address = f.server.http_address.unwrap();
    let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
    let headers = format!("POST /v1/task-access HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nExpect: 100-continue\r\nConnection: close\r\n\r\n",f.owner.credential);
    socket.write_all(headers.as_bytes()).await.unwrap();
    // Hyper sends Continue when the authenticated handler polls the body.
    // Observe admission on the wire instead of assuming a successful TCP write
    // means the request already owns a capacity slot.
    let expected = b"HTTP/1.1 100 Continue\r\n\r\n";
    let mut admitted = vec![0; expected.len()];
    tokio::time::timeout(Duration::from_secs(2), socket.read_exact(&mut admitted))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(admitted, expected);
    socket.write_all(b"1\r\n{\r\n").await.unwrap();
    socket
}
async fn response(mut socket: tokio::net::TcpStream) -> String {
    let mut bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(7), socket.read_to_end(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    String::from_utf8(bytes).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn authenticated_slow_body_releases_admission_at_its_deadline() {
    let f = fixture::Fixture::start_independent().await;
    let started = tokio::time::Instant::now();
    let socket = stalled(&f).await;
    let submitted = started.elapsed();
    let response = response(socket).await;
    assert!(response.starts_with("HTTP/1.1 503"));
    assert!(response.contains("transport-uncertain"));
    assert!(
        started.elapsed() < Duration::from_secs(7),
        "request submission took {submitted:?}; total {:?}",
        started.elapsed()
    );
    let grant = access::issued(
        access::send(
            &f,
            &access::request(&f.task.id, "after-deadline", InvocationKind::Native),
        )
        .await,
    );
    assert_eq!(grant.grant.task_id, f.task.id);
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn capacity_and_body_bounds_hold_and_shutdown_cancels_admitted_uploads() {
    let client = access::http();
    let f = fixture::Fixture::start_independent_with_connections(2).await;
    for token in [&f.owner.credential, "invalid"] {
        let response = client
            .post(access::url(&f))
            .bearer_auth(token)
            .header("content-type", "application/json")
            .body(vec![b'x'; 65_537])
            .send()
            .await
            .unwrap();
        let expected = if token == f.owner.credential {
            reqwest::StatusCode::PAYLOAD_TOO_LARGE
        } else {
            reqwest::StatusCode::UNAUTHORIZED
        };
        assert_eq!(response.status(), expected);
        assert_eq!(response.headers()["cache-control"], "no-store");
    }
    let a = stalled(&f).await;
    let b = stalled(&f).await;
    let request = access::request(&f.task.id, "capacity-denied", InvocationKind::Native);
    let probe = tokio::time::timeout(
        Duration::from_secs(2),
        client
            .post(access::url(&f))
            .bearer_auth(&f.owner.credential)
            .json(&request)
            .send(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(probe.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
    let body: connect_protocol::tasks::TaskAccessResponse = probe.json().await.unwrap();
    assert_eq!(body.result.unwrap_err().code, "capacity-exceeded");
    tokio::time::timeout(Duration::from_secs(4), f.close())
        .await
        .unwrap();
    for socket in [a, b] {
        let text = response(socket).await;
        assert!(text.starts_with("HTTP/1.1 503"));
        assert!(text.contains("transport-uncertain"));
    }
}
