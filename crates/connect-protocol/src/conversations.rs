//! Shared conversation intents and bounded current views.
use crate::{provisioning::IdentityKind, Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub const MAX_METADATA_BYTES: usize = 16 * 1024;
pub const MAX_IDLE_SECONDS: u64 = 7 * 86400;
pub const DEFAULT_IDLE_SECONDS: u64 = 60;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConversationPolicy {
    Persistent,
    Ephemeral,
}
impl ConversationPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Persistent => "persistent",
            Self::Ephemeral => "ephemeral",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct InitialParticipant {
    pub identity_id: String,
    pub role: String,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRequest {
    pub tenant_id: Option<String>,
    pub policy: ConversationPolicy,
    pub idle_close_secs: Option<u64>,
    #[serde(default, deserialize_with = "metadata::deserialize")]
    pub metadata: Option<Value>,
    pub initial_participants: Vec<InitialParticipant>,
}
impl CreateRequest {
    /// A public convenience for the common persistent conversation. All wire
    /// fields are still explicit in the resulting shared typed request.
    pub fn persistent(mut identities: Vec<String>) -> Self {
        identities.sort();
        identities.dedup();
        Self {
            tenant_id: None,
            policy: ConversationPolicy::Persistent,
            idle_close_secs: None,
            metadata: None,
            initial_participants: identities
                .into_iter()
                .map(|identity_id| InitialParticipant {
                    identity_id,
                    role: "participant".into(),
                })
                .collect(),
        }
    }
    pub fn normalize(&mut self) -> Result<()> {
        if let Some(tenant) = &self.tenant_id {
            crate::transitions::id(tenant)?;
        }
        if self.policy == ConversationPolicy::Ephemeral && self.idle_close_secs.is_none() {
            self.idle_close_secs = Some(DEFAULT_IDLE_SECONDS);
        }
        if self
            .idle_close_secs
            .is_some_and(|s| s == 0 || s > MAX_IDLE_SECONDS)
        {
            return Err(invalid());
        }
        if self.initial_participants.len() > 128 {
            return Err(invalid());
        }
        self.initial_participants.sort();
        let mut last = None;
        for p in &self.initial_participants {
            crate::transitions::id(&p.identity_id)?;
            role(&p.role)?;
            if last == Some(&p.identity_id) {
                return Err(invalid());
            }
            last = Some(&p.identity_id);
        }
        if let Some(value) = &self.metadata {
            metadata::validate(value, 0)?;
            let json = serde_jcs::to_string(value).map_err(|_| invalid())?;
            if json.len() > MAX_METADATA_BYTES {
                return Err(invalid());
            }
            self.metadata = Some(serde_json::from_str(&json).map_err(|_| invalid())?);
        }
        Ok(())
    }
}
impl std::fmt::Debug for CreateRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CreateRequest(redacted)")
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloseOptions {
    pub expected_revision: Option<u64>,
    pub reason_code: Option<u64>,
    pub reason: Option<String>,
}
impl CloseOptions {
    pub fn validate(&self) -> Result<()> {
        if self
            .expected_revision
            .is_some_and(|v| v >= 9_007_199_254_740_991)
            || self.reason_code.is_some_and(|v| v > 9_007_199_254_740_991)
            || self
                .reason
                .as_ref()
                .is_some_and(|r| r.len() > 512 || r.chars().any(char::is_control))
        {
            return Err(invalid());
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConversationState {
    #[default]
    Open,
    Closed,
    All,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListFilter {
    pub tenant_id: Option<String>,
    pub participant_id: Option<String>,
    pub identity_id: Option<String>,
    #[serde(default)]
    pub state: ConversationState,
    pub since: Option<chrono::DateTime<chrono::Utc>>,
    pub until: Option<chrono::DateTime<chrono::Utc>>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListQuery {
    #[serde(default)]
    pub filter: ListFilter,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}
impl ListQuery {
    pub fn validate(&self) -> Result<()> {
        for id in [
            &self.filter.tenant_id,
            &self.filter.participant_id,
            &self.filter.identity_id,
        ]
        .into_iter()
        .flatten()
        {
            crate::transitions::id(id)?;
        }
        if self.limit.is_some_and(|v| v == 0 || v > 100)
            || self.cursor.as_ref().is_some_and(|v| v.len() > 2048)
            || self
                .filter
                .since
                .zip(self.filter.until)
                .is_some_and(|(a, b)| a > b)
        {
            return Err(invalid());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Participant {
    pub participant_id: String,
    pub identity_id: String,
    pub kind: IdentityKind,
    pub role: String,
    pub display_name: Option<String>,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Conversation {
    pub id: String,
    pub tenant_id: String,
    pub policy: ConversationPolicy,
    pub idle_close_secs: Option<u64>,
    pub metadata: Option<Value>,
    pub participants: Vec<Participant>,
    pub opened_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt: Option<crate::transitions::MutationReceipt>,
    pub revision: u64,
    pub closed: bool,
    pub closed_at: Option<i64>,
    pub reason_code: Option<u64>,
    pub reason: Option<String>,
}
impl std::fmt::Debug for Conversation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Conversation")
            .field("id", &self.id)
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListPage {
    pub conversations: Vec<Conversation>,
    pub next_cursor: Option<String>,
}
fn role(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 64 || value.chars().any(char::is_control) {
        Err(invalid())
    } else {
        Ok(())
    }
}
fn invalid() -> Error {
    Error::new("invalid-request", "Invalid conversation request")
}
mod metadata {
    use super::*;
    use serde::de::{self, MapAccess, SeqAccess, Visitor};
    struct Strict(Value);
    impl<'de> Deserialize<'de> for Strict {
        fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
            d.deserialize_any(Json)
        }
    }
    struct Json;
    impl<'de> Visitor<'de> for Json {
        type Value = Strict;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("JSON metadata with unique keys")
        }
        fn visit_bool<E: de::Error>(self, v: bool) -> std::result::Result<Strict, E> {
            Ok(Strict(Value::Bool(v)))
        }
        fn visit_i64<E: de::Error>(self, v: i64) -> std::result::Result<Strict, E> {
            Ok(Strict(Value::Number(v.into())))
        }
        fn visit_u64<E: de::Error>(self, v: u64) -> std::result::Result<Strict, E> {
            Ok(Strict(Value::Number(v.into())))
        }
        fn visit_f64<E: de::Error>(self, v: f64) -> std::result::Result<Strict, E> {
            serde_json::Number::from_f64(v)
                .map(|v| Strict(Value::Number(v)))
                .ok_or_else(|| E::custom("Nonfinite metadata"))
        }
        fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<Strict, E> {
            Ok(Strict(Value::String(v.into())))
        }
        fn visit_string<E: de::Error>(self, v: String) -> std::result::Result<Strict, E> {
            Ok(Strict(Value::String(v)))
        }
        fn visit_unit<E: de::Error>(self) -> std::result::Result<Strict, E> {
            Ok(Strict(Value::Null))
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> std::result::Result<Strict, A::Error> {
            let mut v = vec![];
            while let Some(Strict(x)) = a.next_element()? {
                v.push(x);
            }
            Ok(Strict(Value::Array(v)))
        }
        fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> std::result::Result<Strict, A::Error> {
            let mut v = serde_json::Map::new();
            while let Some(k) = a.next_key::<String>()? {
                if v.contains_key(&k) {
                    return Err(de::Error::custom("Duplicate metadata key"));
                }
                let Strict(x) = a.next_value()?;
                v.insert(k, x);
            }
            Ok(Strict(Value::Object(v)))
        }
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        d: D,
    ) -> std::result::Result<Option<Value>, D::Error> {
        Ok(Option::<Strict>::deserialize(d)?.map(|v| v.0))
    }
    pub fn validate(v: &Value, depth: usize) -> Result<()> {
        if depth > 16 {
            return Err(invalid());
        }
        match v {
            Value::Number(n)
                if n.as_f64()
                    .is_some_and(|v| v.fract() == 0.0 && v.abs() > 9_007_199_254_740_991.0)
                    || n.as_u64().is_some_and(|v| v > 9_007_199_254_740_991)
                    || n.as_i64().is_some_and(|v| v < -9_007_199_254_740_991) =>
            {
                Err(invalid())
            }
            Value::Array(a) => {
                for v in a {
                    validate(v, depth + 1)?;
                }
                Ok(())
            }
            Value::Object(a) => {
                for v in a.values() {
                    validate(v, depth + 1)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}
