//! Transport-independent Connect application contract carried by UCTP envelopes.
pub mod application_api;
pub mod canonical_http;
pub mod connection_control;
pub mod conversations;
pub mod native_capabilities;
pub mod native_data;
pub mod native_dtmf;
pub mod native_gates;
pub mod native_network;
pub mod native_streams;
pub mod received_dtmf;
pub mod record_mutations;
pub use conversations::Conversation;

pub mod attachments;
pub mod contacts;
mod discovery;
pub mod events;
pub mod incoming_contacts;
pub mod media_profile;
pub mod membership;
pub mod messages;
pub mod provisioning;
pub mod receipts;
pub mod routing;
pub mod session_codecs;
pub mod session_control;
pub mod session_decisions;
pub mod tasks;
pub mod transitions;
pub mod user_delegation;
pub mod video_profile;
pub use discovery::*;

mod media_diagnostics;
pub use media_diagnostics::{
    MediaDiagnostics, MediaForwardObservation, MediaQueueObservation, QuicSendBufferObservation,
    QuicTransmissionObservation, WebRtcVideoInputObservation,
};
mod readiness;
pub use readiness::{ComponentReadiness, Readiness, ReadinessState};

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

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    ConnectionPrepare {
        request: connection_control::PrepareRequest,
    },
    SessionControl {
        request: session_control::Request,
    },
    SessionDecision {
        request: session_decisions::Request,
    },
    SessionObservations {
        conversation_id: String,
        session_id: String,
        query: events::ObservationQuery,
    },
    ConversationSnapshot {
        conversation_id: String,
        query: events::SnapshotQuery,
    },
    ConversationEvents {
        conversation_id: String,
        query: events::EventQuery,
    },
    Transition {
        request: Box<transitions::TransitionRequest>,
    },
    DurableOperation {
        request: transitions::OperationAction,
    },
    ParticipantRoute {
        request: routing::ParticipantRouteRequest,
    },
    /// Native authenticated pairing verifies an identity only, never SMS consent.
    ContactPairingStart {
        request_id: String,
        peer_identity_id: String,
        expires_at: i64,
    },
    ContactPairingComplete {
        request_id: String,
        pairing_id: String,
        challenge: String,
    },
    ContactPairingRevoke {
        request_id: String,
        pairing_id: String,
    },
    ContactState {
        peer_identity_id: String,
    },
    /// Additive task policy carried by the existing control application. The
    /// authenticated principal supplies ownership, never the request payload.
    Task {
        request: Box<tasks::TaskRequest>,
    },
    Resolve {
        address: String,
    },
    Capabilities {
        address: String,
        conversation_id: Option<String>,
    },
    BrowserBinding {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        conversation_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
    },
    ContextDeliveries {
        conversation_id: String,
        before: Option<String>,
        limit: u32,
    },
    Create {
        request_id: String,
        request: conversations::CreateRequest,
    },
    Membership {
        request: membership::MembershipRequest,
    },
    MembershipSnapshot {
        conversation_id: String,
    },
    MembershipInvitations {
        after: Option<String>,
        limit: u32,
    },
    Conversations {
        query: conversations::ListQuery,
    },
    Send {
        request_id: String,
        conversation_id: String,
        message: messages::MessageDraft,
    },
    MessageReceipts {
        conversation_id: String,
        message_id: String,
        after: Option<String>,
        limit: u32,
    },
    MessageReceipt {
        request_id: String,
        conversation_id: String,
        message_id: String,
        state: receipts::ClientReceiptState,
    },
    ReceiptPreferencesGet {
        conversation_id: String,
    },
    ReceiptPreferencesSet {
        request_id: String,
        conversation_id: String,
        delivery: bool,
        read: bool,
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
        options: conversations::CloseOptions,
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
        to: messages::MessageAudience,
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
    ArtifactInspect {
        conversation_id: String,
        artifact_id: String,
    },
    ArtifactUploadLink {
        conversation_id: String,
        artifact_id: String,
        expires_in: u32,
    },
    ArtifactDownloadLink {
        conversation_id: String,
        artifact_id: String,
        message_id: String,
        expires_in: u32,
    },
    ArtifactLinkGet {
        conversation_id: String,
        artifact_id: String,
        link: String,
        offset: u64,
        limit: u32,
    },
    ArtifactLinkAppend {
        request_id: String,
        conversation_id: String,
        artifact_id: String,
        link: String,
        offset: u64,
        data: String,
    },
    ArtifactDelete {
        request_id: String,
        conversation_id: String,
        artifact_id: String,
    },
    Health {},
}

impl std::fmt::Debug for Operation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Operation")
            .field("op", &self.name())
            .field("request_id", &self.request_id())
            .finish_non_exhaustive()
    }
}

impl Operation {
    /// Canonical conversation routing metadata for UCTP envelopes.
    pub fn conversation_id(&self) -> Option<&str> {
        match self {
            Self::ConnectionPrepare { request } => Some(&request.intent.conversation_id),
            Self::SessionControl { request } => Some(&request.conversation_id),
            Self::SessionDecision { request } => Some(&request.conversation_id),
            Self::Membership { request } => Some(&request.conversation_id),
            Self::MembershipSnapshot { conversation_id } => Some(conversation_id),
            Self::SessionObservations {
                conversation_id, ..
            } => Some(conversation_id),
            Self::ConversationSnapshot {
                conversation_id, ..
            }
            | Self::ConversationEvents {
                conversation_id, ..
            } => Some(conversation_id),
            Self::Transition { request } => Some(&request.conversation_id),
            Self::MessageReceipts {
                conversation_id, ..
            }
            | Self::MessageReceipt {
                conversation_id, ..
            }
            | Self::ReceiptPreferencesGet { conversation_id }
            | Self::ReceiptPreferencesSet {
                conversation_id, ..
            } => Some(conversation_id),
            Self::ParticipantRoute { request } => Some(&request.conversation_id),
            Self::Task { request } => request.command.conversation_id(),
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
            | Self::ArtifactInspect {
                conversation_id, ..
            }
            | Self::ArtifactUploadLink {
                conversation_id, ..
            }
            | Self::ArtifactDownloadLink {
                conversation_id, ..
            }
            | Self::ArtifactLinkGet {
                conversation_id, ..
            }
            | Self::ArtifactLinkAppend {
                conversation_id, ..
            }
            | Self::ArtifactDelete {
                conversation_id, ..
            }
            | Self::Close {
                conversation_id, ..
            } => Some(conversation_id),
            Self::BrowserBinding {
                conversation_id, ..
            } => conversation_id.as_deref(),
            _ => None,
        }
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::ConnectionPrepare { .. } => "connection.offer",
            Self::SessionControl { request } => request.action.required_scope(),
            Self::SessionDecision { request } => request.decision.required_scope(),
            Self::SessionObservations { .. } => "session_observations",
            Self::ConversationSnapshot { .. } => "conversation_snapshot",
            Self::ConversationEvents { .. } => "conversation_events",
            Self::Transition { .. } => "transition",
            Self::DurableOperation { .. } => "durable_operation",
            Self::MessageReceipts { .. } => "message_receipts",
            Self::MessageReceipt { .. } => "message_receipt",
            Self::ReceiptPreferencesGet { .. } => "receipt_preferences_get",
            Self::ReceiptPreferencesSet { .. } => "receipt_preferences_set",
            Self::ParticipantRoute { .. } => "participant_route",
            Self::ContactPairingStart { .. } => "contact_pairing_start",
            Self::ContactPairingComplete { .. } => "contact_pairing_complete",
            Self::ContactPairingRevoke { .. } => "contact_pairing_revoke",
            Self::ContactState { .. } => "contact_state",
            Self::Task { .. } => "task",
            Self::Resolve { .. } => "resolve",
            Self::Capabilities { .. } => "capabilities",
            Self::BrowserBinding { .. } => "browser_binding",
            Self::ContextDeliveries { .. } => "context_deliveries",
            Self::Membership { request } => request.required_scope(),
            Self::MembershipSnapshot { .. } => "conversation.get",
            Self::MembershipInvitations { .. } => "conversation.participant.add",
            Self::Create { .. } => "create",
            Self::Conversations { .. } => "conversation.list",
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
            Self::ArtifactInspect { .. } => "artifact_inspect",
            Self::ArtifactUploadLink { .. } => "artifact_upload_link",
            Self::ArtifactDownloadLink { .. } => "artifact_download_link",
            Self::ArtifactLinkGet { .. } => "artifact_link_get",
            Self::ArtifactLinkAppend { .. } => "artifact_link_append",
            Self::ArtifactDelete { .. } => "artifact_delete",
            Self::Health {} => "health",
        }
    }
    /// Additive read APIs use the existing authority for the information they expose.
    pub fn required_scope(&self) -> &'static str {
        match self {
            Self::SessionObservations { .. } => "session.observation",
            Self::ConversationSnapshot { .. } => "conversation.get",
            Self::ConversationEvents { .. } => "conversation.subscribe",
            Self::Transition { request } => request.intent.required_scope(),
            Self::DurableOperation { request } => match request {
                transitions::OperationAction::Get { .. } => "operation.get",
                transitions::OperationAction::Cancel { .. } => "operation.cancel",
                transitions::OperationAction::Reconcile { .. } => "operation.reconcile",
            },
            Self::MessageReceipts { .. }
            | Self::MessageReceipt { .. }
            | Self::ReceiptPreferencesGet { .. }
            | Self::ReceiptPreferencesSet { .. } => "history",
            Self::ArtifactInspect { .. }
            | Self::ArtifactDownloadLink { .. }
            | Self::ArtifactLinkGet { .. } => "artifact_get",
            Self::ArtifactUploadLink { .. } | Self::ArtifactLinkAppend { .. } => "artifact_append",
            Self::ParticipantRoute { .. } => "resolve",
            Self::ContactPairingStart { .. }
            | Self::ContactPairingComplete { .. }
            | Self::ContactPairingRevoke { .. }
            | Self::ContactState { .. } => "resolve",
            Self::Task { request } => request.command.required_scope(),
            Self::Capabilities { .. } => "resolve",
            Self::BrowserBinding { .. } => "session_get",
            Self::ContextDeliveries { .. } => "context_view",
            _ => self.name(),
        }
    }
    pub fn request_id(&self) -> Option<&str> {
        match self {
            Self::ConnectionPrepare { request } => Some(&request.preparation_id),
            Self::SessionControl { request } => Some(&request.operation_id),
            Self::SessionDecision { request } => Some(&request.operation_id),
            Self::Transition { request } => Some(&request.operation_id),
            Self::DurableOperation { request } => match request {
                transitions::OperationAction::Get { .. } => None,
                transitions::OperationAction::Cancel { operation_id, .. }
                | transitions::OperationAction::Reconcile { operation_id, .. } => {
                    Some(operation_id)
                }
            },
            Self::MessageReceipt { request_id, .. }
            | Self::ReceiptPreferencesSet { request_id, .. } => Some(request_id),
            Self::ContactPairingStart { request_id, .. }
            | Self::ContactPairingComplete { request_id, .. }
            | Self::ContactPairingRevoke { request_id, .. } => Some(request_id),
            Self::Task { request } => request.command.request_id(),
            Self::Membership { request } => Some(&request.operation_id),
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
            | Self::ArtifactLinkAppend { request_id, .. }
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

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Message {
    pub id: String,
    pub conversation_id: String,
    pub sender: String,
    pub body: String,
    pub sender_participant_id: Option<String>,
    pub audience: Vec<messages::MessageRecipient>,
    pub content_type: String,
    pub body_encoding: messages::BodyEncoding,
    pub label: String,
    pub attachments: Vec<messages::AttachmentRef>,
    pub in_reply_to_msg: Option<String>,
    pub accepted_at: Option<i64>,
}
impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Message")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt: Option<receipts::RecipientReceipt>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<ContextRecord>,
}
/// Exact authorized record projection. Raw whole-record/manifest fingerprints
/// remain private to the server. This reference is data and grants no access.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextRecord {
    pub uuid: String,
    pub session_id: String,
    pub revision: u64,
    pub state: ContextRecordState,
    pub content_hash: Option<String>,
    pub valid_until: Option<i64>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextRecordState {
    Available,
    Unavailable,
    Deleted,
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
    pub destination: Option<ContextDeliveryDestination>,
    pub problem: Option<ContextDeliveryProblem>,
    pub lifecycle: Option<ContextDeliveryLifecycle>,
}
/// Captured native destination. This describes the original export and grants no access.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextDeliveryDestination {
    pub connection_id: String,
    pub generation: u64,
    pub purpose: ContextDeliveryPurpose,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextDeliveryPurpose {
    PublicSession,
    PrivateConsultation,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextDeliveryProblem {
    Rejected,
    Truncated,
    Incomplete,
    Unknown,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextDeliveryPhase {
    Initialization,
    PostCreation,
    Update,
}
/// Observed control lifecycle, independent of media readiness. None means that
/// lane does not record an independently admitted cleanup phase.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextDeliveryLifecycle {
    pub phase: ContextDeliveryPhase,
    pub parent_delivery_id: Option<String>,
    pub creation_admitted: bool,
    pub creation_confirmed: bool,
    pub export_admitted: bool,
    pub delivery_confirmed: bool,
    pub cleanup_admitted: Option<bool>,
    pub cleanup_uncertain: Option<bool>,
    pub closed: bool,
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
    /// Terminal refusal before external dispatch, when a connector cannot carry the content.
    pub refusal_code: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
// This bounded wire DTO keeps its existing public Rust construction API.
// Boxing the inline health readiness field would change caller types solely
// to reduce stack size; all wire responses retain their protocol size limits.
#[allow(clippy::large_enum_variant)]
pub enum Outcome {
    SessionControl(session_control::ControlOutcome),
    ConnectionPrepared(connection_control::PreparedConnection),
    SessionDecision(session_decisions::DecisionOutcome),
    SessionObservations(events::ObservationPage),
    ConversationSnapshot(events::SnapshotPage),
    ConversationEvents(events::EventPage),
    DurableOperation(transitions::OperationRecord),
    ArtifactLink(attachments::ArtifactLink),
    ParticipantRoute(routing::ParticipantRoute),
    Contact(contacts::ContactState),
    ContactPairing(contacts::Pairing),
    /// Task results retain their own pending/uncertain semantics and provenance.
    Task(Box<tasks::TaskOutcome>),
    Capabilities(Capabilities),
    BrowserBinding(BrowserBinding),
    ContextDeliveries(ContextDeliveryPage),
    Call(Call),
    Session(Session),
    SipLeg(SipLeg),
    Availability(Availability),
    ContextView(ContextView),
    MessageReceipts(receipts::ReceiptPage),
    ReceiptAcknowledged(receipts::ReceiptAcknowledgement),
    ReceiptPreferences(receipts::ReceiptPreferences),
    Artifact(Artifact),
    ArtifactChunk(ArtifactChunk),
    Identity(Identity),
    Conversation(Conversation),
    Conversations(conversations::ListPage),
    Membership(membership::MembershipOutcome),
    MembershipSnapshot(membership::MembershipSnapshot),
    MembershipInvitations(membership::InvitationPage),
    Accepted {
        #[serde(skip_serializing_if = "Option::is_none")]
        receipt: Option<transitions::MutationReceipt>,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        readiness: Option<Readiness>,
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
            message: crate::messages::MessageDraft::text_all("hello"),
        });
        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(request, serde_json::from_value(value.clone()).unwrap());
        let mut spoofed = value;
        spoofed["sender"] = serde_json::json!("other-actor");
        assert!(serde_json::from_value::<Request>(spoofed).is_err());
    }

    #[test]
    fn browser_selection_preserves_legacy_wire_and_exact_selected_binding() {
        let legacy = Operation::BrowserBinding {
            conversation_id: None,
            session_id: None,
        };
        assert_eq!(
            serde_json::to_value(&legacy).unwrap(),
            serde_json::json!({"op":"browser_binding"})
        );
        assert_eq!(
            serde_json::from_value::<Operation>(serde_json::json!({"op":"browser_binding"}))
                .unwrap(),
            legacy
        );
        let selected = Operation::BrowserBinding {
            conversation_id: Some("conversation".into()),
            session_id: Some("session".into()),
        };
        let value = serde_json::json!({"op":"browser_binding","conversation_id":"conversation","session_id":"session"});
        assert_eq!(serde_json::to_value(&selected).unwrap(), value);
        assert_eq!(
            serde_json::from_value::<Operation>(value).unwrap(),
            selected
        );
        assert_eq!(selected.conversation_id(), Some("conversation"));
        assert_eq!(selected.required_scope(), "session_get");
        assert!(serde_json::from_value::<Operation>(serde_json::json!({"op":"browser_binding","conversation_id":"c","session_id":"s","identity_id":"spoof"})).is_err());
    }

    #[test]
    fn operations_reject_claimed_sender_and_unknown_fields() {
        let mut send = serde_json::to_value(Operation::Send {
            request_id: "r".into(),
            conversation_id: "c".into(),
            message: messages::MessageDraft::text_all("hello"),
        })
        .unwrap();
        send["sender"] = serde_json::json!("other");
        assert!(serde_json::from_value::<Operation>(send).is_err());
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

pub mod record_retention;
