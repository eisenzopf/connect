//! Protected developer exchange, separate from model commands and receipts.
use super::{validate_task_identifier, GrantDefinition, InvocationGrant};
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Local developer grant exchange. Never advertise as a model tool/profile.
pub const TASK_ACCESS_PROFILE: &str = "connect-task-access/1";

/// Request authenticated by a current backing credential with task-grant scope.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskAccessRequest {
    pub profile: String,
    pub command: TaskAccessCommand,
}
impl TaskAccessRequest {
    /// Build the explicitly versioned protected exchange.
    pub fn new(command: TaskAccessCommand) -> Self {
        Self {
            profile: TASK_ACCESS_PROFILE.into(),
            command,
        }
    }
    /// Shape/profile validation; current authority remains a store transaction.
    pub fn validate(&self, now: i64) -> Result<()> {
        if self.profile != TASK_ACCESS_PROFILE {
            return Err(Error::new(
                "unsupported-profile",
                "Task access profile is not supported",
            ));
        }
        match &self.command {
            TaskAccessCommand::Issue {
                request_id,
                definition,
            } => {
                validate_task_identifier(request_id)?;
                definition.validate(now)
            }
            TaskAccessCommand::Revoke { grant_id } => validate_task_identifier(grant_id),
        }
    }
}

/// Protected local management commands. Provider origins use verified adapters.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskAccessCommand {
    Issue {
        request_id: String,
        definition: GrantDefinition,
    },
    Revoke {
        grant_id: String,
    },
}

/// Secret-bearing intended credential exchange. Debug output redacts plaintext;
/// explicitly serializing this type is reserved for protected transport/files.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtectedTaskGrant {
    pub grant: InvocationGrant,
    bearer: String,
}
impl ProtectedTaskGrant {
    /// Compose an intended protected exchange; this value itself grants nothing.
    pub fn new(grant: InvocationGrant, bearer: String) -> Self {
        Self { grant, bearer }
    }
    /// Explicit export to the protected host connection. Never log this value.
    pub fn expose_secret(&self) -> &str {
        &self.bearer
    }
}
impl fmt::Debug for ProtectedTaskGrant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProtectedTaskGrant")
            .field("grant", &self.grant)
            .field("bearer", &"[REDACTED]")
            .finish()
    }
}

/// Results for the protected exchange, never an ordinary task/tool outcome.
#[derive(Debug, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum TaskAccessOutcome {
    Issued(ProtectedTaskGrant),
    Revoked { grant_id: String },
}
/// Explicit envelope. The server disables caching of this credential exchange.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskAccessResponse {
    pub profile: String,
    pub result: Result<TaskAccessOutcome>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn secret_exchange_redacts_debug_and_is_not_a_task_result() {
        let value: TaskAccessOutcome = serde_json::from_value(serde_json::json!({"kind":"issued","data":{"grant":{"id":"g","task_id":"t","binding":{"kind":"native","adapter":"a","external_id":"b"},"operations":["history"],"expires_at":123,"revoked":false},"bearer":"test-secret"}})).expect("exchange");
        assert!(!format!("{value:?}").contains("test-secret"));
        let json = serde_json::to_value(value).expect("protected serialization");
        assert_eq!(json["data"]["bearer"], "test-secret");
        assert!(serde_json::from_value::<super::super::TaskOutcome>(json).is_err());
    }
}
