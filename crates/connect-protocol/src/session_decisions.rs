//! Durable decisions on an existing pending invitation, shared by all facades.
use crate::{transitions::id, Error, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Reject,
    Cancel,
}
impl Decision {
    pub fn required_scope(self) -> &'static str {
        match self {
            Self::Reject => "session.reject",
            Self::Cancel => "session.cancel",
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub operation_id: String,
    pub conversation_id: String,
    pub session_id: String,
    pub decision: Decision,
    pub by_participant: Option<String>,
    pub expected_revision: Option<u64>,
    pub reason_code: u64,
    pub reason: String,
}
impl Request {
    pub fn validate(&self) -> Result<()> {
        for value in [&self.operation_id, &self.conversation_id, &self.session_id] {
            id(value)?;
        }
        if let Some(value) = &self.by_participant {
            id(value)?;
        }
        if self.reason.len() > 256
            || self.reason.chars().any(char::is_control)
            || self.reason_code > 65_535
            || self
                .expected_revision
                .is_some_and(|v| v > 9_007_199_254_740_991)
        {
            return Err(Error::new("invalid-request", "Invalid invitation decision"));
        }
        Ok(())
    }
}
impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionDecision")
            .field("operation_id", &self.operation_id)
            .field("decision", &self.decision)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionOutcome {
    pub operation_id: String,
    pub conversation_id: String,
    pub session_id: String,
    pub state: String,
    pub revision: u64,
    pub reason_code: u64,
    pub reason: String,
}
impl std::fmt::Debug for DecisionOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionDecisionOutcome")
            .field("session_id", &self.session_id)
            .field("state", &self.state)
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}
