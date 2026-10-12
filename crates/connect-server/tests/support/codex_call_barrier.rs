//! Pause actual MCP admission after model output, before a Connect operation.
use async_trait::async_trait;
use connect_mcp::{FixedBindingAuth, McpAuthenticator};
use connect_protocol::Error;
use connect_service::tasks::TaskInvocation;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Notify, Semaphore};

pub struct Barrier {
    pub auth: FixedBindingAuth,
    pub paused: AtomicBool,
    pub entered: Notify,
    pub release: Semaphore,
}
impl Barrier {
    pub fn new(auth: FixedBindingAuth) -> Self {
        Self {
            auth,
            paused: AtomicBool::new(false),
            entered: Notify::new(),
            release: Semaphore::new(0),
        }
    }
    pub fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
    }
    pub fn resume(&self) {
        self.paused.store(false, Ordering::SeqCst);
        self.release.add_permits(1);
    }
}
#[async_trait]
impl McpAuthenticator for Barrier {
    async fn authenticate(&self, headers: &axum::http::HeaderMap) -> Result<TaskInvocation, Error> {
        let invocation = self.auth.authenticate(headers).await?;
        if self.paused.load(Ordering::SeqCst) {
            self.entered.notify_one();
            self.release
                .acquire()
                .await
                .map_err(|_| Error::new("transport-uncertain", "Fixture admission closed"))?
                .forget();
        }
        Ok(invocation)
    }
}
