//! A stalled authenticator cannot retain admission or decode tool arguments.
#![recursion_limit = "256"]
#![allow(clippy::unwrap_used)]
#[path = "support/mcp_fixture.rs"]
mod fixture;
#[path = "support/mcp_client.rs"]
mod mcp;
use async_trait::async_trait;
use connect_mcp::{FixedBindingAuth, McpAuthenticator, RunningMcp};
use connect_protocol::Error;
use connect_service::tasks::TaskInvocation;
use serde_json::json;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

struct StalledAuth {
    inner: FixedBindingAuth,
    stall: AtomicBool,
}
#[async_trait]
impl McpAuthenticator for StalledAuth {
    async fn authenticate(&self, headers: &axum::http::HeaderMap) -> Result<TaskInvocation, Error> {
        if self.stall.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        self.inner.authenticate(headers).await
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn authentication_timeout_releases_admission_and_a_fresh_request_is_authenticated() {
    let f = fixture::Fixture::start().await;
    let auth = Arc::new(StalledAuth {
        inner: FixedBindingAuth::new(f.server.service.clone(), f.binding.clone()).unwrap(),
        stall: AtomicBool::new(true),
    });
    let listener = RunningMcp::start_local(
        f.server.service.clone(),
        auth.clone(),
        "127.0.0.1:0".parse().unwrap(),
    )
    .await
    .unwrap();
    let http = tokio::task::spawn_blocking(|| {
        reqwest_mcp::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(7))
            .build()
            .unwrap()
    })
    .await
    .unwrap();
    let started = Instant::now();
    let response = http
        .post(format!("http://{}/mcp", listener.address))
        .bearer_auth(f.issued.expose_secret())
        .body("unparsed")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
    assert!(started.elapsed() >= Duration::from_secs(5));
    assert!(started.elapsed() < Duration::from_secs(6));
    auth.stall.store(false, Ordering::SeqCst);
    let client = mcp::client_from_grant(listener.address, f.issued.expose_secret()).await;
    assert_eq!(
        mcp::success(mcp::call(&client, "connect_task_status", json!({})).await)["data"]["id"],
        f.task.id
    );
    client.cancel().await.unwrap();
    listener.shutdown().await.unwrap();
    f.close().await;
}
