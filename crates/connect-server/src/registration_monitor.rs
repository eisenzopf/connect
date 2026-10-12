//! Observe the upstream registration lease; never infer readiness from process health.
use connect_protocol::ReadinessState;
use connect_service::readiness::OperationalReadiness;
use rvoip_sip::{RegistrationHandle, RegistrationInfo, RegistrationStatus, UnifiedCoordinator};
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

pub(crate) struct Monitor {
    stop: CancellationToken,
    task: Option<tokio::task::JoinHandle<anyhow::Result<()>>>,
}
impl Monitor {
    pub(crate) fn start(
        coordinator: Arc<UnifiedCoordinator>,
        handle: RegistrationHandle,
        readiness: Arc<OperationalReadiness>,
    ) -> Self {
        let stop = CancellationToken::new();
        let cancelled = stop.clone();
        readiness.set(
            "sip-registration",
            ReadinessState::Uncertain,
            "awaiting-observation",
        );
        let task = crate::worker::spawn(
            readiness.clone(),
            "sip-registration-monitor",
            stop.clone(),
            async move {
                loop {
                    let observed = tokio::select! {
                        _ = cancelled.cancelled() => break,
                        result = tokio::time::timeout(Duration::from_secs(1), coordinator.registration_info(&handle)) => result,
                    };
                    let (state, reason) = match observed {
                        Ok(Ok(info)) => classify(&info),
                        _ => (
                            ReadinessState::Uncertain,
                            "registration-observation-unavailable",
                        ),
                    };
                    readiness.set("sip-registration", state, reason);
                    tokio::select! { _ = cancelled.cancelled() => break, _ = tokio::time::sleep(Duration::from_millis(100)) => {} }
                }
                readiness.set("sip-registration", ReadinessState::Unavailable, "stopped");
                Ok(())
            },
        );
        Self {
            stop,
            task: Some(task),
        }
    }
    pub(crate) async fn shutdown(mut self) -> anyhow::Result<()> {
        self.stop.cancel();
        crate::worker::drain(
            self.task.take().into_iter().collect(),
            Duration::from_secs(2),
        )
        .await
    }
}
impl Drop for Monitor {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// Even a stale upstream Registered flag must not outlive its accepted lease.
fn classify(info: &RegistrationInfo) -> (ReadinessState, &'static str) {
    let fresh = info
        .registered_at
        .zip(info.accepted_expires_secs)
        .is_some_and(|(at, seconds)| at.elapsed() < Duration::from_secs(u64::from(seconds)));
    match info.status {
        RegistrationStatus::Registered if fresh => {
            (ReadinessState::Ready, "registration-lease-current")
        }
        RegistrationStatus::Registered => (
            ReadinessState::Unavailable,
            "registration-lease-expired-or-unknown",
        ),
        RegistrationStatus::Registering if fresh => {
            (ReadinessState::Degraded, "registration-refresh-pending")
        }
        RegistrationStatus::Registering => (
            ReadinessState::Uncertain,
            "registration-pending-without-current-lease",
        ),
        RegistrationStatus::Failed => (ReadinessState::Unavailable, "registration-failed"),
        _ => (ReadinessState::Unavailable, "registration-not-active"),
    }
}

#[cfg(test)]
mod tests;
