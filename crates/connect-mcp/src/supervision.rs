//! Exit observation must survive panic, cancellation and an unpolled task.
use connect_protocol::ReadinessState;
use connect_service::readiness::OperationalReadiness;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub(crate) struct Observation {
    readiness: Arc<OperationalReadiness>,
    stop: CancellationToken,
    primary: bool,
}
impl Observation {
    pub fn new(
        readiness: Arc<OperationalReadiness>,
        stop: CancellationToken,
        primary: bool,
    ) -> Self {
        if primary {
            readiness.set("mcp", ReadinessState::Ready, "listener-bound");
        }
        Self {
            readiness,
            stop,
            primary,
        }
    }
}
pub(crate) fn draining(readiness: &OperationalReadiness, primary: bool) {
    if primary {
        readiness.set("mcp", ReadinessState::Degraded, "listener-draining");
    }
}
impl Drop for Observation {
    fn drop(&mut self) {
        if !self.stop.is_cancelled() {
            self.readiness.fail(if self.primary {
                "mcp"
            } else {
                "mcp-task-listener"
            });
        } else if self.primary {
            self.readiness
                .set("mcp", ReadinessState::Unavailable, "listener-stopped");
        }
    }
}
