//! Deterministic admission checks. Resource authority is checked transactionally later.
use super::{
    GrantDefinition, InvocationBinding, TaskCommand, TaskDefinition, TaskPermission, TaskScope,
    MAX_GRANT_TTL_SECONDS, MAX_SCOPE_ITEMS, MAX_TASK_ID_BYTES, MAX_TASK_TTL_SECONDS,
    MAX_TOOL_WAIT_MS,
};
use crate::{Error, Operation, Result};
use std::collections::HashSet;

pub(super) fn id(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > MAX_TASK_ID_BYTES
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
    {
        return Err(invalid(
            "Task identifiers must be bounded ASCII identifiers",
        ));
    }
    Ok(())
}
fn invalid(message: &str) -> Error {
    Error::new("invalid-request", message)
}
impl TaskScope {
    /// Reject wildcard authority, duplicate scope entries and empty delegation.
    pub fn validate(&self) -> Result<()> {
        if self.operations.is_empty()
            || self.operations.len() > MAX_SCOPE_ITEMS
            || self.recipients.is_empty()
            || self.recipients.len() > MAX_SCOPE_ITEMS
            || self.operations.iter().collect::<HashSet<_>>().len() != self.operations.len()
            || self.recipients.iter().collect::<HashSet<_>>().len() != self.recipients.len()
        {
            return Err(invalid(
                "Task scope requires bounded unique operations and recipients",
            ));
        }
        for recipient in &self.recipients {
            id(recipient)?;
        }
        if self.sms_enrollments.len() > MAX_SCOPE_ITEMS
            || (!self.sms_enrollments.is_empty()
                && !self.operations.contains(&super::TaskPermission::Send))
        {
            return Err(invalid(
                "SMS policy requires bounded enrollment references and send authority",
            ));
        }
        let mut enrollments = HashSet::new();
        for enrollment in &self.sms_enrollments {
            if [&enrollment.source, &enrollment.grant_id]
                .iter()
                .any(|s| s.trim().is_empty() || s.len() > 256 || s.chars().any(char::is_control))
                || enrollment.purpose.trim().is_empty()
                || enrollment.purpose.len() > 2000
                || !enrollments.insert((&enrollment.source, &enrollment.grant_id))
            {
                return Err(invalid(
                    "SMS policy requires unique bounded references and an exact reviewed purpose",
                ));
            }
        }
        Ok(())
    }
}
impl TaskDefinition {
    /// Parent credential expiry is an additional ceiling at task creation.
    pub fn validate(&self, now: i64) -> Result<()> {
        id(&self.conversation_id)?;
        self.scope.validate()?;
        if self.expires_at <= now || self.expires_at.saturating_sub(now) > MAX_TASK_TTL_SECONDS {
            return Err(invalid("Task expiry must be within the next 24 hours"));
        }
        if let Some(event) = &self.initiating_event_id {
            id(event)?;
        }
        Ok(())
    }
}
impl TaskPermission {
    /// Convert only delegatable operations; unknown/new/admin operations fail closed.
    pub fn for_operation(operation: &Operation) -> Result<Self> {
        serde_json::from_value(serde_json::Value::String(operation.name().into()))
            .map_err(|_| Error::forbidden())
    }
}
impl InvocationBinding {
    /// Bound correlation strings are not proof of a verified provider origin.
    pub fn validate(&self) -> Result<()> {
        id(&self.adapter)?;
        id(&self.external_id)
    }
}
impl GrantDefinition {
    /// Validate shape and local lifetime. The store additionally checks the task,
    /// parent credential, subset ceiling and adapter-verified binding.
    pub fn validate(&self, now: i64) -> Result<()> {
        id(&self.task_id)?;
        self.binding.validate()?;
        if self.operations.is_empty()
            || self.operations.len() > MAX_SCOPE_ITEMS
            || self.operations.iter().collect::<HashSet<_>>().len() != self.operations.len()
            || self.expires_at <= now
            || self.expires_at.saturating_sub(now) > MAX_GRANT_TTL_SECONDS
        {
            return Err(invalid(
                "Grant requires a unique bounded subset and at most five minutes",
            ));
        }
        Ok(())
    }
}
impl TaskCommand {
    /// Validate command shapes without interpreting caller-supplied IDs as authority.
    pub fn validate(&self, now: i64) -> Result<()> {
        if let Some(value) = self.request_id() {
            id(value)?;
        }
        if let Some(value) = self.task_id() {
            id(value)?;
        }
        match self {
            Self::Create { definition, .. } => definition.validate(now)?,
            Self::Cancel { reason, .. } | Self::Transition { reason, .. } => {
                if reason
                    .as_ref()
                    .is_some_and(|s| s.len() > 512 || s.chars().any(char::is_control))
                {
                    return Err(invalid(
                        "Task reason must be at most 512 bytes without control characters",
                    ));
                }
            }
            Self::AssignReply {
                event_id, reply_to, ..
            } => {
                id(event_id)?;
                if let Some(value) = reply_to {
                    id(value)?;
                }
            }
            Self::ConsumeReply { event_id, .. } => id(event_id)?,
            Self::Reply {
                conversation_id,
                reply_to,
                ..
            } => {
                id(conversation_id)?;
                id(reply_to)?;
                // The receiving store applies its current message-byte limit.
            }
            Self::Invoke {
                request_id,
                step_id,
                operation,
                reply_to,
                wait_for_reply,
                ..
            } => {
                id(step_id)?;
                TaskPermission::for_operation(operation)?;
                if *wait_for_reply && !matches!(operation.as_ref(), Operation::Send { .. }) {
                    return Err(invalid(
                        "Only an explicit message question can await a reply",
                    ));
                }
                if operation
                    .request_id()
                    .is_some_and(|nested| nested != request_id)
                {
                    return Err(invalid(
                        "Task and effect must preserve one logical mutation ID",
                    ));
                }
                if let Some(value) = reply_to {
                    if !matches!(operation.as_ref(), Operation::Send { .. }) {
                        return Err(invalid("Only a message can reference a task reply"));
                    }
                    id(value)?;
                }
            }
            Self::Replies {
                conversation_id,
                after,
                limit,
            } => {
                id(conversation_id)?;
                page(after, *limit)?;
            }
            Self::Inbox {
                after,
                limit,
                wait_ms,
                ..
            }
            | Self::Wait {
                after,
                limit,
                wait_ms,
                ..
            } => {
                page(after, *limit)?;
                if *wait_ms > MAX_TOOL_WAIT_MS {
                    return Err(invalid("Task wait exceeds ten seconds"));
                }
            }
            Self::Recall { after, limit, .. } => page(after, *limit)?,
            Self::EffectGet { effect_id, .. } => id(effect_id)?,
            Self::Claim {
                step_id, worker_id, ..
            }
            | Self::ReleaseClaim {
                step_id, worker_id, ..
            } => {
                id(step_id)?;
                id(worker_id)?;
            }
            Self::ClaimGet { step_id, .. } => id(step_id)?,
            Self::Get { .. } => (),
        }
        Ok(())
    }
}
fn page(after: &Option<String>, limit: u32) -> Result<()> {
    if limit == 0
        || limit > 100
        || after
            .as_ref()
            .is_some_and(|value| value.is_empty() || value.len() > 1024)
    {
        return Err(invalid(
            "Task pages require a limit of 1–100 and a bounded cursor",
        ));
    }
    Ok(())
}
