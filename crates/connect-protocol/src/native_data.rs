//! Exact native application-message facts. Parsing these facts grants no IO.
use crate::{transitions::id, Error, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::Value;
use std::collections::BTreeSet;

/// Matches the native engine's decoded application-message contract. Large
/// files use artifact transfer; the signed envelope has independent headroom.
pub const MAX_BODY_BYTES: usize = 64 * 1024;
/// Local writer facts, never a remote delivery or consumption receipt.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteDisposition {
    NotStarted,
    LocalWritten,
    SubmissionUnknown,
}
impl WriteDisposition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotStarted => "not_started",
            Self::LocalWritten => "local_written",
            Self::SubmissionUnknown => "submission_unknown",
        }
    }
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteProvenance {
    LocalWriter,
    RuntimeRecovery,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WriteObservation {
    pub message_id: String,
    pub source_connection_id: String,
    pub target_connection_id: String,
    pub participant_id: String,
    pub membership_epoch: u64,
    pub generation: u64,
    pub disposition: WriteDisposition,
    pub provenance: WriteProvenance,
    pub observed_at: i64,
}
#[derive(Clone, PartialEq, Eq)]
pub struct Message {
    pub message_id: String,
    pub label: String,
    pub from: Option<String>,
    /// None denotes the explicit "all" audience; order is retained in intent.
    pub recipients: Option<Vec<String>>,
    pub content_type: String,
    pub bytes: Vec<u8>,
}
impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeDataMessage(redacted)")
    }
}
fn invalid() -> Error {
    Error::new("invalid-request", "Invalid native application message")
}
fn required<'a>(payload: &'a Value, field: &str) -> Result<&'a str> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(invalid)
}
pub fn message(payload: &Value) -> Result<Message> {
    if !payload.is_object() {
        return Err(invalid());
    }
    let message_id = required(payload, "msg_id")?;
    let label = required(payload, "label")?;
    id(message_id)?;
    id(label)?;
    let from = payload
        .get("from")
        .map(|v| v.as_str().ok_or_else(invalid))
        .transpose()?;
    if let Some(from) = from {
        id(from)?;
    }
    let recipients = match payload.get("to") {
        Some(Value::String(value)) if value == "all" => None,
        Some(Value::Array(values)) if !values.is_empty() && values.len() <= 256 => {
            let mut seen = BTreeSet::new();
            let mut participants = Vec::with_capacity(values.len());
            for value in values {
                let pid = value.as_str().ok_or_else(invalid)?;
                id(pid)?;
                if !seen.insert(pid) {
                    return Err(invalid());
                }
                participants.push(pid.into());
            }
            Some(participants)
        }
        _ => return Err(invalid()),
    };
    let content_type = required(payload, "content_type")?;
    // MIME syntax follows the transport-neutral native data contract.
    let mut sections = content_type.split(';');
    let media = sections.next().unwrap_or_default().trim();
    let token = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
    };
    let valid_media = media
        .split_once('/')
        .is_some_and(|(kind, subtype)| token(kind) && token(subtype));
    if content_type.is_empty()
        || content_type.len() > 255
        || content_type.chars().any(char::is_control)
        || !valid_media
        || !sections.all(|part| {
            !part.trim().is_empty()
                && part
                    .chars()
                    .all(|ch| ch.is_ascii() && !ch.is_ascii_control())
        })
    {
        return Err(invalid());
    }
    if payload
        .get("attachments")
        .is_some_and(|a| !a.as_array().is_some_and(Vec::is_empty))
        || payload.get("in_reply_to_msg").is_some_and(|v| !v.is_null())
    {
        return Err(invalid());
    }
    if let Some(reliability) = payload.get("reliability") {
        if !reliability.is_object() || reliability["mode"] != "reliable_ordered" {
            return Err(invalid());
        }
    }
    let body = required(payload, "body")?;
    if body.len() > 4 * MAX_BODY_BYTES / 3 + 4 {
        return Err(invalid());
    }
    let bytes = match payload.get("body_encoding") {
        None => body.as_bytes().to_vec(),
        Some(Value::String(value)) if value == "utf8" => body.as_bytes().to_vec(),
        Some(Value::String(value)) if value == "base64" => {
            let bytes = STANDARD.decode(body).map_err(|_| invalid())?;
            if STANDARD.encode(&bytes) != body {
                return Err(invalid());
            }
            bytes
        }
        _ => return Err(invalid()),
    };
    if bytes.len() > MAX_BODY_BYTES {
        return Err(invalid());
    }
    Ok(Message {
        message_id: message_id.into(),
        label: label.into(),
        from: from.map(str::to_owned),
        recipients,
        content_type: content_type.into(),
        bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_streams::{Command, Intent};
    use serde_json::json;
    fn wire() -> Intent {
        Intent {
            operation_id: "publish-data".into(),
            conversation_id: "conversation".into(),
            session_id: "session".into(),
            connection_id: "source".into(),
            expected_revision: None,
            by_participant: Some("sender".into()),
            command: Command::Data,
            payload: json!({"msg_id":"message","from":"sender","to":["second","first"],"label":"application","content_type":"application/octet-stream","body_encoding":"base64","body":"AP8=","reliability":{"mode":"reliable_ordered","future":true},"future":{"keep":"original"}}),
        }
    }
    #[test]
    fn exact_native_data_intent_preserves_scope_encoding_audience_order_and_extensions() {
        let wire = wire();
        wire.validate().unwrap();
        let stored: Intent = serde_json::from_slice(&serde_json::to_vec(&wire).unwrap()).unwrap();
        assert_eq!(wire, stored);
        let parsed = message(&stored.payload).unwrap();
        assert_eq!(parsed.bytes, vec![0, 255]);
        assert_eq!(
            parsed.recipients,
            Some(vec!["second".into(), "first".into()])
        );
        for field in ["msg_id", "label", "body", "to", "future"] {
            let mut changed = wire.clone();
            changed.payload[field] = json!("different");
            assert_ne!(changed, wire);
        }
        assert!(!format!("{parsed:?}").contains("application"));
    }
    #[test]
    fn decoded_body_limits_apply_to_utf8_and_canonical_binary_independently_of_json_length() {
        let mut wire = wire();
        wire.payload["body_encoding"] = json!("utf8");
        wire.payload["body"] = json!("é".repeat(MAX_BODY_BYTES / 2));
        wire.validate().unwrap();
        wire.payload["body"] = json!("é".repeat(MAX_BODY_BYTES / 2 + 1));
        assert!(wire.validate().is_err());
        wire.payload["body_encoding"] = json!("base64");
        wire.payload["body"] = json!(STANDARD.encode(vec![0; MAX_BODY_BYTES]));
        wire.validate().unwrap();
        wire.payload["body"] = json!(STANDARD.encode(vec![0; MAX_BODY_BYTES + 1]));
        assert!(wire.validate().is_err());
        wire.payload["body"] = json!("AP8");
        assert!(wire.validate().is_err());
    }
    #[test]
    fn malformed_audience_actor_payload_and_unsupported_delivery_modes_refuse_before_admission() {
        for (field, value) in [
            ("to", json!([])),
            ("to", json!(["p", "p"])),
            ("to", json!("everyone")),
            ("to", json!(vec!["p"; 257])),
            ("from", json!("another-sender")),
            ("label", json!("")),
            ("body_encoding", json!("unknown")),
            ("attachments", json!([{"id":"private"}])),
            ("in_reply_to_msg", json!("reply")),
            ("reliability", json!({"mode":"reliable_unordered"})),
            ("content_type", json!("not-a-mime")),
        ] {
            let mut bad = wire();
            bad.payload[field] = value;
            assert!(bad.validate().is_err(), "{field}");
        }
    }
}
