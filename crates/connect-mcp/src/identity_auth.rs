//! Ordinary identity access is the local default; narrow grants stay explicit.
use crate::{auth::bearer, LocalGrantAuth, McpAccess, McpAuthenticator};
use async_trait::async_trait;
use axum::http::HeaderMap;
use connect_protocol::Error;
use connect_service::{tasks::TaskInvocation, Service};
use std::sync::Arc;

/// Local MCP accepts a reusable revocable communication credential or an
/// existing protected native/host task grant. Provider origins require their
/// separately verified adapter; headers never select an identity or task.
pub struct LocalAuth {
    service: Arc<Service>,
}
impl LocalAuth {
    /// Compose from the server's existing application service. Ordinary identity
    /// access needs no per-task issuance or additional invocation registration.
    pub fn new(service: Arc<Service>) -> Self {
        Self { service }
    }
}
#[async_trait]
impl McpAuthenticator for LocalAuth {
    /// Preserve explicit legacy narrow admission for trusted callers of this
    /// method; an identity credential cannot be converted into a task grant.
    async fn authenticate(&self, headers: &HeaderMap) -> Result<TaskInvocation, Error> {
        LocalGrantAuth::new(self.service.clone())
            .authenticate(headers)
            .await
    }

    async fn authenticate_access(&self, headers: &HeaderMap) -> Result<McpAccess, Error> {
        let token = bearer(headers)?;
        if token.starts_with("ctg1.") {
            return self.authenticate(headers).await.map(McpAccess::Task);
        }
        if ["x-call-id", "x-chat-id", "x-session-id"]
            .iter()
            .any(|name| headers.contains_key(*name))
        {
            return Err(Error::forbidden());
        }
        self.service
            .authenticate_identity(token.to_owned())
            .await
            .map(McpAccess::Identity)
    }
}
