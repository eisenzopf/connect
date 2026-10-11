//! Bounded admission and interrupted authentication cannot orphan a listener.
#![allow(clippy::unwrap_used)]
#[path = "support/mcp_fixture.rs"]
mod fixture;
use async_trait::async_trait;
use axum::http::HeaderMap;
use connect_mcp::{McpAuthenticator, McpCapacity};
use connect_protocol::Error;
use connect_service::tasks::TaskInvocation;
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

struct StalledAuth {
    started: AtomicUsize,
    changed: tokio::sync::Notify,
}
#[async_trait]
impl McpAuthenticator for StalledAuth {
    async fn authenticate(&self, _: &HeaderMap) -> Result<TaskInvocation, Error> {
        self.started.fetch_add(1, Ordering::SeqCst);
        self.changed.notify_waiters();
        std::future::pending().await
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn primary_and_task_endpoints_share_thirty_two_admissions_and_recover_after_shutdown() {
    let f = fixture::Fixture::start().await;
    let auth = Arc::new(StalledAuth {
        started: AtomicUsize::new(0),
        changed: tokio::sync::Notify::new(),
    });
    let capacity = McpCapacity::default();
    let listener = capacity
        .start_primary(
            f.server.service.clone(),
            auth.clone(),
            "127.0.0.1:0".parse().unwrap(),
        )
        .await
        .unwrap();
    let auxiliary = capacity
        .clone()
        .start_auxiliary(
            f.server.service.clone(),
            auth.clone(),
            "127.0.0.1:0".parse().unwrap(),
        )
        .await
        .unwrap();
    let url = format!("http://{}/mcp", listener.address);
    let auxiliary_url = format!("http://{}/mcp", auxiliary.address);
    let http = tokio::task::spawn_blocking(|| {
        reqwest_mcp::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap()
    })
    .await
    .unwrap();
    let mut requests = Vec::new();
    for index in 0..32 {
        let http = http.clone();
        let url = if index < 16 {
            url.clone()
        } else {
            auxiliary_url.clone()
        };
        requests.push(tokio::spawn(async move {
            http.post(url)
                .body("unparsed")
                .send()
                .await
                .unwrap()
                .status()
        }));
    }
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let wake = auth.changed.notified();
            tokio::pin!(wake);
            wake.as_mut().enable();
            if auth.started.load(Ordering::SeqCst) == 32 {
                break;
            }
            wake.await;
        }
    })
    .await
    .unwrap();
    for endpoint in [&url, &auxiliary_url] {
        let excess = http.post(endpoint).body("unparsed").send().await.unwrap();
        assert_eq!(excess.status(), 503);
    }
    assert_eq!(
        auth.started.load(Ordering::SeqCst),
        32,
        "excess request entered authentication"
    );
    tokio::time::timeout(Duration::from_secs(2), listener.shutdown())
        .await
        .unwrap()
        .unwrap();
    for request in requests.drain(..16) {
        assert_eq!(request.await.unwrap(), 503);
    }
    // Closing one endpoint frees its request permits for the other endpoint.
    let next_http = http.clone();
    let next_url = auxiliary_url.clone();
    requests.push(tokio::spawn(async move {
        next_http
            .post(next_url)
            .body("unparsed")
            .send()
            .await
            .unwrap()
            .status()
    }));
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let wake = auth.changed.notified();
            tokio::pin!(wake);
            wake.as_mut().enable();
            if auth.started.load(Ordering::SeqCst) == 33 {
                break;
            }
            wake.await;
        }
    })
    .await
    .unwrap();
    auxiliary.shutdown().await.unwrap();
    for request in requests {
        assert_eq!(request.await.unwrap(), 503);
    }
    assert!(
        http.post(&url).send().await.is_err(),
        "closed listener accepted another connection"
    );
    f.close().await;
}
