//! One admission deadline shared with parsed tool work.
use connect_protocol::tasks::MAX_TOOL_WAIT_MS;
use std::time::Duration;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

const RESPONSE_RESERVE: Duration = Duration::from_millis(500);
const WAIT_PROJECTION_RESERVE: Duration = Duration::from_millis(250);

#[derive(Clone)]
pub(crate) struct RequestBudget {
    pub http_deadline: Instant,
    pub tool_deadline: Instant,
    pub cancellation: CancellationToken,
}
impl RequestBudget {
    pub fn new() -> Self {
        let http_deadline = Instant::now() + Duration::from_millis(u64::from(MAX_TOOL_WAIT_MS));
        Self {
            http_deadline,
            tool_deadline: http_deadline - RESPONSE_RESERVE,
            cancellation: CancellationToken::new(),
        }
    }
    /// Leave time to recheck current authority and project the authoritative
    /// inbox page. Never invent an empty page when work has not completed.
    pub fn limit_wait(&self, requested: u32) -> u32 {
        let remaining = self
            .tool_deadline
            .saturating_duration_since(Instant::now())
            .saturating_sub(WAIT_PROJECTION_RESERVE);
        requested.min(u32::try_from(remaining.as_millis()).unwrap_or(u32::MAX))
    }
}
