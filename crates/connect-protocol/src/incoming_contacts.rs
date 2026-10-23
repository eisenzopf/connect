//! Private administrative routing metadata. Neither caller IDs nor these views
//! grant membership, history, consent, or provider execution authority.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IncomingContactKind {
    Sms,
    Voice,
}
impl IncomingContactKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sms => "sms",
            Self::Voice => "voice",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IncomingContactReason {
    AmbiguousRoute,
    ResourceConflict,
    HistoricalOverlap,
    Routed,
    Ineligible,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IncomingContactState {
    Held,
    Resolved,
    Rejected,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IncomingContactCandidate {
    pub id: String,
    pub route_id: String,
    pub conversation_id: String,
    pub endpoint_id: String,
    pub participant_id: String,
    pub membership_epoch: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IncomingContactDecision {
    pub operation_id: String,
    pub actor_identity_id: String,
    pub actor_credential_id: String,
    pub candidate_id: Option<String>,
    pub message_id: Option<String>,
    pub session_id: Option<String>,
    pub created_at: i64,
}
/// Metadata only. Held bodies, model context and private provider control URLs
/// are deliberately absent; administration does not imply Conversation history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IncomingContact {
    pub id: String,
    pub kind: IncomingContactKind,
    pub provider: String,
    pub reason: IncomingContactReason,
    pub state: IncomingContactState,
    pub revision: u64,
    pub decision: Option<IncomingContactDecision>,
    pub candidates: Vec<IncomingContactCandidate>,
    pub created_at: i64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IncomingContactPage {
    pub contacts: Vec<IncomingContact>,
    pub next: Option<String>,
}

/// A stable administrative decision identity, retained across uncertain replies.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IncomingContactSelection {
    pub operation_id: String,
    pub contact_id: String,
    pub candidate_id: String,
    pub expected_revision: u64,
}
impl IncomingContactSelection {
    pub fn validate(&self) -> crate::Result<()> {
        for value in [&self.operation_id, &self.contact_id, &self.candidate_id] {
            crate::transitions::id(value)?;
        }
        if self.expected_revision > 1 {
            return Err(crate::Error::new(
                "invalid-request",
                "Invalid incoming contact revision",
            ));
        }
        Ok(())
    }
}

pub const INCOMING_CONTACT_PROFILE: &str = "connect/2";
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum IncomingContactCommand {
    List {
        after: Option<String>,
        limit: u32,
    },
    Get {
        contact_id: String,
    },
    Resolve(IncomingContactSelection),
    Reject {
        operation_id: String,
        contact_id: String,
        expected_revision: u64,
        reason: String,
    },
}
impl IncomingContactCommand {
    pub fn validate(&self) -> crate::Result<()> {
        let invalid = || crate::Error::new("invalid-request", "Invalid incoming contact command");
        match self {
            Self::List { after, limit } => {
                if !(1..=32).contains(limit)
                    || after.as_ref().is_some_and(|s| {
                        s.is_empty() || s.len() > 4096 || s.chars().any(char::is_control)
                    })
                {
                    return Err(invalid());
                }
            }
            Self::Get { contact_id } => crate::transitions::id(contact_id)?,
            Self::Resolve(selection) => selection.validate()?,
            Self::Reject {
                operation_id,
                contact_id,
                expected_revision,
                reason,
            } => {
                for value in [operation_id, contact_id] {
                    crate::transitions::id(value)?;
                }
                if *expected_revision > 1
                    || reason.trim().is_empty()
                    || reason.len() > 256
                    || reason.chars().any(char::is_control)
                {
                    return Err(invalid());
                }
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum IncomingContactOutcome {
    Contact { contact: IncomingContact },
    Page { page: IncomingContactPage },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IncomingContactResponse {
    pub profile: String,
    pub result: crate::Result<IncomingContactOutcome>,
}
