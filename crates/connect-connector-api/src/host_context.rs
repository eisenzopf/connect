//! Exact host context envelopes passed in `DeliveryRequest::context`.
//!
//! These API-only DTOs mirror the public ContextView and task Recall result.
//! They select no data and grant no access. The host journals and authorizes the
//! exact selection before dispatch. Unknown event kinds remain opaque records.
use crate::{lifecycle::ContextItem, Failure};
use serde::{Deserialize, Serialize};

/// A conversation projection or the tagged task recall result, never raw history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum HostContext {
    Conversation(ConversationProjection),
    Task(TaskProjection),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationProjection {
    pub conversation_id: String,
    pub watermark: u64,
    pub items: Vec<JournalItem>,
    pub omitted: u64,
    #[serde(default)]
    pub deliveries: Vec<ContextDelivery>,
    #[serde(default)]
    pub deliveries_omitted: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalItem {
    pub sequence: u64,
    /// Event category, not an instruction role or a general derived-material flag.
    pub kind: String,
    /// Underlying message, session or artifact ID; repeated across journal events.
    pub source_id: String,
    pub actor_id: String,
    /// Unchanged text or serialized session/artifact metadata; never file bytes.
    pub body: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextDelivery {
    pub id: String,
    pub session_id: Option<String>,
    pub provider: String,
    pub watermark: u64,
    pub state: String,
    pub provider_ref: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}
/// Only the recall result is an admissible task context envelope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum TaskProjection {
    Recall {
        sources: Vec<TaskSource>,
        next: Option<String>,
        omitted: u64,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskSource {
    pub source_id: String,
    pub kind: String,
    /// Empty when retained attribution is unavailable; never invent a substitute.
    pub actor_id: String,
    pub created_at: Option<i64>,
    pub provenance: String,
    pub state: String,
    pub body: Option<String>,
}
impl HostContext {
    /// Bound bytes before parsing. Malformed/unsupported envelopes fail before
    /// any provider effect. `None` context is distinct from an empty selection.
    pub fn decode(raw: &str, maximum_bytes: usize) -> Result<Self, Failure> {
        if raw.len() > maximum_bytes {
            return Err(Failure::Rejected);
        }
        serde_json::from_str(raw).map_err(|_| Failure::Rejected)
    }
    /// Copy selected original source bodies with attributable journal references.
    /// Omissions are retained in each item's provenance; callers still retain the
    /// full envelope (including receipts/cursor/empty-page omissions) separately.
    /// Missing task bodies/attribution are explicit in the DTO and fail this
    /// strict body-bearing conversion rather than becoming fabricated text.
    pub fn messaging_items(&self) -> Result<Vec<ContextItem>, Failure> {
        match self {
            Self::Conversation(p) => p
                .items
                .iter()
                .map(|item| {
                    Ok(ContextItem {
                    event_id: format!("journal:{}:{}", item.sequence, item.source_id),
                    author_id: item.actor_id.clone(),
                    source: serde_json::to_string(&serde_json::json!({
                        "kind": item.kind, "source_id": item.source_id,
                        "sequence": item.sequence, "watermark": p.watermark, "omitted": p.omitted,
                    })).map_err(|_| Failure::Rejected)?,
                    derived: derived_kind(&item.kind),
                    body: item.body.clone(),
                })
                })
                .collect(),
            Self::Task(TaskProjection::Recall {
                sources,
                next,
                omitted,
            }) => sources
                .iter()
                .map(|item| {
                    if item.state != "available" || item.actor_id.is_empty() {
                        return Err(Failure::Rejected);
                    }
                    let event = item
                        .provenance
                        .split(';')
                        .next()
                        .filter(|value| {
                            value.starts_with("conversation-journal:")
                                && value.len() > "conversation-journal:".len()
                        })
                        .ok_or(Failure::Rejected)?;
                    Ok(ContextItem {
                        event_id: event.into(),
                        author_id: item.actor_id.clone(),
                        source: serde_json::to_string(&serde_json::json!({
                            "kind": item.kind, "source_id": item.source_id, "state": item.state,
                            "created_at": item.created_at, "provenance": item.provenance,
                            "omitted": omitted, "continuation_available": next.is_some(),
                        }))
                        .map_err(|_| Failure::Rejected)?,
                        derived: derived_kind(&item.kind),
                        body: item.body.clone().ok_or(Failure::Rejected)?,
                    })
                })
                .collect(),
        }
    }
}
/// This is an explicit mapping for existing provider outcome records, which can
/// contain provider-generated analysis. It never guesses from arbitrary body text
/// or labels such as "summary". Transcript and context-gap bodies stay unchanged;
/// final excerpts and explicit incomplete markers do not promise a full transcript.
fn derived_kind(kind: &str) -> bool {
    matches!(kind, "voice_outcome" | "voice_outcome_correction")
}
