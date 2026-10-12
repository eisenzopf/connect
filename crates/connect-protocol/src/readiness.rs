//! Safe operational observations. These are not authorization or delivery receipts.
use serde::{Deserialize, Serialize};

/// Local operational state; `ready` never promises remote end-to-end reachability.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReadinessState {
    Ready,
    Degraded,
    Unavailable,
    Uncertain,
}

/// One configured component. Reasons are stable codes, never provider errors.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentReadiness {
    pub component: String,
    pub state: ReadinessState,
    pub reason: String,
    pub observed_at: i64,
    /// False distinguishes omitted optional configuration from worker failure.
    /// Older observations without this field describe configured components.
    #[serde(default = "default_configured", skip_serializing_if = "is_configured")]
    pub configured: bool,
}
fn default_configured() -> bool {
    true
}
fn is_configured(configured: &bool) -> bool {
    *configured
}

/// Authenticated health diagnostic, sampled from this running process.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Readiness {
    pub state: ReadinessState,
    pub components: Vec<ComponentReadiness>,
}
impl Readiness {
    /// Aggregate configured observations conservatively. Optional absence does
    /// not mask failures; without a configured observation readiness is unknown.
    pub fn from_components(components: Vec<ComponentReadiness>) -> Self {
        let state = [
            ReadinessState::Unavailable,
            ReadinessState::Degraded,
            ReadinessState::Uncertain,
        ]
        .into_iter()
        .find(|state| components.iter().any(|c| c.configured && c.state == *state))
        .unwrap_or(if components.iter().any(|c| c.configured) {
            ReadinessState::Ready
        } else {
            ReadinessState::Uncertain
        });
        Self { state, components }
    }
}
#[cfg(test)]
#[path = "readiness_tests.rs"]
mod tests;
