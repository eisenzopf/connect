//! Per-recipient, per-channel evidence. Client reports never establish human action.
use serde::{Deserialize, Serialize};
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryState {
    Queued,
    Accepted,
    Delivered,
    Read,
    Failed,
    Unknown,
}
impl DeliveryState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Accepted => "accepted",
            Self::Delivered => "delivered",
            Self::Read => "read",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
        }
    }
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientReceiptState {
    Delivered,
    Read,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiptPreferences {
    pub conversation_id: String,
    pub participant_id: String,
    pub membership_epoch: u64,
    pub delivery: bool,
    pub read: bool,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipientReceipt {
    pub conversation_id: String,
    pub message_id: String,
    pub participant_id: String,
    pub membership_epoch: u64,
    /// Native and provider observations never upgrade one another.
    pub channel: String,
    pub delivery_id: String,
    pub state: DeliveryState,
    /// journal, client_reported, provider_observation, or local_attempt.
    pub provenance: String,
    pub observed_at: i64,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptPage {
    pub receipts: Vec<RecipientReceipt>,
    pub next: Option<String>,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptAcknowledgement {
    pub conversation_id: String,
    pub message_id: String,
    pub participant_id: String,
    pub membership_epoch: u64,
    /// None when the recipient's preferences suppress the report.
    pub recorded: Option<DeliveryState>,
}
