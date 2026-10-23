//! Authorized snapshot and journal reads; transport wakeups are never facts.
use crate::{
    membership::MemberView, transitions::OperationRecord, Artifact, Call, Message, Session,
};
use serde::{Deserialize, Serialize};
mod agent;
mod provider_call;
pub use agent::*;
pub use provider_call::*;
pub const SNAPSHOT_LEASE_SECONDS: u32 = 30;
pub const EVENT_QUEUE_LIMIT: usize = 256;
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotQuery {
    pub page: Option<String>,
    pub limit: Option<u32>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventQuery {
    pub after: Option<String>,
    pub limit: Option<u32>,
    pub lease: Option<String>,
    pub lease_seconds: Option<u32>,
    #[serde(default)]
    pub wait_ms: u32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationState {
    pub conversation_id: String,
    pub revision: u64,
    pub closed: bool,
    pub policy: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionState {
    pub connection_id: String,
    pub session_id: String,
    pub participant_id: String,
    pub membership_epoch: u64,
    pub endpoint_id: String,
    pub device_id: String,
    pub generation: u64,
    pub revision: u64,
    pub state: String,
    pub replaces: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamState {
    pub stream_id: String,
    pub session_id: String,
    pub participant_id: String,
    pub connection_id: Option<String>,
    pub medium: String,
    pub state: String,
    pub revision: u64,
}
/// Original committed Conversation closure, shared by journal and typed hints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationClosed {
    pub conversation_id: String,
    pub reason_code: u64,
    pub reason: String,
    pub closed_at: i64,
}
/// Original terminal metadata, independent of later record finalization/retention.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionEnded {
    pub session: SessionSnapshot,
    pub by_participant: Option<String>,
    pub by_membership_epoch: Option<u64>,
    pub reason_code: u64,
    pub reason: String,
    pub ended_at: i64,
}
/// Original activation snapshot. Later topology or codec changes do not rewrite
/// this fact; current state is obtained independently from a snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionStarted {
    pub session: SessionSnapshot,
    pub started_at: i64,
    pub participants_present: Vec<String>,
    pub active_connections: Vec<SessionActiveConnection>,
    pub negotiated_capabilities: serde_json::Value,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionActiveConnection {
    pub connid: String,
    pub participant_id: String,
    pub transport: String,
}
/// Immutable verified public appearance, independent of later role/name changes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionJoinedParticipant {
    pub participant_id: String,
    pub identity_id: String,
    pub kind: crate::provisioning::IdentityKind,
    pub role: String,
    pub display_name: String,
    pub joined_at: i64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionParticipantJoined {
    pub fact_id: String,
    pub session: SessionSnapshot,
    pub membership_epoch: u64,
    pub participant: SessionJoinedParticipant,
    pub via_connection: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionParticipantLeft {
    pub fact_id: String,
    pub session: SessionSnapshot,
    pub participant_id: String,
    pub membership_epoch: u64,
    pub left_at: i64,
    pub reason: String,
}
/// A bounded hint, never a source grant or an assertion that audio was captured.
/// Fetch authenticates again and returns the digest of the caller's exact view.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecordAvailability {
    Pending {
        session_id: String,
    },
    Ready {
        session_id: String,
        uuid: String,
        revision: u64,
    },
    Unavailable {
        session_id: String,
    },
}
impl RecordAvailability {
    pub fn session_id(&self) -> &str {
        match self {
            Self::Pending { session_id }
            | Self::Ready { session_id, .. }
            | Self::Unavailable { session_id } => session_id,
        }
    }
}
/// Current conversation topology and operation state. Message bodies and file
/// bytes remain separately paged by history and artifact APIs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum SnapshotItem {
    Conversation(ConversationState),
    Participant(MemberView),
    Session(SessionSnapshot),
    SessionParticipant(SessionAppearance),
    Connection(ConnectionState),
    Stream(StreamState),
    Operation(OperationRecord),
    ProviderCall(ProviderSessionVoiceView),
    Record(RecordAvailability),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotPage {
    pub conversation_id: String,
    pub snapshot_id: String,
    pub revision: u64,
    pub cursor: String,
    pub expires_at: i64,
    pub items: Vec<SnapshotItem>,
    pub next: Option<String>,
    pub has_more: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum JournalData {
    SessionStarted(SessionStarted),
    SessionParticipantJoined(SessionParticipantJoined),
    SessionParticipantLeft(SessionParticipantLeft),
    ConversationClosed(ConversationClosed),
    SessionEnded(SessionEnded),
    ProviderCall(ProviderSessionVoiceView),
    Record(RecordAvailability),
    ConnectionPrepared(crate::connection_control::PreparedConnection),
    ConnectionRetired(ConnectionState),
    ConnectionReady(ConnectionState),
    Message(Message),
    Session(Session),
    SessionControl(crate::session_control::SessionView),
    Artifact(Artifact),
    Call(Call),
    Receipt(crate::receipts::RecipientReceipt),
    NativeDataWrite(crate::native_data::WriteObservation),
    Membership {
        participant_id: Option<String>,
        identity_id: Option<String>,
        role: Option<String>,
        active: Option<bool>,
        membership_epoch: Option<u64>,
        removal_reason: Option<String>,
    },
    Operation {
        operation_id: String,
        record_id: String,
        state: crate::transitions::OperationState,
    },
    Changed {
        resource_kind: Option<String>,
        resource_id: Option<String>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventSource {
    pub kind: String,
    pub correlation_id: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEvent {
    pub event_id: String,
    pub cursor: String,
    pub conversation_id: String,
    pub session_id: Option<String>,
    pub connection_id: Option<String>,
    pub participant_id: Option<String>,
    pub operation_id: Option<String>,
    pub kind: String,
    pub revision: u64,
    pub source: EventSource,
    pub observed_at: i64,
    pub audience: Vec<crate::messages::MessageRecipient>,
    pub data: JournalData,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventPage {
    pub conversation_id: String,
    pub events: Vec<JournalEvent>,
    pub cursor: String,
    pub has_more: bool,
    pub lease: Option<String>,
    pub expires_at: Option<i64>,
    pub resync_required: bool,
    pub snapshot: Option<SnapshotPage>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionAppearance {
    pub session_id: String,
    pub participant_id: String,
    pub membership_epoch: u64,
    pub role: String,
    pub state: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationCapabilities {
    pub transcript: bool,
    pub speech: bool,
    pub actions: Vec<String>,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ObservationDraft {
    Agent {
        observation: AgentObservation,
    },
    Transcript {
        text: String,
        completeness: TranscriptCompleteness,
        turn_id: Option<String>,
        fragment_id: Option<String>,
    },
    Speech {
        state: SpeechState,
        turn_id: Option<String>,
    },
    ActionRequest {
        action: String,
        provider_tool_call_id: String,
        arguments: serde_json::Value,
        turn_id: Option<String>,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptCompleteness {
    Interim,
    Final,
    Incomplete,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeechState {
    Started,
    Stopped,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionObservation {
    pub work: Option<ObservationWork>,
    pub observation_id: String,
    pub conversation_id: String,
    pub session_id: String,
    pub participant_id: String,
    pub membership_epoch: u64,
    pub connection_id: String,
    pub generation: u64,
    pub source: EventSource,
    pub observed_at: i64,
    pub data: ObservationDraft,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationRoute {
    pub work: Option<ObservationWork>,
    pub correlation_id: String,
    pub participant_id: String,
    pub connection_id: String,
    pub generation: u64,
    pub source: String,
    pub capabilities: ObservationCapabilities,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationPage {
    pub conversation_id: String,
    pub session_id: String,
    pub observations: Vec<SessionObservation>,
    pub expires_at: i64,
    pub routes: Vec<ObservationRoute>,
    pub cursor: String,
    pub has_more: bool,
    pub resync_required: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationQuery {
    pub after: Option<String>,
    pub limit: Option<u32>,
    #[serde(default)]
    pub wait_ms: u32,
}

impl std::fmt::Debug for ObservationDraft {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Agent { .. } => "Agent(redacted)",
            Self::Transcript { .. } => "Transcript(redacted)",
            Self::Speech { .. } => "Speech",
            Self::ActionRequest { .. } => "ActionRequest(redacted)",
        })
    }
}

pub type SessionSnapshot = crate::session_control::SessionView;

/// The committed question exported to this host-selected work context. These
/// labels are provenance, never a task grant, semantic answer or execution ID.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationWork {
    pub task_id: String,
    pub effect_id: String,
    pub question_id: String,
}
