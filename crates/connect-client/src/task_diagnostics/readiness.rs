//! Combine authenticated server health with explicitly selected local workers.
use super::{DiagnosticSection, TaskDiagnostics};
use connect_protocol::{ComponentReadiness, Readiness, ReadinessState};

/// A bounded local observation, never proof of model progress or delivery.
/// Adapter-specific state and recovery identifiers belong in the adjacent
/// protected diagnostic view; arbitrary error or session text is not a reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerCondition {
    /// No local worker installation/state was selected for inspection.
    Unconfigured,
    /// The selected worker has no saved execution state.
    NotStarted,
    /// An OS ownership lock is held; this does not attest healthy progress.
    OwnershipObserved,
    /// The selected automatic runner is no longer running.
    Stopped,
    /// Interrupted work must be reconciled using the original saved intent.
    RecoveryRequired,
    /// A bounded host turn ended; continuation requires explicit resume.
    ExplicitResumeRequired,
    /// Private installation, binding, state or lock validation failed.
    InvalidMetadata,
    /// State changed around inspection; retry rather than combine versions.
    ChangedDuringSample,
}
impl WorkerCondition {
    /// Project a trusted adapter observation into the existing readiness shape.
    /// Use a static component label; never put paths, prompts or errors in it.
    pub fn component(self, name: &'static str) -> ComponentReadiness {
        use ReadinessState::{Degraded, Unavailable, Uncertain};
        let (state, reason) = match self {
            Self::Unconfigured => (Unavailable, "not-configured"),
            Self::NotStarted => (Unavailable, "worker-not-started"),
            Self::OwnershipObserved => (Uncertain, "ownership-observed-progress-unknown"),
            Self::Stopped => (Unavailable, "worker-stopped"),
            Self::RecoveryRequired => (Unavailable, "original-worker-recovery-required"),
            Self::ExplicitResumeRequired => (Degraded, "explicit-resume-required"),
            Self::InvalidMetadata => (Unavailable, "worker-metadata-invalid"),
            Self::ChangedDuringSample => (Uncertain, "worker-changed-during-sample"),
        };
        ComponentReadiness {
            component: name.into(),
            state,
            reason: reason.into(),
            configured: self != Self::Unconfigured,
            observed_at: chrono::Utc::now().timestamp(),
        }
    }
}
impl TaskDiagnostics {
    /// Compose server and external-worker observations without altering either
    /// source. Server health cannot mask a stopped runner. An unconfigured or
    /// lock-held worker never establishes execution readiness. This performs no
    /// I/O or mutation and grants no authority to resume work.
    pub fn execution_readiness(&self, workers: &[ComponentReadiness]) -> Readiness {
        let (state, reason) = match &self.server_readiness {
            DiagnosticSection::Available(server) => (server.state, "authenticated-server-snapshot"),
            DiagnosticSection::Unavailable { .. } => {
                (ReadinessState::Uncertain, "server-readiness-unavailable")
            }
        };
        let mut components = vec![ComponentReadiness {
            component: "connect-server".into(),
            state,
            reason: reason.into(),
            configured: true,
            observed_at: chrono::Utc::now().timestamp(),
        }];
        components.extend_from_slice(workers);
        if self.changed_during_sample {
            components.push(ComponentReadiness {
                component: "task-snapshot".into(),
                state: ReadinessState::Uncertain,
                reason: "task-changed-during-sample".into(),
                configured: true,
                observed_at: chrono::Utc::now().timestamp(),
            });
        }
        let mut readiness = Readiness::from_components(components);
        if readiness.state == ReadinessState::Ready && !workers.iter().any(|w| w.configured) {
            readiness.state = ReadinessState::Uncertain;
        }
        readiness
    }
}
