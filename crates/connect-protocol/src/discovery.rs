//! Authorized discovery and context-inspection results, independent of transport.
use crate::{Availability, ContextDelivery};
use serde::{Deserialize, Serialize};

/// A route configured by this installation. Configuration is not live reachability.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RouteCapability {
    pub id: String,
    pub transport: String,
    pub media: Vec<String>,
    pub operations: Vec<String>,
    /// `configured` or `unconfigured`; never an assertion of carrier delivery.
    pub readiness: String,
}
/// Public limits needed before sending a message, file or native audio frame.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilityLimits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_profile: Option<crate::video_profile::VideoProfile>,
    pub message_bytes: usize,
    pub artifact_bytes: u64,
    pub artifact_chunk_bytes: u32,
    pub page_size: u32,
    pub audio_frame_ms: u32,
    pub audio_codec: String,
}
/// Current caller-visible routes and permissions for a resolved participant.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Capabilities {
    /// Supported additive control profiles, not delegated permissions. Older
    /// servers omit this field and therefore do not advertise task support.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
    pub identity_id: String,
    pub conversation_id: Option<String>,
    /// Conversation appearance for participant-based routing; absent without
    /// a selected Conversation. An Identity ID is not a Participant ID.
    pub participant_id: Option<String>,
    pub routes: Vec<RouteCapability>,
    /// Only present when the caller has availability_get authority.
    pub availability: Option<Availability>,
    pub limits: CapabilityLimits,
}
/// The server-selected browser route. A client must not confuse it with its UI selection.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserBinding {
    pub conversation_id: String,
    pub participant_id: String,
    pub session_id: Option<String>,
    pub signaling_url: String,
}
/// Newest-first delivery receipts with an authenticated older-page cursor.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextDeliveryPage {
    pub deliveries: Vec<ContextDelivery>,
    pub next: Option<String>,
}
