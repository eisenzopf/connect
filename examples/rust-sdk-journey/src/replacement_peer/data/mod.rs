//! Fixture interpretation of already signature-verified public Data envelopes.
//! This grants no publication authority and performs no service mutation.
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};

pub(super) struct ExpectedData {
    conversation: String,
    session: String,
    connection: String,
    operation: String,
    message: String,
    stream: String,
    sender: String,
    recipient: String,
    body: String,
    received: bool,
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._~-".contains(&b))
}
impl ExpectedData {
    pub(super) fn new(
        scope: [&str; 3],
        original: [&str; 3],
        audience: [&str; 2],
        body: &str,
    ) -> Result<Self, &'static str> {
        if !scope
            .iter()
            .chain(original.iter())
            .chain(audience.iter())
            .all(|value| identifier(value))
            || audience[0] == audience[1]
            || body.len() != 87384
        {
            return Err("invalid original Data fixture identity or size");
        }
        let bytes = STANDARD
            .decode(body)
            .map_err(|_| "invalid original Data fixture bytes")?;
        if bytes.len() != 65536 || STANDARD.encode(bytes) != body {
            return Err("noncanonical original Data fixture bytes");
        }
        Ok(Self {
            conversation: scope[0].into(),
            session: scope[1].into(),
            connection: scope[2].into(),
            operation: original[0].into(),
            message: original[1].into(),
            stream: original[2].into(),
            sender: audience[0].into(),
            recipient: audience[1].into(),
            body: body.into(),
            received: false,
        })
    }
    pub(super) fn received(&self) -> bool {
        self.received
    }
    pub(super) fn observe(
        &mut self,
        frame: &Value,
        original_root_completed: bool,
    ) -> Result<(), &'static str> {
        if !original_root_completed {
            return Err("Data arrived before original completed root observation");
        }
        if self.received {
            return Err("duplicate original physical Data delivery");
        }
        let p = &frame["payload"];
        if frame["v"] != 2
            || frame["type"] != "message.send"
            || frame["cid"] != self.conversation
            || frame["sid"] != self.session
            || frame["connid"] != self.connection
            || frame["operation_id"] != self.operation
            || frame.get("in_reply_to").is_some()
            || frame.get("expected_revision").is_some()
            || p["msg_id"] != self.message
            || p["label"] != self.stream
            || p["from"] != self.sender
            || p["to"] != json!([self.recipient])
            || p["content_type"] != "application/octet-stream"
            || p["body_encoding"] != "base64"
            || p["body"] != self.body
            || p["reliability"] != json!({"mode":"reliable_ordered"})
            || p.get("in_reply_to_msg").is_some()
            || p.get("attachments")
                .is_some_and(|v| v.as_array().is_none_or(|v| !v.is_empty()))
        {
            return Err("physical Data changed its original scope, audience or bytes");
        }
        self.received = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (ExpectedData, Value) {
        let body = STANDARD.encode((0..65536).map(|i| (i * 13 % 256) as u8).collect::<Vec<_>>());
        let expected = ExpectedData::new(
            ["room", "session", "physical-target"],
            ["original-operation", "original-message", "retained-data"],
            ["retained-sender", "local-participant"],
            &body,
        )
        .unwrap();
        let frame = json!({"v":2,"type":"message.send","cid":"room","sid":"session","connid":"physical-target","operation_id":"original-operation", "payload":{"msg_id":"original-message","label":"retained-data","from":"retained-sender","to":["local-participant"],"content_type":"application/octet-stream","body_encoding":"base64","body":body,"reliability":{"mode":"reliable_ordered"}}});
        (expected, frame)
    }
    #[test]
    fn exact_original_full_payload_is_observed_once_after_known_completion() {
        let (mut expected, frame) = fixture();
        assert!(!expected.received());
        expected.observe(&frame, true).unwrap();
        assert!(expected.received());
        assert!(expected.observe(&frame, true).is_err());
    }
    #[test]
    fn precompletion_data_is_refused_without_claiming_received_custody() {
        let (mut expected, frame) = fixture();
        assert!(expected.observe(&frame, false).is_err());
        assert!(!expected.received());
    }
    #[test]
    fn every_original_scope_and_operation_field_is_required() {
        for key in ["v", "type", "cid", "sid", "connid", "operation_id"] {
            let (mut expected, mut frame) = fixture();
            frame[key] = json!("foreign");
            assert!(expected.observe(&frame, true).is_err(), "{key}");
            assert!(!expected.received());
        }
        for key in ["in_reply_to", "expected_revision"] {
            let (mut expected, mut frame) = fixture();
            frame[key] = Value::Null;
            assert!(expected.observe(&frame, true).is_err(), "{key}");
        }
    }
    #[test]
    fn foreign_stream_sender_recipient_codec_or_payload_is_refused() {
        for key in [
            "msg_id",
            "label",
            "from",
            "to",
            "content_type",
            "body_encoding",
            "body",
            "reliability",
        ] {
            let (mut expected, mut frame) = fixture();
            frame["payload"][key] = json!("foreign");
            assert!(expected.observe(&frame, true).is_err(), "{key}");
            assert!(!expected.received());
        }
        let (mut expected, mut frame) = fixture();
        frame["payload"]["to"] = json!(["local-participant", "unaddressed-third-party"]);
        assert!(expected.observe(&frame, true).is_err());
    }
    #[test]
    fn hidden_attachment_or_reply_content_is_refused() {
        for (key, value) in [
            ("attachments", json!([{}])),
            ("attachments", json!(null)),
            ("in_reply_to_msg", json!("other-message")),
        ] {
            let (mut expected, mut frame) = fixture();
            frame["payload"][key] = value;
            assert!(expected.observe(&frame, true).is_err(), "{key}");
        }
    }
    #[test]
    fn malformed_or_short_originals_cannot_define_a_smaller_probe() {
        let scope = ["room", "session", "physical-target"];
        let original = ["original-operation", "original-message", "data"];
        let audience = ["sender", "recipient"];
        for body in ["".into(), STANDARD.encode([0; 65535]), "!".repeat(87384)] {
            assert!(ExpectedData::new(scope, original, audience, &body).is_err());
        }
        let body = STANDARD.encode([0; 65536]);
        assert!(ExpectedData::new(scope, original, ["sender", "sender"], &body).is_err());
        assert!(ExpectedData::new(
            ["room", "session", "foreign scope"],
            original,
            audience,
            &body
        )
        .is_err());
    }
}

pub(super) mod io;
#[path = "outgoing.rs"]
mod outgoing;
