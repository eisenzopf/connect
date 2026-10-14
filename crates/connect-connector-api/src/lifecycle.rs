//! Normalized observations. Messaging and call lifecycle remain separate.
//!
//! Only trusted adapter composition constructs observations after signature or
//! authenticated response verification. External IDs and caller fields identify
//! effects, never principals. The host must recheck current authorization when
//! applying/replaying an observation and journal it transactionally with effects.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageState {
    Accepted,
    Delivered,
    Failed,
    Uncertain,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VoiceState {
    Starting,
    Active,
    Ended,
    Failed,
    Uncertain,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum State {
    Message(MessageState),
    Voice(VoiceState),
}
impl State {
    pub fn terminal(self) -> bool {
        matches!(
            self,
            Self::Message(MessageState::Delivered | MessageState::Failed)
                | Self::Voice(VoiceState::Ended | VoiceState::Failed)
        )
    }
    fn uncertain(self) -> bool {
        matches!(
            self,
            Self::Message(MessageState::Uncertain) | Self::Voice(VoiceState::Uncertain)
        )
    }
    fn rank(self) -> u8 {
        match self {
            Self::Message(MessageState::Accepted) | Self::Voice(VoiceState::Starting) => 0,
            Self::Voice(VoiceState::Active) => 1,
            _ => 2,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verification {
    LocalAttempt,
    AuthenticatedResponse,
    VerifiedCallback,
    Reconciliation,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub mutation_id: String,
    pub external_id: Option<String>,
    pub event_id: String,
    pub state: State,
    pub verification: Verification,
}
/// Normalize a single legacy adapter attempt without strengthening delivery
/// guarantees. A synchronous reply means completion to the host, while the
/// provider receipt itself remains acceptance. Transport failures are local
/// observations, not authenticated provider statements.
pub fn message_attempt(
    mutation_id: &str,
    result: Result<crate::Completion, crate::Failure>,
) -> MessageReceipt {
    let (external_id, state, reply, verification) = match result {
        Ok(done) if !identifier(&done.provider_id) => (
            None,
            MessageState::Uncertain,
            None,
            Verification::LocalAttempt,
        ),
        Ok(done) => (
            Some(done.provider_id),
            MessageState::Accepted,
            done.reply,
            Verification::AuthenticatedResponse,
        ),
        Err(crate::Failure::Rejected) => {
            (None, MessageState::Failed, None, Verification::LocalAttempt)
        }
        Err(crate::Failure::Uncertain) => (
            None,
            MessageState::Uncertain,
            None,
            Verification::LocalAttempt,
        ),
    };
    MessageReceipt {
        observation: Observation {
            mutation_id: mutation_id.into(),
            external_id,
            event_id: format!("attempt:{mutation_id}"),
            state: State::Message(state),
            verification,
        },
        reply,
        reply_sources: vec![],
    }
}
/// Authorized historical data separate from the current input. The host selects
/// and bounds items; no projection can grant authority or become a system prompt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextItem {
    pub event_id: String,
    pub author_id: String,
    pub source: String,
    pub derived: bool,
    pub body: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageRequest {
    pub mutation_id: String,
    pub target: String,
    pub body: String,
    pub context: Vec<ContextItem>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageReceipt {
    pub observation: Observation,
    pub reply: Option<String>,
    /// References selected source events, not inferred sources or provider authority.
    pub reply_sources: Vec<String>,
}
/// An independently created provider message, authenticated by the adapter.
/// Trusted composition maps it to an existing route; these IDs grant no rights.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IncomingMessage {
    pub mutation_id: String,
    pub external_id: String,
    pub body: String,
    pub verification: Verification,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceRequest {
    pub mutation_id: String,
    pub destination: String,
}
#[async_trait::async_trait]
pub trait MessageExtension: Send + Sync {
    /// Read authoritative status for the original logical effect. Never create
    /// a second send to discover whether the first succeeded.
    async fn message_status(&self, _mutation_id: &str) -> Result<MessageReceipt, crate::Failure> {
        Err(crate::Failure::Rejected)
    }
    async fn send(&self, request: &MessageRequest) -> Result<MessageReceipt, crate::Failure>;
}
#[async_trait::async_trait]
pub trait MediaExtension: Send + Sync {
    /// One creation attempt. Uncertain creation must be reconciled, never retried blindly.
    async fn open(&self, request: &VoiceRequest) -> Result<Observation, crate::Failure>;
    async fn reconcile(&self, mutation_id: &str) -> Result<Observation, crate::Failure>;
    /// Idempotent teardown by logical mutation. A cancelled create can have succeeded remotely.
    async fn close(&self, mutation_id: &str) -> Result<Observation, crate::Failure>;
    /// Bounded decoded PCM frame exchange for the declared format. Real providers
    /// implement their rvoip media/session interface behind this fixture boundary.
    async fn exchange(&self, mutation_id: &str, pcm: &[i16]) -> Result<Vec<i16>, crate::Failure>;
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Projection {
    pub mutation_id: String,
    pub external_id: Option<String>,
    pub state: State,
    /// Bounded conflict detection. Capacity exhaustion requires host reconciliation,
    /// not eviction that would make an old callback new again.
    events: BTreeMap<String, Observation>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Apply {
    Applied,
    Duplicate,
    Older,
}
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ObservationError {
    #[error("current authorization required")]
    Unauthorized,
    #[error("invalid or mismatched connector observation")]
    Invalid,
    #[error("conflicting terminal or event observation; reconcile")]
    Conflict,
    #[error("uncertain effect requires reconciliation")]
    Uncertain,
    #[error("observation retention capacity reached")]
    Capacity,
}
fn identifier(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}
impl Projection {
    pub fn new(mutation_id: String, state: State) -> Result<Self, ObservationError> {
        if !identifier(&mutation_id) {
            return Err(ObservationError::Invalid);
        }
        Ok(Self {
            mutation_id,
            external_id: None,
            state,
            events: BTreeMap::new(),
        })
    }
    /// `authorized` must be supplied from the host's current policy check, including
    /// expiry/revocation. It is deliberately checked before duplicate detection.
    pub fn apply(
        &mut self,
        event: Observation,
        authorized: bool,
    ) -> Result<Apply, ObservationError> {
        if !authorized {
            return Err(ObservationError::Unauthorized);
        }
        if event.mutation_id != self.mutation_id
            || !identifier(&event.event_id)
            || event
                .external_id
                .as_deref()
                .is_some_and(|id| !identifier(id))
            || (event.external_id.is_none()
                && !matches!(
                    event.state,
                    State::Message(MessageState::Failed | MessageState::Uncertain)
                        | State::Voice(VoiceState::Failed | VoiceState::Uncertain)
                ))
            || std::mem::discriminant(&event.state) != std::mem::discriminant(&self.state)
            || (self.external_id.is_some()
                && event.external_id.is_some()
                && self.external_id != event.external_id)
        {
            return Err(ObservationError::Invalid);
        }
        if let Some(prior) = self.events.get(&event.event_id) {
            return if prior == &event {
                Ok(Apply::Duplicate)
            } else {
                Err(ObservationError::Conflict)
            };
        }
        if self.events.len() >= 256 {
            return Err(ObservationError::Capacity);
        }
        let terminal_reconciliation = self.state.terminal()
            && event.state.terminal()
            && event.state != self.state
            && event.verification == Verification::Reconciliation;
        if self.state.terminal()
            && event.state.terminal()
            && event.state != self.state
            && !terminal_reconciliation
        {
            return Err(ObservationError::Conflict);
        }
        if self.state.uncertain() && event.verification != Verification::Reconciliation {
            return Err(ObservationError::Uncertain);
        }
        let older = (self.state.terminal() && !terminal_reconciliation)
            || (!self.state.uncertain()
                && !event.state.uncertain()
                && event.state.rank() < self.state.rank());
        if !older {
            self.state = event.state;
        }
        if self.external_id.is_none() {
            self.external_id = event.external_id.clone();
        }
        self.events.insert(event.event_id.clone(), event);
        Ok(if older { Apply::Older } else { Apply::Applied })
    }
}
