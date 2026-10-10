//! Public task records contain provenance and bindings, never grant plaintext.
use super::{EffectState, TaskState};
use serde::{Deserialize, Serialize};

/// Closed set of delegatable communication operations; administrative authority
/// and creation of a new conversation are never implicitly delegated.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TaskPermission {
    Resolve,
    Capabilities,
    Send,
    History,
    ContextView,
    ContextDeliveries,
    Inbox,
    RequestStatus,
    ArtifactBegin,
    ArtifactAppend,
    ArtifactFinish,
    ArtifactGet,
    ArtifactDelete,
    SessionInvite,
    SessionAccept,
    SessionReject,
    SessionCancel,
    SessionEnd,
    SessionGet,
    CallStart,
    CallStatus,
    CallEnd,
    SipDial,
    SipStatus,
    SipEnd,
}
impl TaskPermission {
    /// Required backing actor scope; capability discovery reuses resolve authority.
    pub fn required_scope(self) -> &'static str {
        match self {
            Self::Resolve | Self::Capabilities => "resolve",
            Self::Send => "send",
            Self::History => "history",
            Self::ContextView => "context_view",
            Self::ContextDeliveries => "context_view",
            Self::Inbox => "inbox",
            Self::RequestStatus => "request_status",
            Self::ArtifactBegin => "artifact_begin",
            Self::ArtifactAppend => "artifact_append",
            Self::ArtifactFinish => "artifact_finish",
            Self::ArtifactGet => "artifact_get",
            Self::ArtifactDelete => "artifact_delete",
            Self::SessionInvite => "session_invite",
            Self::SessionAccept => "session_accept",
            Self::SessionReject => "session_reject",
            Self::SessionCancel => "session_cancel",
            Self::SessionEnd => "session_end",
            Self::SessionGet => "session_get",
            Self::CallStart => "call_start",
            Self::CallStatus => "call_status",
            Self::CallEnd => "call_end",
            Self::SipDial => "sip_dial",
            Self::SipStatus => "sip_status",
            Self::SipEnd => "sip_end",
        }
    }
}

/// Immutable authority ceiling; current actor/membership/route checks still apply.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TaskScope {
    pub operations: Vec<TaskPermission>,
    pub recipients: Vec<String>,
}
/// Developer-supplied task inputs. The authenticated actor supplies ownership.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TaskDefinition {
    pub conversation_id: String,
    pub scope: TaskScope,
    pub expires_at: i64,
    /// Optional existing conversation event; its ownership is checked by the store.
    pub initiating_event_id: Option<String>,
}
/// Durable task metadata; revision is incremented on every lifecycle transition.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TaskRecord {
    pub id: String,
    pub owner_id: String,
    pub conversation_id: String,
    pub scope: TaskScope,
    pub state: TaskState,
    pub revision: u64,
    pub expires_at: i64,
    pub initiating_event_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub reason: Option<String>,
}
/// Runtime correlation type; neither headers nor IDs confer authority on their own.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InvocationKind {
    Native,
    ProviderChat,
    ProviderCall,
    HostSession,
}
/// Bound by the trusted composition/adapter after verifying its external origin.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InvocationBinding {
    pub kind: InvocationKind,
    pub adapter: String,
    pub external_id: String,
}
/// Inspectable grant metadata; the opaque bearer secret is issued separately.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InvocationGrant {
    pub id: String,
    pub task_id: String,
    pub binding: InvocationBinding,
    pub operations: Vec<TaskPermission>,
    pub expires_at: i64,
    pub revoked: bool,
}
/// Trusted issuance input. It is never a model-facing task command, and runtime
/// admission must prove that the binding was registered by its owning adapter.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GrantDefinition {
    pub task_id: String,
    pub binding: InvocationBinding,
    pub operations: Vec<TaskPermission>,
    pub expires_at: i64,
}
/// A bounded step lease is an ownership hint, not permission to retry an effect.
/// Once an effect may have left the process, expiry requires reconciliation.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TaskStepClaim {
    pub task_id: String,
    pub step_id: String,
    pub worker_id: String,
    pub revision: u64,
    pub expires_at: i64,
}
/// One effect identity includes its task, step and logical mutation.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TaskEffect {
    pub id: String,
    pub task_id: String,
    pub step_id: String,
    pub request_id: String,
    pub state: EffectState,
    pub outcome: Option<Box<crate::Outcome>>,
}
/// A reply explicitly assigned to one task; unassigned history has no task ID.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TaskReply {
    pub event_id: String,
    pub conversation_id: String,
    pub task_id: Option<String>,
    pub reply_to: Option<String>,
    pub assigned_by: Option<String>,
    pub consumed: bool,
}
/// A bounded tool result does not manufacture completion while the task is waiting.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskToolResult {
    Completed {
        task_id: String,
        outcome: Box<crate::Outcome>,
    },
    Pending {
        task_id: String,
        effect_id: Option<String>,
    },
    Uncertain {
        task_id: String,
        effect_id: String,
    },
}

/// Durable notification payload; authority is always rechecked when it is read.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum TaskEventData {
    State {
        state: TaskState,
        revision: u64,
        reason: Option<String>,
    },
    Reply(TaskReply),
    Effect(TaskEffect),
}
/// Journal entry with stable source attribution and commit order.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TaskEvent {
    pub id: String,
    pub task_id: String,
    pub actor_id: String,
    pub sequence: u64,
    pub created_at: i64,
    pub data: TaskEventData,
}
/// Retained evidence is data, never an instruction that expands task authority.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TaskSource {
    pub source_id: String,
    pub kind: String,
    pub actor_id: String,
    /// None explicitly represents legacy sources with no recorded timestamp.
    pub created_at: Option<i64>,
    pub provenance: String,
    /// Available, deleted, expired or unavailable; unavailable sources have no body.
    pub state: String,
    pub body: Option<String>,
}
/// Public results shared by native, HTTP, language SDK and MCP adapters.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum TaskOutcome {
    Record(Box<TaskRecord>),
    Replies {
        items: Vec<TaskReply>,
        next: Option<String>,
    },
    Inbox {
        events: Vec<TaskEvent>,
        next: Option<String>,
    },
    Recall {
        sources: Vec<TaskSource>,
        next: Option<String>,
        omitted: u64,
    },
    Effect(TaskEffect),
    Tool(TaskToolResult),
    /// Incoming message and explicit task assignment are committed before ACK.
    /// A terminal/unavailable task may leave the retained reply unassigned.
    ReplyAccepted {
        request_id: String,
        message_id: String,
        reply: TaskReply,
    },
}
