//! Process-local operational state, separate from the authoritative journal.
use connect_protocol::{ComponentReadiness, Readiness, ReadinessState};
use std::{collections::BTreeMap, sync::Mutex};
use tokio_util::sync::CancellationToken;

/// Shared by runtime supervisors and authenticated health responses.
#[derive(Default)]
pub struct OperationalReadiness {
    components: Mutex<BTreeMap<&'static str, ComponentReadiness>>,
    fatal: CancellationToken,
}
impl OperationalReadiness {
    /// Publish a bounded, static component/reason pair without private data.
    pub fn set(&self, component: &'static str, state: ReadinessState, reason: &'static str) {
        self.components
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(
                component,
                ComponentReadiness {
                    component: component.into(),
                    state,
                    reason: reason.into(),
                    observed_at: chrono::Utc::now().timestamp(),
                    configured: true,
                },
            );
    }
    /// An optional component was not enabled. Expose that fact without failing
    /// configured components or claiming a remote worker is running.
    pub fn unconfigured(&self, component: &'static str) {
        self.components
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(
                component,
                ComponentReadiness {
                    component: component.into(),
                    state: ReadinessState::Unavailable,
                    reason: "not-configured".into(),
                    observed_at: chrono::Utc::now().timestamp(),
                    configured: false,
                },
            );
    }
    /// A worker cannot continue safely. Wake supervision without replaying effects.
    pub fn fail(&self, component: &'static str) {
        self.set(component, ReadinessState::Unavailable, "worker-exited");
        self.fatal.cancel();
    }
    /// Wait without a lost-wakeup race, including failures before subscription.
    pub async fn failed(&self) {
        self.fatal.cancelled().await;
    }

    /// Optional unconfigured components are explicit and do not degrade the
    /// aggregate. No configured observations still means uncertain, not ready.
    pub fn snapshot(&self) -> Readiness {
        let components: Vec<_> = self
            .components
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .cloned()
            .collect();
        Readiness::from_components(components)
    }
}
#[cfg(test)]
#[path = "readiness_tests.rs"]
mod tests;
