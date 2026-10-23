//! Durable Session codec reconciliation metadata. None of these values grants IO.
use serde::{Deserialize, Serialize};

/// The Session control receipt records the policy decision. Follow this separate
/// operation for the actual bounded Connection reconciliation outcome.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconciliationReference {
    pub operation_id: String,
    pub record_id: String,
}

/// Aggregate counts deliberately omit physical peer and private Connection IDs.
/// Success for one target cannot imply success for the rest of the Session.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconciliationCounts {
    pub targets: u64,
    pub queued: u64,
    pub preparing: u64,
    pub updated: u64,
    pub unchanged: u64,
    pub refused: u64,
    pub unavailable: u64,
    pub cancelled: u64,
    pub superseded: u64,
    pub unknown: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconciliationOutcome {
    pub session_id: String,
    pub policy_operation_id: String,
    pub policy_revision: u64,
    pub counts: ReconciliationCounts,
}
