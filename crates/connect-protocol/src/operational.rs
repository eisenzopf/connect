//! Administrator-only, bounded numeric observations. No IDs, bodies or grants.
use serde::{Deserialize, Serialize};
pub const MAX_OPERATIONAL_COUNTER: u64 = 9_007_199_254_740_991;
pub const MAX_OPERATIONAL_POOLS: usize = 32;

/// Metadata counts from the operation ledger, not running worker counts.
/// Interrupted and unknown work remains unsettled even without a live worker.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationCounts {
    pub retained: u64,
    pub accepted: u64,
    pub preparing: u64,
    pub ready: u64,
    pub committing: u64,
    pub interrupted: u64,
    pub unknown: u64,
}

/// All configured values are explicit, including values omitted in TOML.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationalLimits {
    pub message_bytes: u64,
    pub page_size: u32,
    pub journal_events: u64,
    pub operation_receipts: u64,
    pub message_storage_bytes: u64,
    pub participants: u64,
    pub wait_ms: u32,
    pub request_timeout_ms: u64,
    pub connections: u64,
    pub sessions: u64,
    pub artifact_file_bytes: u64,
    pub artifact_bytes: u64,
    pub artifact_chunk_bytes: u32,
}
impl From<&crate::Limits> for OperationalLimits {
    fn from(v: &crate::Limits) -> Self {
        Self {
            message_bytes: v.message_bytes as u64,
            page_size: v.page_size,
            journal_events: v.journal_events,
            operation_receipts: v.operation_receipts,
            message_storage_bytes: v.message_storage_bytes,
            participants: v.participants as u64,
            wait_ms: v.wait_ms,
            request_timeout_ms: v.request_timeout_ms,
            connections: v.connections as u64,
            sessions: v.sessions,
            artifact_file_bytes: v.artifact_file_bytes,
            artifact_bytes: v.artifact_bytes,
            artifact_chunk_bytes: v.artifact_chunk_bytes,
        }
    }
}

/// Each variant names one real owner/budget. Shared budgets are not additive.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResourcePoolKind {
    QuicIngress,
    NativeQuic,
    #[serde(rename = "webtransport")]
    WebTransport,
    #[serde(rename = "websocket")]
    WebSocket,
    HttpConnections,
    HttpRequests,
    HttpCallbackConnections,
    HttpCallbackRequests,
    HttpsApplicationConnections,
    HttpsApplicationRequests,
    HttpsApplicationHandshakes,
    HttpsAdministrationConnections,
    HttpsAdministrationRequests,
    HttpsAdministrationHandshakes,
    HttpsCallbacksConnections,
    HttpsCallbacksRequests,
    HttpsCallbacksHandshakes,
    HttpsHealthConnections,
    HttpsHealthRequests,
    HttpsHealthHandshakes,
    RecordHttpsConnections,
    RecordHttpsRequests,
    RecordHttpsHandshakes,
    McpRequests,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResourceSampleState {
    Available,
    Busy,
    Retired,
}

/// Cumulative counters saturate at MAX_OPERATIONAL_COUNTER on the wire.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceUsage {
    pub used: u32,
    pub limit: u32,
    pub accepted: u64,
    pub rejected: u64,
    pub draining: bool,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourcePool {
    pub kind: ResourcePoolKind,
    pub state: ResourceSampleState,
    /// Busy/retired observations are unavailable, never a zero-resource claim.
    pub usage: Option<ResourceUsage>,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeResources {
    pub physical_owners: u32,
    pub physical_capacity: u32,
    /// Owned JoinSet entries, including completed tasks awaiting join.
    pub owned_tasks: u32,
}
/// Independently sampled observations; this is not an atomic global snapshot.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationalHealth {
    pub observed_at: i64,
    pub limits: OperationalLimits,
    pub operations: OperationCounts,
    /// Null means not configured or temporarily unavailable, never zero usage.
    pub runtime: Option<RuntimeResources>,
    /// Null means registry unavailable; an empty list means no registered probes.
    pub resource_pools: Option<Vec<ResourcePool>>,
}

#[cfg(test)]
mod tests {
    #[test]
    fn operational_fixture_is_closed_and_preserves_unavailable_observations() {
        let bytes = include_str!("../../../contracts/fixtures/operational-health-v1.json");
        let sample: super::OperationalHealth =
            serde_json::from_str(bytes).expect("shared diagnostic fixture");
        assert_eq!(
            sample.resource_pools.as_ref().expect("pools")[1].state,
            super::ResourceSampleState::Busy
        );
        assert!(sample.resource_pools.as_ref().expect("pools")[1]
            .usage
            .is_none());
        let mut value = serde_json::to_value(sample).expect("encode");
        value["credential"] = serde_json::json!("must-not-enter-diagnostics");
        assert!(serde_json::from_value::<super::OperationalHealth>(value).is_err());
    }
}

impl crate::Limits {
    /// Preserve exact configured values in all three public SDKs and SQLite.
    /// Zero remains an explicit refusing quota where the owning API permits it.
    pub fn validate(&self) -> crate::Result<()> {
        let limits = OperationalLimits::from(self);
        if [
            limits.message_bytes,
            limits.journal_events,
            limits.operation_receipts,
            limits.message_storage_bytes,
            limits.participants,
            limits.request_timeout_ms,
            limits.connections,
            limits.sessions,
            limits.artifact_file_bytes,
            limits.artifact_bytes,
        ]
        .into_iter()
        .any(|v| v > MAX_OPERATIONAL_COUNTER)
            || self.connections > u32::MAX as usize
        {
            return Err(crate::Error::new(
                "invalid-config",
                "Configured limit exceeds supported numeric bounds",
            ));
        }
        Ok(())
    }
}
