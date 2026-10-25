//! Whole-request deadlines include admission and body decoding, not only wait_ms.
#![recursion_limit = "256"]
#![allow(clippy::unwrap_used)]
#[path = "support/mcp_fixture.rs"]
mod fixture;
#[path = "support/mcp_client.rs"]
mod mcp;
use async_trait::async_trait;
use connect_mcp::{FixedBindingAuth, McpAuthenticator, RunningMcp};
use connect_protocol::{tasks::TaskPermission, Error};
use connect_service::tasks::TaskInvocation;
use serde_json::json;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

struct DelayedAuth {
    inner: FixedBindingAuth,
    delay: AtomicBool,
}
#[async_trait]
impl McpAuthenticator for DelayedAuth {
    async fn authenticate(&self, headers: &axum::http::HeaderMap) -> Result<TaskInvocation, Error> {
        let result = self.inner.authenticate(headers).await;
        if self.delay.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        result
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn maximum_wait_includes_slow_admission_and_returns_usable_pending_cursor() {
    let f = fixture::Fixture::start_permitted(false, vec![TaskPermission::Inbox]).await;
    let auth = Arc::new(DelayedAuth {
        inner: FixedBindingAuth::new(f.server.service.clone(), f.binding.clone()).unwrap(),
        delay: AtomicBool::new(false),
    });
    let listener = RunningMcp::start_local(
        f.server.service.clone(),
        auth.clone(),
        "127.0.0.1:0".parse().unwrap(),
    )
    .await
    .unwrap();
    let client = mcp::client_from_grant(listener.address, f.issued.expose_secret()).await;
    let inbox = mcp::success(
        mcp::call(
            &client,
            "connect_task_wait",
            json!({"limit":100,"wait_ms":0}),
        )
        .await,
    );
    let cursor = inbox["data"]["next"].clone();
    auth.delay.store(true, Ordering::SeqCst);
    let started = Instant::now();
    let pending = mcp::call(
        &client,
        "connect_task_wait",
        json!({"after":cursor,"limit":100,"wait_ms":10000}),
    )
    .await;
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(10),
        "whole tool request exceeded ten seconds: {elapsed:?}"
    );
    assert!(
        elapsed >= Duration::from_secs(7),
        "wait did not exercise the budget: {elapsed:?}"
    );
    assert_ne!(pending.is_error, Some(true));
    assert_eq!(pending.meta.unwrap()["connect"]["pending"], true);
    let inbox = pending.structured_content.unwrap();
    assert_eq!(inbox["data"]["events"], json!([]));
    assert_eq!(inbox["data"]["next"], cursor);
    auth.delay.store(false, Ordering::SeqCst);
    let resumed = mcp::success(
        mcp::call(
            &client,
            "connect_task_wait",
            json!({"after":cursor,"limit":100,"wait_ms":0}),
        )
        .await,
    );
    assert_eq!(resumed, inbox);
    client.cancel().await.unwrap();
    listener.shutdown().await.unwrap();
    f.close().await;
}
