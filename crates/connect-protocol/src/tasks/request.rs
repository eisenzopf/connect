//! Task command shapes; bearer credentials and verified bindings stay out of inputs.
use super::{TaskDefinition, TaskState, TASK_PROFILE};
use crate::{Error, Operation, Result};
use serde::{Deserialize, Serialize};

/// Explicit versioned command carried by the task extension to control profile v1.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TaskRequest {
    pub task_profile: String,
    pub command: TaskCommand,
}
impl TaskRequest {
    pub fn new(command: TaskCommand) -> Self {
        Self {
            task_profile: TASK_PROFILE.into(),
            command,
        }
    }
    /// Profile and shape admission only. The store owns current authorization.
    pub fn validate(&self, now: i64) -> Result<()> {
        if self.task_profile != TASK_PROFILE {
            return Err(Error::new(
                "unsupported-profile",
                "Task profile is not supported",
            ));
        }
        self.command.validate(now)
    }
}

/// Administrative commands require backing actor authority; narrow invocation
/// grants may only use their bound task's communication/read operations.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskCommand {
    Create {
        request_id: String,
        definition: TaskDefinition,
    },
    Get {
        task_id: String,
    },
    Cancel {
        request_id: String,
        task_id: String,
        reason: Option<String>,
    },
    Transition {
        request_id: String,
        task_id: String,
        expected_revision: u64,
        state: TaskState,
        reason: Option<String>,
    },
    Replies {
        conversation_id: String,
        after: Option<String>,
        limit: u32,
    },
    AssignReply {
        request_id: String,
        task_id: String,
        event_id: String,
        reply_to: Option<String>,
    },
    /// A recipient answers a verified question using their own current send
    /// authority. The store derives the task from the question reference.
    Reply {
        request_id: String,
        conversation_id: String,
        reply_to: String,
        body: String,
    },
    Inbox {
        task_id: String,
        after: Option<String>,
        limit: u32,
        wait_ms: u32,
    },
    ConsumeReply {
        request_id: String,
        task_id: String,
        event_id: String,
        expected_revision: u64,
    },
    Invoke {
        request_id: String,
        task_id: String,
        step_id: String,
        /// Existing public operation; must match the enclosing logical request ID
        /// and bound conversation. Administrative/nested operations are forbidden.
        operation: Box<Operation>,
        /// Binds a question to an existing task reply; not an authority selector.
        reply_to: Option<String>,
        /// Explicit question intent. The server waits for an attributed reply;
        /// it never infers a question or task routing from message text.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        wait_for_reply: bool,
    },
    /// Read bounded source projections from the task's existing conversation.
    /// `after` is a task/reader-bound journal checkpoint, not an authority token.
    /// History or context-view authority must remain current on every page.
    Recall {
        task_id: String,
        after: Option<String>,
        limit: u32,
    },
    Wait {
        task_id: String,
        after: Option<String>,
        limit: u32,
        wait_ms: u32,
    },
    EffectGet {
        task_id: String,
        effect_id: String,
    },
    /// Runner coordination only; possessing a worker ID does not confer authority.
    Claim {
        request_id: String,
        task_id: String,
        step_id: String,
        worker_id: String,
        expected_revision: Option<u64>,
    },
    ClaimGet {
        task_id: String,
        step_id: String,
    },
    ReleaseClaim {
        request_id: String,
        task_id: String,
        step_id: String,
        worker_id: String,
        expected_revision: u64,
    },
}
impl TaskCommand {
    /// Conversation metadata present in the command. Task-ID reads resolve
    /// their conversation under current authority at the store boundary.
    pub fn conversation_id(&self) -> Option<&str> {
        match self {
            Self::Create { definition, .. } => Some(&definition.conversation_id),
            Self::Replies {
                conversation_id, ..
            }
            | Self::Reply {
                conversation_id, ..
            } => Some(conversation_id),
            Self::Invoke { operation, .. } => operation.conversation_id(),
            _ => None,
        }
    }
    /// Only read commands may hold a physical request open for a bounded wait.
    pub fn wait_ms(&self) -> Option<u32> {
        match self {
            Self::Inbox { wait_ms, .. } | Self::Wait { wait_ms, .. } => Some(*wait_ms),
            _ => None,
        }
    }
    /// Scope name for the parent actor. An invocation grant has a separate ceiling.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Create { .. } => "task_create",
            Self::Get { .. } => "task_get",
            Self::Cancel { .. } => "task_cancel",
            Self::Transition { .. } => "task_transition",
            Self::Replies { .. } => "task_replies",
            Self::AssignReply { .. } => "task_assign_reply",
            Self::Reply { .. } => "task_reply",
            Self::Inbox { .. } => "task_inbox",
            Self::ConsumeReply { .. } => "task_consume_reply",
            Self::Invoke { .. } => "task_invoke",
            Self::Recall { .. } => "task_recall",
            Self::Wait { .. } => "task_wait",
            Self::EffectGet { .. } => "task_effect_get",
            Self::Claim { .. } => "task_claim",
            Self::ClaimGet { .. } => "task_claim_get",
            Self::ReleaseClaim { .. } => "task_release_claim",
        }
    }
    /// Stable logical identity for mutations, independent of tool/request envelope IDs.
    pub fn request_id(&self) -> Option<&str> {
        match self {
            Self::Create { request_id, .. }
            | Self::Cancel { request_id, .. }
            | Self::Transition { request_id, .. }
            | Self::AssignReply { request_id, .. }
            | Self::Reply { request_id, .. }
            | Self::ConsumeReply { request_id, .. }
            | Self::Invoke { request_id, .. } => Some(request_id),
            Self::Claim { request_id, .. } | Self::ReleaseClaim { request_id, .. } => {
                Some(request_id)
            }
            _ => None,
        }
    }
    /// Target task, always compared with the authenticated invocation binding.
    pub fn task_id(&self) -> Option<&str> {
        match self {
            Self::Create { .. } | Self::Replies { .. } | Self::Reply { .. } => None,
            Self::Get { task_id }
            | Self::Cancel { task_id, .. }
            | Self::Transition { task_id, .. }
            | Self::AssignReply { task_id, .. }
            | Self::Inbox { task_id, .. }
            | Self::ConsumeReply { task_id, .. }
            | Self::Invoke { task_id, .. }
            | Self::Recall { task_id, .. }
            | Self::Wait { task_id, .. }
            | Self::EffectGet { task_id, .. } => Some(task_id),
            Self::Claim { task_id, .. }
            | Self::ClaimGet { task_id, .. }
            | Self::ReleaseClaim { task_id, .. } => Some(task_id),
        }
    }
    /// Native replies preserve ordinary sender authority. They never borrow a
    /// task owner's credential; the reply target is verified by the store.
    pub fn required_scope(&self) -> &'static str {
        match self {
            Self::Reply { .. } => "send",
            _ => self.name(),
        }
    }
}
