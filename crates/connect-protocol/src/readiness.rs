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
}

/// Authenticated health diagnostic, sampled from this running process.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Readiness {
    pub state: ReadinessState,
    pub components: Vec<ComponentReadiness>,
}
