//! Provider data captured on a host-bound source. Never an execution receipt.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
pub const MAX_AGENT_OBSERVATION_BYTES: usize = 65_536;
pub const MAX_AGENT_DRAFT_BYTES: usize = 70 * 1024;
pub const MAX_AGENT_SEQUENCE: u64 = 9_007_199_254_740_991;
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentCompleteness {
    Partial,
    Final,
    Unavailable,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentSpeakerRole {
    User,
    Assistant,
    System,
    Other,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentActionPhase {
    Requested,
    ResultObserved,
    Unknown,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentObservationIssue {
    MissingFields,
    MalformedEvent,
    MismatchedSession,
    UnsupportedEvent,
    SequenceExhausted,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentObservationGap {
    pub first_sequence: u64,
    pub last_sequence: u64,
    pub observations: u64,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum AgentObservationContent {
    Transcript {
        text: Option<String>,
        speaker_role: Option<AgentSpeakerRole>,
        completeness: AgentCompleteness,
    },
    Speech {
        state: Option<String>,
        speaker_role: Option<AgentSpeakerRole>,
        turn: Option<u64>,
        completeness: AgentCompleteness,
    },
    Action {
        name: Option<String>,
        call_id: Option<String>,
        phase: AgentActionPhase,
        arguments: Option<serde_json::Value>,
        result: Option<serde_json::Value>,
        completeness: AgentCompleteness,
    },
    ProviderStatus {
        state: String,
    },
    Unavailable {
        issue: AgentObservationIssue,
    },
    Gap {
        first_sequence: u64,
        last_sequence: u64,
        observations: u64,
    },
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentObservation {
    pub local_sequence: u64,
    pub capture_complete: bool,
    pub missing_before: Option<AgentObservationGap>,
    pub provider_session_id: Option<String>,
    pub provider_event_id: Option<String>,
    pub captured_at: i64,
    pub content: AgentObservationContent,
}
impl std::fmt::Debug for AgentObservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AgentObservation(redacted)")
    }
}
impl std::fmt::Debug for AgentObservationContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AgentObservationContent(redacted)")
    }
}
fn reference(v: &str) -> bool {
    !v.trim().is_empty() && v.len() <= 512 && !v.chars().any(char::is_control)
}
fn gap(v: AgentObservationGap) -> bool {
    v.first_sequence > 0
        && v.first_sequence <= v.last_sequence
        && v.last_sequence <= MAX_AGENT_SEQUENCE
        && v.observations == v.last_sequence - v.first_sequence + 1
}
impl AgentObservationContent {
    pub fn supported_by(&self, cap: &super::ObservationCapabilities) -> bool {
        match self {
            Self::Transcript { .. } => cap.transcript,
            Self::Speech { .. } => cap.speech,
            Self::Action { name, .. } => {
                !cap.actions.is_empty() && name.as_ref().is_none_or(|n| cap.actions.contains(n))
            }
            _ => true,
        }
    }
}
impl AgentObservation {
    pub fn validate(&self) -> Result<()> {
        let shape = self.local_sequence > 0
            && self.local_sequence <= MAX_AGENT_SEQUENCE
            && !self.capture_complete
            && self.captured_at >= 0
            && self.captured_at as u64 <= MAX_AGENT_SEQUENCE
            && self.provider_session_id.as_deref().is_none_or(reference)
            && self.provider_event_id.as_deref().is_none_or(reference)
            && self
                .missing_before
                .is_none_or(|v| gap(v) && v.last_sequence < self.local_sequence)
            && match &self.content {
                AgentObservationContent::Speech { state, turn, .. } => {
                    state.as_deref().is_none_or(reference)
                        && turn.is_none_or(|v| v <= MAX_AGENT_SEQUENCE)
                }
                AgentObservationContent::Action { name, call_id, .. } => {
                    name.as_deref().is_none_or(reference)
                        && call_id.as_deref().is_none_or(reference)
                }
                AgentObservationContent::ProviderStatus { state } => reference(state),
                AgentObservationContent::Gap {
                    first_sequence,
                    last_sequence,
                    observations,
                } => {
                    gap(AgentObservationGap {
                        first_sequence: *first_sequence,
                        last_sequence: *last_sequence,
                        observations: *observations,
                    }) && *last_sequence == self.local_sequence
                }
                _ => true,
            };
        if !shape {
            return Err(Error::new(
                "invalid-request",
                "Invalid agent observation metadata",
            ));
        }
        if serde_json::to_vec(self)
            .map_err(|_| Error::storage())?
            .len()
            > MAX_AGENT_OBSERVATION_BYTES
        {
            return Err(Error::new(
                "capacity-exceeded",
                "Agent observation exceeds encoded byte budget",
            ));
        }
        Ok(())
    }
}
