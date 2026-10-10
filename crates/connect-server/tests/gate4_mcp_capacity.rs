//! Bounded admission and interrupted authentication cannot orphan a listener.
#![allow(clippy::unwrap_used)]
#[path = "support/mcp_fixture.rs"]
mod fixture;
use async_trait::async_trait;
use axum::http::HeaderMap;
use connect_mcp::{McpAuthenticator, RunningMcp};
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
async fn thirty_two_stalled_admissions_are_bounded_and_shutdown_cancels_them() {
    let f = fixture::Fixture::start().await;
    let auth = Arc::new(StalledAuth {
        started: AtomicUsize::new(0),
        changed: tokio::sync::Notify::new(),
    });
    let listener = RunningMcp::start_local(
        f.server.service.clone(),
        auth.clone(),
        "127.0.0.1:0".parse().unwrap(),
    )
    .await
    .unwrap();
    let url = format!("http://{}/mcp", listener.address);
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
    for _ in 0..32 {
        let http = http.clone();
        let url = url.clone();
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
    let excess = http.post(&url).body("unparsed").send().await.unwrap();
    assert_eq!(excess.status(), 503);
    assert_eq!(
        auth.started.load(Ordering::SeqCst),
        32,
        "excess request entered authentication"
    );
    tokio::time::timeout(Duration::from_secs(2), listener.shutdown())
        .await
        .unwrap()
        .unwrap();
    for request in requests {
        assert_eq!(request.await.unwrap(), 503);
    }
    assert!(
        http.post(&url).send().await.is_err(),
        "closed listener accepted another connection"
    );
    f.close().await;
}
