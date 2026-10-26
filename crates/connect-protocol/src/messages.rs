//! Explicit addressed messages. A draft has no authoritative sender field.
use crate::{Error, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum MessageAudience {
    All(AllAudience),
    Participants(Vec<String>),
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum AllAudience {
    #[serde(rename = "all")]
    All,
}
impl MessageAudience {
    pub fn all() -> Self {
        Self::All(AllAudience::All)
    }
    pub fn normalize(&mut self) -> Result<()> {
        if let Self::Participants(ids) = self {
            if ids.is_empty() || ids.len() > 100 {
                return Err(invalid());
            }
            for id in ids.iter() {
                identifier(id)?;
            }
            ids.sort();
            if ids.windows(2).any(|p| p[0] == p[1]) {
                return Err(invalid());
            }
        }
        Ok(())
    }
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BodyEncoding {
    #[default]
    Utf8,
    Base64,
}
impl BodyEncoding {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Utf8 => "utf8",
            Self::Base64 => "base64",
        }
    }
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AttachmentRef {
    pub id: String,
    pub content_type: String,
    pub size_bytes: u64,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MessageDraft {
    pub to: MessageAudience,
    pub content_type: String,
    pub body: String,
    pub body_encoding: BodyEncoding,
    pub label: String,
    pub attachments: Vec<AttachmentRef>,
    pub in_reply_to_msg: Option<String>,
    /// Canonical clients can choose their message identity independently of
    /// the logical operation ID. Other clients use the accepted server ID.
    pub msg_id: Option<String>,
}
impl MessageDraft {
    pub fn text(to: MessageAudience, body: impl Into<String>) -> Self {
        Self {
            to,
            body: body.into(),
            content_type: "text/plain".into(),
            body_encoding: BodyEncoding::Utf8,
            label: "connect-messages".into(),
            attachments: vec![],
            in_reply_to_msg: None,
            msg_id: None,
        }
    }
    pub fn text_all(body: impl Into<String>) -> Self {
        Self::text(MessageAudience::all(), body)
    }
    pub fn normalize(&mut self) -> Result<()> {
        if self.body.len() > crate::MAX_PAGE_BYTES {
            return Err(invalid());
        }
        self.to.normalize()?;
        media_type(&self.content_type)?;
        identifier(&self.label)?;
        if let Some(id) = &self.msg_id {
            identifier(id)?;
        }
        if let Some(id) = &self.in_reply_to_msg {
            identifier(id)?;
        }
        if self.attachments.len() > 16 {
            return Err(invalid());
        }
        for a in &self.attachments {
            identifier(&a.id)?;
            media_type(&a.content_type)?;
            if a.size_bytes > 9_007_199_254_740_991 {
                return Err(invalid());
            }
        }
        self.attachments.sort_by(|a, b| a.id.cmp(&b.id));
        if self.attachments.windows(2).any(|p| p[0].id == p[1].id) {
            return Err(invalid());
        }
        if self.body_encoding == BodyEncoding::Base64 {
            let bytes = STANDARD.decode(&self.body).map_err(|_| invalid())?;
            if STANDARD.encode(&bytes) != self.body {
                return Err(invalid());
            }
        }
        Ok(())
    }
    pub fn bytes(&self) -> Result<Vec<u8>> {
        match self.body_encoding {
            BodyEncoding::Utf8 => Ok(self.body.as_bytes().to_vec()),
            BodyEncoding::Base64 => STANDARD.decode(&self.body).map_err(|_| invalid()),
        }
    }
    pub fn connector_text(&self) -> bool {
        self.body_encoding == BodyEncoding::Utf8
            && self.content_type == "text/plain"
            && self.attachments.is_empty()
    }
}
impl std::fmt::Debug for MessageDraft {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MessageDraft(redacted)")
    }
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageRecipient {
    pub participant_id: String,
    pub membership_epoch: u64,
}
pub(crate) fn identifier(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._~-".contains(&b))
    {
        Err(invalid())
    } else {
        Ok(())
    }
}
fn media_type(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value.is_ascii()
        || value.bytes().any(|b| b.is_ascii_control())
        || value.split(';').next().is_none_or(|s| {
            s.split('/').count() != 2
                || s.split('/').any(|v| {
                    v.is_empty()
                        || !v
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"!#$&^_.+-".contains(&b))
                })
        })
    {
        Err(invalid())
    } else {
        Ok(())
    }
}
fn invalid() -> Error {
    Error::new("invalid-request", "Invalid addressed message")
}

/// Filters are applied inside the authorized history query, before pagination.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryQuery {
    pub since: Option<i64>,
    pub until: Option<i64>,
    pub since_msg_id: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
    pub include_attachments: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sets_normalize_without_equating_changed_content() {
        let mut draft = MessageDraft::text(
            MessageAudience::Participants(vec!["part_b".into(), "part_a".into()]),
            "private",
        );
        draft.attachments = vec![
            AttachmentRef {
                id: "art_b".into(),
                content_type: "image/png".into(),
                size_bytes: 2,
            },
            AttachmentRef {
                id: "art_a".into(),
                content_type: "image/png".into(),
                size_bytes: 1,
            },
        ];
        draft.normalize().expect("valid");
        assert_eq!(
            draft.to,
            MessageAudience::Participants(vec!["part_a".into(), "part_b".into()])
        );
        assert_eq!(draft.attachments[0].id, "art_a");
        let mut changed = draft.clone();
        changed.label = "other".into();
        assert_ne!(
            serde_json::to_value(&draft).expect("json"),
            serde_json::to_value(&changed).expect("json")
        );
        assert!(!format!("{draft:?}").contains("private"));
    }
    #[test]
    fn binary_encoding_is_strict_and_round_trips_all_bytes() {
        let bytes = (0..=255).collect::<Vec<u8>>();
        let mut draft = MessageDraft::text_all(STANDARD.encode(&bytes));
        draft.body_encoding = BodyEncoding::Base64;
        draft.content_type = "application/octet-stream".into();
        draft.normalize().expect("valid binary");
        assert_eq!(draft.bytes().expect("bytes"), bytes);
        for body in ["AA", "AB==", "-w==", "AA==\n", "☃"] {
            draft.body = body.into();
            assert!(draft.normalize().is_err());
        }
    }
    #[test]
    fn malformed_sets_metadata_and_claimed_authority_are_rejected() {
        let good = MessageDraft::text_all("hello");
        let base = serde_json::to_value(&good).expect("fixture");
        for field in [
            "sender",
            "from",
            "owner_id",
            "credential_id",
            "membership_epoch",
        ] {
            let mut value = base.clone();
            value[field] = serde_json::json!("claimed");
            assert!(serde_json::from_value::<MessageDraft>(value).is_err());
        }
        for to in [
            MessageAudience::Participants(vec![]),
            MessageAudience::Participants(vec!["part_a".into(), "part_a".into()]),
            MessageAudience::Participants(vec!["foreign/id".into()]),
        ] {
            let mut draft = good.clone();
            draft.to = to;
            assert!(draft.normalize().is_err());
        }
        for content_type in ["text", "text/", "text/plain\n", "text /plain", "☃/snow"] {
            let mut draft = good.clone();
            draft.content_type = content_type.into();
            assert!(draft.normalize().is_err());
        }
        let mut draft = good;
        draft.attachments = vec![
            AttachmentRef {
                id: "art".into(),
                content_type: "image/png".into(),
                size_bytes: 1
            };
            2
        ];
        assert!(draft.normalize().is_err());
    }
}
