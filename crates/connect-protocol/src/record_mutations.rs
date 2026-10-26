//! Explicit local record erasure. A source selector is never read authority.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub operation_id: String,
    pub conversation_id: String,
    pub session_id: String,
    pub uuid: String,
    pub expected_revision: u64,
    pub action: Action,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Action {
    Redact { sources: Vec<Source> },
    Delete,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub target: Target,
    pub mode: Mode,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Target {
    Document,
    SessionOutcome,
    CaptureDiagnostics,
    Participant {
        participant_id: String,
        membership_epoch: u64,
    },
    Connection {
        connection_id: String,
        generation: u64,
    },
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    Content,
    Metadata,
}
impl Request {
    pub fn validate(&self) -> Result<()> {
        let id = |v: &str| {
            !v.is_empty()
                && v.len() <= 128
                && v.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._~-".contains(&b))
        };
        if ![&self.operation_id, &self.conversation_id, &self.session_id]
            .into_iter()
            .all(|s| id(s))
            || uuid::Uuid::parse_str(&self.uuid)
                .ok()
                .is_none_or(|id| id.is_nil() || id.to_string() != self.uuid)
            || !(1..9_007_199_254_740_991).contains(&self.expected_revision)
        {
            return Err(Error::new("invalid-request", "Invalid record mutation"));
        }
        if let Action::Redact { sources } = &self.action {
            let mut ids = std::collections::BTreeSet::new();
            if sources.is_empty()
                || sources.len() > 128
                || sources.iter().any(|s| {
                    !ids.insert(&s.target)
                        || match &s.target {
                            Target::Participant {
                                participant_id,
                                membership_epoch,
                            } => {
                                !id(participant_id)
                                    || !(1..=9_007_199_254_740_991).contains(membership_epoch)
                            }
                            Target::Connection {
                                connection_id,
                                generation,
                            } => {
                                !id(connection_id)
                                    || !(1..=9_007_199_254_740_991).contains(generation)
                            }
                            _ => false,
                        }
                })
            {
                return Err(Error::new(
                    "invalid-request",
                    "Invalid record source selection",
                ));
            }
        }
        Ok(())
    }
    pub fn permission(&self) -> &'static str {
        match self.action {
            Action::Redact { .. } => "record_redact",
            Action::Delete => "record_delete",
        }
    }
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub operation_id: String,
    pub uuid: String,
    pub revision: u64,
    pub action: String,
    pub at: chrono::DateTime<chrono::Utc>,
    pub remote_deletion: RemoteDeletion,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RemoteDeletion {
    NotAttempted,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub profile: String,
    pub result: Result<Outcome>,
}
