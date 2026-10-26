//! Public task records contain provenance and bindings, never grant plaintext.
use super::{EffectState, TaskState};
use serde::{Deserialize, Serialize};

/// Closed set of delegatable communication operations and explicit bound-task
/// cancellation. Administrative authority and conversation creation are never
/// implicitly delegated; `TaskCancel` requires its own immutable ceiling.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TaskPermission {
    Resolve,
    Capabilities,
    Send,
    History,
    ContextView,
    /// Explicit permission to include authorized record projections in context.
    RecordGet,
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
    /// Explicit cancellation of only this delegated task. Never implied by
    /// messaging, session control, status or any other communication permission.
    TaskCancel,
}
impl TaskPermission {
    /// Required backing actor scope; capability discovery reuses resolve authority.
    pub fn required_scope(self) -> &'static str {
        match self {
            Self::Resolve | Self::Capabilities => "resolve",
            Self::Send => "send",
            Self::History => "history",
            Self::ContextView => "context_view",
            Self::RecordGet => "record_get",
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
            Self::TaskCancel => "task_cancel",
        }
    }
}

/// Immutable authority ceiling; current actor/membership/route checks still apply.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TaskScope {
    pub operations: Vec<TaskPermission>,
    pub recipients: Vec<String>,
    /// Explicit reviewed enrollment purposes for task-origin SMS. Empty denies
    /// SMS dispatch without limiting the task's other communication channels.
    /// Omit empty policy to preserve canonical digests of existing grants.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sms_enrollments: Vec<TaskSmsEnrollment>,
}
/// A reference to immutable evidence supplied by a trusted enrollment authority.
/// This reference neither enrolls a recipient nor renews or overrides consent.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TaskSmsEnrollment {
    pub source: String,
    pub grant_id: String,
    pub purpose: String,
}
/// Developer-supplied task inputs. The authenticated actor supplies ownership.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InvocationKind {
    Native,
    ProviderChat,
    ProviderCall,
    HostSession,
}
/// Bound by the trusted composition/adapter after verifying its external origin.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InvocationBinding {
    pub kind: InvocationKind,
    pub adapter: String,
    pub external_id: String,
}
/// Inspectable grant metadata; the opaque bearer secret is issued separately.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TaskEffect {
    pub id: String,
    pub task_id: String,
    pub step_id: String,
    pub request_id: String,
    pub state: EffectState,
    pub outcome: Option<Box<crate::Outcome>>,
    /// Known rejection/delivery failure; uncertainty is represented by state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<crate::Error>,
}
/// A reply explicitly assigned to one task; unassigned history has no task ID.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
    Claim(TaskStepClaim),
}
/// Journal entry with stable source attribution and commit order.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TaskSource {
    pub source_id: String,
    pub kind: String,
    /// Empty only when a retained reference's underlying attribution is unavailable.
    pub actor_id: String,
    /// None explicitly represents legacy sources with no recorded timestamp.
    pub created_at: Option<i64>,
    /// Journal event reference and projection/omission notes, never authority.
    pub provenance: String,
    /// Available, deleted, expired or unavailable; unavailable sources have no body.
    pub state: String,
    /// Message text or current session/artifact metadata. Never artifact bytes,
    /// server paths, or a replacement for an independently authorized file read.
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<crate::ContextRecord>,
}
/// Public results shared by native, HTTP, language SDK and MCP adapters.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
    /// Oldest-first recipient journal sources, at most 32 KiB serialized per page.
    Recall {
        sources: Vec<TaskSource>,
        /// Last returned journal position. Reuse it to catch up after an empty page.
        next: Option<String>,
        /// Eligible entries after the input cursor not included in this page.
        /// Missing bodies are separately explicit in each returned source's state.
        omitted: u64,
    },
    Effect(TaskEffect),
    Claim(Option<TaskStepClaim>),
    Tool(TaskToolResult),
    /// Incoming message and explicit task assignment are committed before ACK.
    /// A terminal/unavailable task may leave the retained reply unassigned.
    ReplyAccepted {
        request_id: String,
        message_id: String,
        reply: TaskReply,
    },
}
