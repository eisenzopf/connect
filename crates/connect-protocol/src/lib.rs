//! Transport-independent Connect application contract carried by UCTP envelopes.

mod discovery;
pub mod media_profile;
pub use discovery::*;

use serde::{Deserialize, Serialize};

pub const PROFILE: &str = "connect-control/1";
pub const REQUIRED_SCOPE: &str = "connect:control";
pub const REQUEST_TYPE: &str = "connect.request";
pub const RESULT_TYPE: &str = "connect.result";
/// Leave framing headroom for result metadata/cursors below UCTP's 1 MiB cap.
pub const MAX_PAGE_BYTES: usize = 512 * 1024;

/// The transport envelope ID correlates one exchange. `request_id` identifies a
/// logical mutation across retries, transports, and server restarts.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Request {
    pub profile: String,
    #[serde(flatten)]
    pub operation: Operation,
}

impl Request {
    pub fn new(operation: Operation) -> Self {
        Self {
            profile: PROFILE.into(),
            operation,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Resolve {
        address: String,
    },
    Capabilities {
        address: String,
        conversation_id: Option<String>,
    },
    BrowserBinding {},
    ContextDeliveries {
        conversation_id: String,
        before: Option<String>,
        limit: u32,
    },
    Create {
        request_id: String,
        participants: Vec<String>,
    },
    Send {
        request_id: String,
        conversation_id: String,
        body: String,
    },
    History {
        conversation_id: String,
        after: Option<String>,
        limit: u32,
    },
    Inbox {
        cursor: Option<String>,
        limit: u32,
        wait_ms: u32,
    },
    RequestStatus {
        request_id: String,
    },
    Close {
        request_id: String,
        conversation_id: String,
    },
    ProviderStatus {
        conversation_id: String,
    },
    CallStart {
        request_id: String,
        conversation_id: String,
        route_id: String,
        destination: String,
    },
    CallStatus {
        conversation_id: String,
        call_id: String,
    },
    CallEnd {
        request_id: String,
        conversation_id: String,
        call_id: String,
    },
    SessionInvite {
        request_id: String,
        conversation_id: String,
        participant_id: String,
        media: Vec<String>,
    },
    SessionAccept {
        request_id: String,
        conversation_id: String,
        session_id: String,
    },
    SessionReject {
        request_id: String,
        conversation_id: String,
        session_id: String,
        reason: Option<String>,
    },
    SessionCancel {
        request_id: String,
        conversation_id: String,
        session_id: String,
    },
    SessionEnd {
        request_id: String,
        conversation_id: String,
        session_id: String,
        reason: Option<String>,
    },
    SessionGet {
        conversation_id: String,
        session_id: String,
    },
    SipDial {
        request_id: String,
        conversation_id: String,
        session_id: String,
        destination: String,
    },
    SipStatus {
        conversation_id: String,
        sip_leg_id: String,
    },
    SipEnd {
        request_id: String,
        conversation_id: String,
        sip_leg_id: String,
    },
    ContextView {
        conversation_id: String,
        limit: u32,
    },
    AvailabilitySet {
        request_id: String,
        state: AvailabilityState,
        preferred_route: Option<String>,
        expires_at: i64,
    },
    AvailabilityGet {
        identity_id: String,
    },
    ArtifactBegin {
        request_id: String,
        conversation_id: String,
        name: String,
        media_type: String,
        size: u64,
        sha256: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expires_at: Option<i64>,
    },
    ArtifactAppend {
        request_id: String,
        conversation_id: String,
        artifact_id: String,
        offset: u64,
        data: String,
    },
    ArtifactFinish {
        request_id: String,
        conversation_id: String,
        artifact_id: String,
    },
    ArtifactGet {
        conversation_id: String,
        artifact_id: String,
        offset: u64,
        limit: u32,
    },
    ArtifactDelete {
        request_id: String,
        conversation_id: String,
        artifact_id: String,
    },
    Health {},
}

impl Operation {
    /// Canonical conversation routing metadata for UCTP envelopes.
    pub fn conversation_id(&self) -> Option<&str> {
        match self {
            Self::Capabilities {
                conversation_id, ..
            } => conversation_id.as_deref(),
            Self::ContextDeliveries {
                conversation_id, ..
            } => Some(conversation_id),
            Self::Send {
                conversation_id, ..
            }
            | Self::History {
                conversation_id, ..
            }
            | Self::ProviderStatus { conversation_id }
            | Self::CallStart {
                conversation_id, ..
            }
            | Self::CallStatus {
                conversation_id, ..
            }
            | Self::CallEnd {
                conversation_id, ..
            }
            | Self::SessionInvite {
                conversation_id, ..
            }
            | Self::SessionAccept {
                conversation_id, ..
            }
            | Self::SessionReject {
                conversation_id, ..
            }
            | Self::SessionCancel {
                conversation_id, ..
            }
            | Self::SessionEnd {
                conversation_id, ..
            }
            | Self::SessionGet {
                conversation_id, ..
            }
            | Self::SipDial {
                conversation_id, ..
            }
            | Self::SipStatus {
                conversation_id, ..
            }
            | Self::SipEnd {
                conversation_id, ..
            }
            | Self::ContextView {
                conversation_id, ..
            }
            | Self::ArtifactBegin {
                conversation_id, ..
            }
            | Self::ArtifactAppend {
                conversation_id, ..
            }
            | Self::ArtifactFinish {
                conversation_id, ..
            }
            | Self::ArtifactGet {
                conversation_id, ..
            }
            | Self::ArtifactDelete {
                conversation_id, ..
            }
            | Self::Close {
                conversation_id, ..
            } => Some(conversation_id),
            _ => None,
        }
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::Resolve { .. } => "resolve",
            Self::Capabilities { .. } => "capabilities",
            Self::BrowserBinding {} => "browser_binding",
            Self::ContextDeliveries { .. } => "context_deliveries",
            Self::Create { .. } => "create",
            Self::Send { .. } => "send",
            Self::History { .. } => "history",
            Self::Inbox { .. } => "inbox",
            Self::RequestStatus { .. } => "request_status",
            Self::Close { .. } => "close",
            Self::ProviderStatus { .. } => "provider_status",
            Self::CallStart { .. } => "call_start",
            Self::CallStatus { .. } => "call_status",
            Self::CallEnd { .. } => "call_end",
            Self::SessionInvite { .. } => "session_invite",
            Self::SessionAccept { .. } => "session_accept",
            Self::SessionReject { .. } => "session_reject",
            Self::SessionCancel { .. } => "session_cancel",
            Self::SessionEnd { .. } => "session_end",
            Self::SessionGet { .. } => "session_get",
            Self::SipDial { .. } => "sip_dial",
            Self::SipStatus { .. } => "sip_status",
            Self::SipEnd { .. } => "sip_end",
            Self::ContextView { .. } => "context_view",
            Self::AvailabilitySet { .. } => "availability_set",
            Self::AvailabilityGet { .. } => "availability_get",
            Self::ArtifactBegin { .. } => "artifact_begin",
            Self::ArtifactAppend { .. } => "artifact_append",
            Self::ArtifactFinish { .. } => "artifact_finish",
            Self::ArtifactGet { .. } => "artifact_get",
            Self::ArtifactDelete { .. } => "artifact_delete",
            Self::Health {} => "health",
        }
    }
    /// Additive read APIs use the existing authority for the information they expose.
    pub fn required_scope(&self) -> &'static str {
        match self {
            Self::Capabilities { .. } => "resolve",
            Self::BrowserBinding {} => "session_get",
            Self::ContextDeliveries { .. } => "context_view",
            _ => self.name(),
        }
    }
    pub fn request_id(&self) -> Option<&str> {
        match self {
            Self::Create { request_id, .. }
            | Self::Send { request_id, .. }
            | Self::CallStart { request_id, .. }
            | Self::CallEnd { request_id, .. }
            | Self::SessionInvite { request_id, .. }
            | Self::SessionAccept { request_id, .. }
            | Self::SessionReject { request_id, .. }
            | Self::SessionCancel { request_id, .. }
            | Self::SessionEnd { request_id, .. }
            | Self::SipDial { request_id, .. }
            | Self::SipEnd { request_id, .. }
            | Self::AvailabilitySet { request_id, .. }
            | Self::ArtifactBegin { request_id, .. }
            | Self::ArtifactAppend { request_id, .. }
            | Self::ArtifactFinish { request_id, .. }
            | Self::ArtifactDelete { request_id, .. }
            | Self::Close { request_id, .. } => Some(request_id),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Identity {
    pub id: String,
    pub alias: String,
    /// Durable declared capability, independent of whether a client is online.
    pub capabilities: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Conversation {
    pub id: String,
    pub participants: Vec<String>,
    pub closed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Message {
    pub id: String,
    pub conversation_id: String,
    pub sender: String,
    pub body: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Event {
    pub id: String,
    pub kind: String,
    pub conversation_id: String,
    pub message: Option<Message>,
    pub cursor: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call: Option<Call>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<Session>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<Artifact>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AvailabilityState {
    Available,
    Unavailable,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Availability {
    pub identity_id: String,
    /// `unknown` is returned once a stored observation expires.
    pub state: String,
    pub preferred_route: Option<String>,
    pub expires_at: Option<i64>,
    pub observed_at: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    pub conversation_id: String,
    pub initiator_id: String,
    pub participant_id: String,
    pub state: String,
    pub media: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub ended_reason: Option<String>,
}

/// Durable intent and observed state for one SIP connection in a Session.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SipLeg {
    pub id: String,
    pub conversation_id: String,
    pub session_id: String,
    pub destination: String,
    pub state: String,
    pub connection_id: Option<String>,
    pub ended_reason: Option<String>,
    pub end_requested: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextItem {
    pub sequence: u64,
    pub kind: String,
    pub source_id: String,
    pub actor_id: String,
    pub body: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextDelivery {
    pub id: String,
    pub session_id: Option<String>,
    pub provider: String,
    pub watermark: u64,
    pub state: String,
    pub provider_ref: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextView {
    pub conversation_id: String,
    pub watermark: u64,
    pub items: Vec<ContextItem>,
    pub omitted: u64,
    #[serde(default)]
    pub deliveries: Vec<ContextDelivery>,
    /// Older delivery receipts excluded from this recent snapshot. Page with context_deliveries.
    #[serde(default)]
    pub deliveries_omitted: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Artifact {
    pub id: String,
    pub conversation_id: String,
    pub owner_id: String,
    pub name: String,
    pub media_type: String,
    pub size: u64,
    pub received: u64,
    pub sha256: String,
    pub state: String,
    pub created_at: i64,
    #[serde(default)]
    pub expires_at: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactChunk {
    pub artifact: Artifact,
    pub offset: u64,
    /// Standard padded base64. It is for bounded reliable artifact chunks, never audio.
    pub data: String,
    pub eof: bool,
}

/// Durable call receipt. Acceptance does not imply ringing or completed audio.
fn outbound_direction() -> String {
    "outbound".into()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Call {
    /// An accepted end intent remains visible until provider termination is known.
    #[serde(default)]
    pub end_requested: bool,
    #[serde(default = "outbound_direction")]
    pub direction: String,
    pub id: String,
    pub conversation_id: String,
    pub route_id: String,
    pub destination: String,
    pub state: String,
    pub provider_id: Option<String>,
    pub ended_reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderDelivery {
    pub job_id: String,
    pub message_id: String,
    pub connector: String,
    pub state: String,
    pub provider_id: Option<String>,
    /// Dispatch eligibility decision, if this delivery uses SMS.
    #[serde(default)]
    pub sms_decision: Option<String>,
    /// Opaque reference to the reviewed enrollment evidence used at dispatch.
    #[serde(default)]
    pub sms_grant_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum Outcome {
    Capabilities(Capabilities),
    BrowserBinding(BrowserBinding),
    ContextDeliveries(ContextDeliveryPage),
    Call(Call),
    Session(Session),
    SipLeg(SipLeg),
    Availability(Availability),
    ContextView(ContextView),
    Artifact(Artifact),
    ArtifactChunk(ArtifactChunk),
    Identity(Identity),
    Conversation(Conversation),
    Accepted {
        request_id: String,
        message_id: String,
        conversation_id: String,
    },
    History {
        messages: Vec<Message>,
        next: Option<String>,
    },
    Inbox {
        events: Vec<Event>,
        cursor: Option<String>,
    },
    RequestStatus {
        state: String,
        outcome: Option<Box<Outcome>>,
    },
    ProviderStatus {
        deliveries: Vec<ProviderDelivery>,
    },
    Health {
        installation_id: String,
        schema: u32,
        profile: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, thiserror::Error, PartialEq, Eq)]
#[error("{code}: {message}")]
pub struct Error {
    pub code: String,
    pub message: String,
}

impl Error {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
    pub fn unauthorized() -> Self {
        Self::new("unauthorized", "Credential is invalid, expired, or revoked")
    }
    pub fn forbidden() -> Self {
        Self::new("forbidden", "Operation or resource is not permitted")
    }
    pub fn storage() -> Self {
        Self::new("storage-unavailable", "Durable storage is unavailable")
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub profile: String,
    pub result: Result<Outcome>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub message_bytes: usize,
    pub page_size: u32,
    pub journal_events: u64,
    #[serde(
        default = "default_receipt_limit",
        skip_serializing_if = "is_default_receipt_limit"
    )]
    pub operation_receipts: u64,
    pub message_storage_bytes: u64,
    pub participants: usize,
    pub wait_ms: u32,
    pub request_timeout_ms: u64,
    pub connections: usize,
    #[serde(
        default = "default_sessions",
        skip_serializing_if = "is_default_sessions"
    )]
    pub sessions: u64,
    #[serde(
        default = "default_artifact_file_bytes",
        skip_serializing_if = "is_default_artifact_file_bytes"
    )]
    pub artifact_file_bytes: u64,
    #[serde(
        default = "default_artifact_bytes",
        skip_serializing_if = "is_default_artifact_bytes"
    )]
    pub artifact_bytes: u64,
    #[serde(
        default = "default_artifact_chunk_bytes",
        skip_serializing_if = "is_default_artifact_chunk_bytes"
    )]
    pub artifact_chunk_bytes: u32,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            message_bytes: 16_384,
            page_size: 100,
            journal_events: 100_000,
            operation_receipts: default_receipt_limit(),
            message_storage_bytes: 64 * 1024 * 1024,
            participants: 8,
            wait_ms: 5000,
            request_timeout_ms: 10_000,
            connections: 64,
            sessions: default_sessions(),
            artifact_file_bytes: default_artifact_file_bytes(),
            artifact_bytes: default_artifact_bytes(),
            artifact_chunk_bytes: default_artifact_chunk_bytes(),
        }
    }
}
fn default_sessions() -> u64 {
    10_000
}
fn is_default_sessions(value: &u64) -> bool {
    *value == default_sessions()
}
fn default_artifact_bytes() -> u64 {
    256 * 1024 * 1024
}
fn default_artifact_file_bytes() -> u64 {
    16 * 1024 * 1024
}
fn is_default_artifact_file_bytes(value: &u64) -> bool {
    *value == default_artifact_file_bytes()
}
fn is_default_artifact_bytes(value: &u64) -> bool {
    *value == default_artifact_bytes()
}
fn default_artifact_chunk_bytes() -> u32 {
    32 * 1024
}
fn is_default_artifact_chunk_bytes(value: &u32) -> bool {
    *value == default_artifact_chunk_bytes()
}
fn default_receipt_limit() -> u64 {
    100_000
}
fn is_default_receipt_limit(value: &u64) -> bool {
    *value == default_receipt_limit()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)] // Panics are deliberate assertions in these fixtures.
mod tests {
    use super::*;
    #[test]
    fn profile_payload_roundtrips_and_rejects_spoofing() {
        let request = Request::new(Operation::Send {
            request_id: "logical-1".into(),
            conversation_id: "conversation-1".into(),
            body: "hello".into(),
        });
        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(request, serde_json::from_value(value.clone()).unwrap());
        let mut spoofed = value;
        spoofed["sender"] = serde_json::json!("other-actor");
        assert!(serde_json::from_value::<Request>(spoofed).is_err());
    }

    #[test]
    fn operations_reject_claimed_sender_and_unknown_fields() {
        assert!(serde_json::from_str::<Operation>(r#"{"op":"send","request_id":"r","conversation_id":"c","body":"hello","sender":"other"}"#).is_err());
        for operation in [
            Operation::Health {},
            Operation::Resolve {
                address: "connect:reviews@local".into(),
            },
            Operation::Inbox {
                cursor: None,
                limit: 5,
                wait_ms: 0,
            },
        ] {
            let json = serde_json::to_string(&operation).unwrap();
            assert_eq!(operation, serde_json::from_str(&json).unwrap());
        }
    }
}
