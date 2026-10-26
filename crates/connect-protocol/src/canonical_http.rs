//! HTTP carries the current canonical envelopes without a second command dialect.
use serde::{Deserialize, Serialize};
pub const PROFILE: &str = "connect/2";
pub const MAX_REQUEST_BYTES: usize = 65_536;
pub const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_REPLY_ENVELOPES: usize = 128;

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Problem {
    pub code: u16,
    pub category: String,
    pub reason: String,
}
impl Problem {
    /// Closed public diagnostics: never copy private storage/provider errors.
    pub fn from_error(error: &crate::Error) -> Self {
        let (code, category, reason) = match error.code.as_str() {
            "unauthorized" => (401, "auth", "authentication_failed"),
            "forbidden" => (403, "auth", "forbidden"),
            "not-found" => (404, "state", "resource_not_found"),
            "incompatible-capabilities" => (488, "media", "incompatible_capabilities"),
            "record-reference-changed" => (409, "state", "record_view_changed"),
            "record-reference-unavailable" => (503, "storage", "record_delivery_unavailable"),
            "invalid-request" | "invalid-cursor" => (400, "validation", "invalid_request"),
            "request-too-large" => (413, "validation", "request_too_large"),
            "unsupported-command" | "unsupported-feature" | "unsupported-media-profile" => {
                (501, "unsupported", "unsupported_feature")
            }
            "resync-required" => (409, "state", "resync_required"),
            "reconciliation-required" => (409, "state", "reconciliation_required"),
            "storage-unavailable" => (503, "storage", "storage_unavailable"),
            "authority-busy" | "authority-unavailable" => {
                (503, "authorization", "authority_unavailable")
            }
            "capacity-exceeded" => (429, "capacity", "capacity_exceeded"),
            "conflict" | "request-conflict" | "revision-conflict" => {
                (409, "conflict", "operation_conflict")
            }
            _ => (400, "state", "operation_rejected"),
        };
        Self {
            code,
            category: category.into(),
            reason: reason.into(),
        }
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplyBatch {
    pub profile: String,
    pub request_id: String,
    pub envelopes: Vec<serde_json::Value>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rejection {
    pub profile: String,
    pub problem: Problem,
}
