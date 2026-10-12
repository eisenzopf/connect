//! Aggregate resource admission for one server's MCP endpoints.
use crate::{McpAuthenticator, RunningMcp};
use connect_service::Service;
use std::{io, net::SocketAddr, sync::Arc};
use tokio::sync::Semaphore;

/// Maximum simultaneous requests across one server's primary and task endpoints.
pub const MAX_MCP_REQUESTS: usize = 32;
/// Maximum live or starting MCP listeners within one server's capacity pool.
pub const MAX_MCP_LISTENERS: usize = 32;

/// Shared admission for a server's endpoints. Clone this pool for each task;
/// creating a fresh pool would create an independent resource allowance.
/// Overload fails immediately rather than adding an unbounded admission queue.
#[derive(Clone)]
pub struct McpCapacity {
    pub(crate) requests: Arc<Semaphore>,
    pub(crate) listeners: Arc<Semaphore>,
}
impl Default for McpCapacity {
    fn default() -> Self {
        Self {
            requests: Arc::new(Semaphore::new(MAX_MCP_REQUESTS)),
            listeners: Arc::new(Semaphore::new(MAX_MCP_LISTENERS)),
        }
    }
}
impl McpCapacity {
    /// Bind a primary endpoint using this pool. Shutdown updates main readiness.
    pub async fn start_primary(
        &self,
        service: Arc<Service>,
        authenticator: Arc<dyn McpAuthenticator>,
        bind: SocketAddr,
    ) -> io::Result<RunningMcp> {
        RunningMcp::start(service, authenticator, bind, true, self).await
    }

    /// Bind a task endpoint using this pool without replacing main readiness.
    /// The returned handle owns shutdown; its slot remains reserved until the
    /// accept task exits, including when the handle or shutdown future is dropped.
    pub async fn start_auxiliary(
        &self,
        service: Arc<Service>,
        authenticator: Arc<dyn McpAuthenticator>,
        bind: SocketAddr,
    ) -> io::Result<RunningMcp> {
        RunningMcp::start(service, authenticator, bind, false, self).await
    }
}

#[cfg(test)]
#[path = "capacity_tests.rs"]
mod tests;
