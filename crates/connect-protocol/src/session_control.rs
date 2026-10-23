//! Canonical Session intent. Participant IDs are assertions, never credentials.
use crate::{transitions::id, Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub operation_id: String,
    pub conversation_id: String,
    pub session_id: String,
    pub by_participant: Option<String>,
    pub expected_revision: Option<u64>,
    pub action: Action,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Action {
    Invite {
        participants: Vec<String>,
        medium: String,
        intent: String,
        capabilities: Value,
    },
    Accept {
        capabilities: Value,
    },
    Reject {
        reason_code: u64,
        reason: String,
    },
    Cancel {
        reason_code: u64,
        reason: String,
    },
    Leave {
        participant_id: String,
    },
    Role {
        participant_id: String,
        role: String,
    },
    Upgrade {
        media: Vec<String>,
    },
    Renegotiate {
        codec_preferences: Vec<String>,
    },
    End {
        reason_code: u64,
        reason: String,
    },
}
impl Action {
    pub fn required_scope(&self) -> &'static str {
        match self {
            Self::Invite { .. } => "session.invite",
            Self::Accept { .. } => "session.accept",
            Self::Reject { .. } => "session.reject",
            Self::Cancel { .. } => "session.cancel",
            Self::End { .. } => "session.end",
            _ => "session.update",
        }
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::Leave { .. } => "participant-leave",
            Self::Role { .. } => "role-change",
            Self::Upgrade { .. } => "medium-upgrade",
            Self::Renegotiate { .. } => "codec-renegotiate",
            _ => self.required_scope(),
        }
    }
    pub fn reason(&self) -> (u64, &str) {
        match self {
            Self::Reject {
                reason_code,
                reason,
            }
            | Self::Cancel {
                reason_code,
                reason,
            }
            | Self::End {
                reason_code,
                reason,
            } => (*reason_code, reason),
            _ => (0, ""),
        }
    }
}
impl Request {
    pub fn normalize(&mut self) -> Result<()> {
        for value in [&self.operation_id, &self.conversation_id, &self.session_id] {
            id(value)?;
        }
        if let Some(by) = &self.by_participant {
            id(by)?;
        }
        if self
            .expected_revision
            .is_some_and(|r| r > 9_007_199_254_740_991)
        {
            return Err(invalid());
        }
        let (code, reason) = self.action.reason();
        if code > 65535 || reason.len() > 256 || reason.chars().any(char::is_control) {
            return Err(invalid());
        }
        match &mut self.action {
            Action::Invite {
                participants,
                medium,
                intent,
                capabilities,
            } => {
                // The initiator is implicit. An empty target set admits a
                // single-Participant Session; Store decides whether it is new.
                if participants.len() > 256
                    || intent.is_empty()
                    || intent.len() > 128
                    || intent.chars().any(char::is_control)
                {
                    return Err(invalid());
                }
                media(medium)?;
                for value in participants.iter() {
                    id(value)?;
                }
                participants.sort();
                if participants.windows(2).any(|p| p[0] == p[1]) {
                    return Err(invalid());
                }
                validate_capabilities(capabilities)?;
            }
            Action::Accept { capabilities } => validate_capabilities(capabilities)?,
            Action::Leave { participant_id } => id(participant_id)?,
            Action::Role {
                participant_id,
                role,
            } => {
                id(participant_id)?;
                if role.is_empty() || role.len() > 64 || role.chars().any(char::is_control) {
                    return Err(invalid());
                }
            }
            Action::Upgrade { media } => {
                if media.is_empty()
                    || media.len() > 4
                    || media
                        .iter()
                        .any(|m| !matches!(m.as_str(), "audio" | "video" | "screen" | "data"))
                {
                    return Err(invalid());
                }
                media.sort();
                if media.windows(2).any(|m| m[0] == m[1]) {
                    return Err(invalid());
                }
            }
            Action::Renegotiate { codec_preferences } => validate_names(codec_preferences)?,
            _ => (),
        }
        Ok(())
    }
}
pub fn media(medium: &str) -> Result<Vec<String>> {
    Ok(match medium {
        "voice" => vec!["audio"],
        "video" => vec!["video"],
        "voice+video" => vec!["audio", "video"],
        "screen-share" => vec!["screen"],
        "text-chat" => vec!["data"],
        "mixed" => vec!["audio", "data", "screen", "video"],
        _ => return Err(invalid()),
    }
    .into_iter()
    .map(str::to_owned)
    .collect())
}
fn validate_names(names: &[String]) -> Result<()> {
    if names.is_empty()
        || names.len() > 16
        || names.iter().any(|n| {
            n.is_empty()
                || n.len() > 64
                || !n
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._/-".contains(&b))
        })
        || names.iter().collect::<std::collections::HashSet<_>>().len() != names.len()
    {
        return Err(invalid());
    }
    Ok(())
}
pub fn validate_capabilities(value: &Value) -> Result<()> {
    if !value.is_object() || serde_json::to_vec(value).map_err(|_| invalid())?.len() > 16384 {
        return Err(invalid());
    }
    for kind in ["audio", "video", "screen", "data"] {
        if let Some(values) = value.get(kind) {
            let names: Vec<String> =
                serde_json::from_value(values.clone()).map_err(|_| invalid())?;
            validate_names(&names)?;
        }
    }
    Ok(())
}
/// Current shared Session metadata; live appearances and Connections are separate
/// scoped facts. No initiator/counterpart pair defines the Participant set.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionView {
    pub id: String,
    pub conversation_id: String,
    pub state: String,
    pub media: Vec<String>,
    pub intent: Option<String>,
    pub capabilities_offer: Option<Value>,
    pub codec_preferences: Vec<String>,
    pub is_main: bool,
    pub revision: u64,
    pub topology_revision: u64,
    pub created_at: i64,
    pub updated_at: i64,
    pub ended_reason: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlOutcome {
    pub operation_id: String,
    pub action: String,
    pub session: SessionView,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codec_reconciliation: Option<crate::session_codecs::ReconciliationReference>,
}
impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionControl")
            .field("operation_id", &self.operation_id)
            .field("action", &self.action.name())
            .finish_non_exhaustive()
    }
}
fn invalid() -> Error {
    Error::new("invalid-request", "Invalid Session command")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn screen_is_a_distinct_bounded_accepted_media_kind() {
        let mut request = Request {
            operation_id: "upgrade-screen".into(),
            conversation_id: "conversation".into(),
            session_id: "session".into(),
            by_participant: None,
            expected_revision: Some(1),
            action: Action::Upgrade {
                media: vec![
                    "screen".into(),
                    "audio".into(),
                    "data".into(),
                    "video".into(),
                ],
            },
        };
        request.normalize().unwrap();
        assert!(
            matches!(&request.action,Action::Upgrade{media} if media==&vec!["audio","data","screen","video"])
        );
        assert_eq!(
            media("mixed").unwrap(),
            vec!["audio", "data", "screen", "video"]
        );
        validate_capabilities(&json!({"screen":["vp8"],"future":{"retain":true}})).unwrap();
        for caps in [
            json!({"screen":[]}),
            json!({"screen":"vp8"}),
            json!({"screen":["vp8","vp8"]}),
            json!({"screen":["vp8",null]}),
        ] {
            assert!(validate_capabilities(&caps).is_err());
        }
        request.action = Action::Upgrade {
            media: vec!["screen".into(), "screen".into()],
        };
        assert!(request.normalize().is_err());
    }
}
