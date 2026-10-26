//! Executable conformance for the explicitly simulated outbound PCMU profile.
//! Real rvoip streams and codecs are exercised. This is neither a provider nor
//! full stock Connect application/network qualification. No Store/service access.
use connect_connector_api::{
    conformance::Check, manifest::Manifest, media::LiveMediaLifecycle, DispatchPermit,
};
use rvoip_core::{
    adapter::{ConnectionAdapter, OriginateRequest},
    error::Result,
    ids::ConnectionId,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
mod checks;

pub const PROFILE: &str = "connect-live-media-fixture-v1";
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Metrics {
    pub generated: usize,
    pub decoded: usize,
    pub last_sample: i32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Prepared,
    Active,
    Stopped,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observation {
    pub connection_id: String,
    pub state: Status,
    pub effect_attempts: usize,
}

/// Fixture instrumentation only: never use this factory as production admission.
/// Implement in an independent package alongside its own simulated transport.
/// The harness supplies custody; request must preserve that exact permit and ID.
#[async_trait::async_trait]
pub trait Fixture: Send + Sync + 'static {
    type Adapter: ConnectionAdapter + LiveMediaLifecycle + 'static;
    type Backend: Default + Send + Sync + 'static;
    const QUEUE_CAPACITY: usize;
    async fn open(backend: Arc<Self::Backend>) -> Result<Arc<Self::Adapter>>;
    fn manifest() -> Manifest;
    fn request(
        adapter: &Self::Adapter,
        operation: &str,
        permit: DispatchPermit,
    ) -> OriginateRequest;
    fn observe(
        adapter: &Self::Adapter,
        operation: &str,
        permit: &DispatchPermit,
    ) -> Result<Observation>;
    fn metrics(adapter: &Self::Adapter, connection: &ConnectionId) -> Result<Metrics>;
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Report {
    pub profile: String,
    pub manifest: Manifest,
    pub checks: Vec<Check>,
    pub unsupported: Vec<String>,
}
impl Report {
    pub fn passed(&self) -> bool {
        self.profile == PROFILE
            && self.checks.len() == CHECK_NAMES.len()
            && self
                .checks
                .iter()
                .zip(CHECK_NAMES)
                .all(|(check, name)| check.passed && check.name == *name)
    }
}
pub const CHECK_NAMES: &[&str] = &[
    "staged_admission_uses_current_effect_custody_and_exact_original_intent",
    "actual_duplex_pcmu_decodes_speaker_markers_and_owned_end_stops_both_queues",
    "stale_buffered_generation_is_discarded_at_the_actual_decode",
    "revocation_expiry_and_backpressure_fence_media_instead_of_inventing_health",
    "closed_terminal_queue_uses_owned_fallback_and_restart_keeps_original_effect",
    "capabilities_and_receipt_retention_refuse_unsupported_or_excess_work_before_effect",
    "cancelling_drain_retains_the_original_join_for_cleanup_retry",
    "guarded_core_registers_atomic_outbound_fixture_and_host_cancel_is_terminal",
];
/// Each check opens its own retained backend, invokes real transport behavior,
/// and has a bounded deadline. No caller-supplied pass/fail vector is accepted.
pub async fn run<F: Fixture>() -> Report {
    let mut report = Report {
        profile: PROFILE.into(),
        manifest: F::manifest(),
        checks: vec![],
        unsupported: [
            "video",
            "inbound",
            "network",
            "live-provider",
            "model-response",
            "process-crash-persistence",
            "stock-application-journey",
        ]
        .map(str::to_owned)
        .into(),
    };
    {
        let mut check = tokio::spawn(
            checks::staged_admission_uses_current_effect_custody_and_exact_original_intent::<F>(),
        );
        let passed = match tokio::time::timeout(std::time::Duration::from_secs(5), &mut check).await
        {
            Ok(result) => result.is_ok(),
            Err(_) => {
                check.abort();
                let _ = check.await;
                false
            }
        };
        report.checks.push(Check {
            name: "staged_admission_uses_current_effect_custody_and_exact_original_intent".into(),
            passed,
        });
    }
    {
        let mut check = tokio::spawn(
            checks::actual_duplex_pcmu_decodes_speaker_markers_and_owned_end_stops_both_queues::<F>(
            ),
        );
        let passed = match tokio::time::timeout(std::time::Duration::from_secs(5), &mut check).await
        {
            Ok(result) => result.is_ok(),
            Err(_) => {
                check.abort();
                let _ = check.await;
                false
            }
        };
        report.checks.push(Check {
            name: "actual_duplex_pcmu_decodes_speaker_markers_and_owned_end_stops_both_queues"
                .into(),
            passed,
        });
    }
    {
        let mut check = tokio::spawn(
            checks::stale_buffered_generation_is_discarded_at_the_actual_decode::<F>(),
        );
        let passed = match tokio::time::timeout(std::time::Duration::from_secs(5), &mut check).await
        {
            Ok(result) => result.is_ok(),
            Err(_) => {
                check.abort();
                let _ = check.await;
                false
            }
        };
        report.checks.push(Check {
            name: "stale_buffered_generation_is_discarded_at_the_actual_decode".into(),
            passed,
        });
    }
    {
        let mut check = tokio::spawn(
            checks::revocation_expiry_and_backpressure_fence_media_instead_of_inventing_health::<F>(
            ),
        );
        let passed = match tokio::time::timeout(std::time::Duration::from_secs(5), &mut check).await
        {
            Ok(result) => result.is_ok(),
            Err(_) => {
                check.abort();
                let _ = check.await;
                false
            }
        };
        report.checks.push(Check {
            name: "revocation_expiry_and_backpressure_fence_media_instead_of_inventing_health"
                .into(),
            passed,
        });
    }
    {
        let mut check = tokio::spawn(
            checks::closed_terminal_queue_uses_owned_fallback_and_restart_keeps_original_effect::<F>(
            ),
        );
        let passed = match tokio::time::timeout(std::time::Duration::from_secs(5), &mut check).await
        {
            Ok(result) => result.is_ok(),
            Err(_) => {
                check.abort();
                let _ = check.await;
                false
            }
        };
        report.checks.push(Check {
            name: "closed_terminal_queue_uses_owned_fallback_and_restart_keeps_original_effect"
                .into(),
            passed,
        });
    }
    {
        let mut check = tokio::spawn(checks::capabilities_and_receipt_retention_refuse_unsupported_or_excess_work_before_effect::<F>());
        let passed = match tokio::time::timeout(std::time::Duration::from_secs(5), &mut check).await
        {
            Ok(result) => result.is_ok(),
            Err(_) => {
                check.abort();
                let _ = check.await;
                false
            }
        };
        report.checks.push(Check {
            name:
                "capabilities_and_receipt_retention_refuse_unsupported_or_excess_work_before_effect"
                    .into(),
            passed,
        });
    }
    {
        let mut check = tokio::spawn(
            checks::cancelling_drain_retains_the_original_join_for_cleanup_retry::<F>(),
        );
        let passed = match tokio::time::timeout(std::time::Duration::from_secs(5), &mut check).await
        {
            Ok(result) => result.is_ok(),
            Err(_) => {
                check.abort();
                let _ = check.await;
                false
            }
        };
        report.checks.push(Check {
            name: "cancelling_drain_retains_the_original_join_for_cleanup_retry".into(),
            passed,
        });
    }
    {
        let mut check = tokio::spawn(
            checks::guarded_core_registers_atomic_outbound_fixture_and_host_cancel_is_terminal::<F>(
            ),
        );
        let passed = match tokio::time::timeout(std::time::Duration::from_secs(5), &mut check).await
        {
            Ok(result) => result.is_ok(),
            Err(_) => {
                check.abort();
                let _ = check.await;
                false
            }
        };
        report.checks.push(Check {
            name: "guarded_core_registers_atomic_outbound_fixture_and_host_cancel_is_terminal"
                .into(),
            passed,
        });
    }
    report
}
