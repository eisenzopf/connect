//! Exact native stream selection intent. This carries no route or media grant.
use crate::{transitions::id, Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

/// A signed hint about a recorded routing-only operation. Read the journal
/// under current authority before applying any observation from that operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconciliationHint {
    pub operation_id: String,
    pub record_id: String,
    pub command_type: Command,
    pub expected_revision: u64,
    pub state: crate::transitions::OperationState,
    pub revision: u64,
    pub subscription_record_id: String,
    pub stream_revision: u64,
    pub journal_hint: bool,
    pub resources_changed: bool,
}
impl ReconciliationHint {
    pub fn validate(&self) -> Result<()> {
        for value in [
            &self.operation_id,
            &self.record_id,
            &self.subscription_record_id,
        ] {
            id(value)?;
        }
        if self.command_type != Command::Reconcile
            || self.state != crate::transitions::OperationState::Completed
            || !self.journal_hint
            || self.resources_changed
            || self.expected_revision == 0
            || self.expected_revision >= 9_007_199_254_740_991
            || self.revision > 9_007_199_254_740_991
            || !(self.expected_revision..=self.expected_revision + 1)
                .contains(&self.stream_revision)
        {
            return Err(invalid());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Command {
    #[serde(rename = "stream.subscribe")]
    Subscribe,
    #[serde(rename = "stream.unsubscribe")]
    Unsubscribe,
    #[serde(rename = "connection.end")]
    End,
    #[serde(rename = "message.send")]
    Data,
    #[serde(rename = "connection.update")]
    Update,
    #[serde(rename = "capability.advertise")]
    Advertise,
    #[serde(rename = "dtmf.send")]
    Dtmf,
    /// Trusted server materialization of committed selectors; not a wire command.
    #[serde(rename = "stream.reconcile")]
    Reconcile,
}
impl Command {
    pub fn required_scope(self) -> &'static str {
        match self {
            Self::Data => "send",
            Self::Reconcile => "stream.subscribe",
            _ => self.name(),
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Subscribe => "stream.subscribe",
            Self::Unsubscribe => "stream.unsubscribe",
            Self::End => "connection.end",
            Self::Data => "message.send",
            Self::Update => "connection.update",
            Self::Advertise => "capability.advertise",
            Self::Dtmf => "dtmf.send",
            Self::Reconcile => "stream.reconcile",
        }
    }
}

/// Store this original, bounded payload as part of logical operation identity.
/// Optional extension fields remain included even when the current engine does
/// not interpret them. Selected sources are resolved under current authority.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Intent {
    pub operation_id: String,
    pub conversation_id: String,
    pub session_id: String,
    pub connection_id: String,
    pub expected_revision: Option<u64>,
    pub by_participant: Option<String>,
    pub command: Command,
    pub payload: Value,
}
impl std::fmt::Debug for Intent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeStreamIntent(redacted)")
    }
}
impl Intent {
    /// Exact local quality update; the original payload remains ledger identity.
    pub fn quality_interval_ms(&self) -> Option<u64> {
        (self.command == Command::Update && self.payload["action"] == "quality")
            .then(|| self.payload["quality_interval_ms"].as_u64())
            .flatten()
    }
    pub fn validate(&self) -> Result<()> {
        for value in [
            &self.operation_id,
            &self.conversation_id,
            &self.session_id,
            &self.connection_id,
        ] {
            id(value)?;
        }
        if let Some(by) = &self.by_participant {
            id(by)?;
        }
        let payload = self.payload.as_object().ok_or_else(invalid)?;
        if self
            .expected_revision
            .is_some_and(|r| r > 9_007_199_254_740_991)
            || (!matches!(self.command, Command::End | Command::Data | Command::Dtmf)
                && self.expected_revision.is_none())
            || serde_json::to_vec(payload).map_err(|_| invalid())?.len()
                > if self.command == Command::Data {
                    crate::MAX_PAGE_BYTES
                } else {
                    32 * 1024
                }
        {
            return Err(invalid());
        }
        let actor_field = if self.command == Command::Data {
            "from"
        } else {
            "by_participant"
        };
        if let Some(by) = payload.get(actor_field) {
            if by.as_str().is_none() || by.as_str() != self.by_participant.as_deref() {
                return Err(invalid());
            }
        } else if self.by_participant.is_some() {
            return Err(invalid());
        }
        let mut seen = BTreeSet::new();
        match self.command {
            Command::Dtmf => crate::native_dtmf::validate_payload(&self.payload)?,
            Command::Advertise => crate::native_capabilities::validate_payload(&self.payload)?,
            Command::Update if payload.get("action").and_then(Value::as_str) == Some("quality") => {
                if self
                    .quality_interval_ms()
                    .is_none_or(|ms| !(100..=60_000).contains(&ms))
                    || ["streams", "codec_preferences", "details"]
                        .iter()
                        .any(|key| payload.contains_key(*key))
                {
                    return Err(invalid());
                }
            }
            Command::Update if crate::native_gates::is_gate(&self.payload) => {
                crate::native_gates::validate(&self.payload)?;
            }
            Command::Update => {
                if payload.contains_key("quality_interval_ms")
                    || payload.get("action").and_then(Value::as_str) != Some("codec-renegotiate")
                {
                    return Err(invalid());
                }
                let preferences = payload
                    .get("codec_preferences")
                    .and_then(Value::as_array)
                    .ok_or_else(invalid)?;
                if preferences.is_empty() || preferences.len() > 32 {
                    return Err(invalid());
                }
                for value in preferences {
                    let name = value.as_str().ok_or_else(invalid)?;
                    if name.is_empty()
                        || name.len() > 64
                        || !name
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"-._/".contains(&b))
                        || !seen.insert(("codec", name))
                    {
                        return Err(invalid());
                    }
                }
                if let Some(streams) = payload.get("streams") {
                    let streams = streams.as_array().ok_or_else(invalid)?;
                    if streams.is_empty() || streams.len() > 32 {
                        return Err(invalid());
                    }
                    for value in streams {
                        let stream = value.as_str().ok_or_else(invalid)?;
                        id(stream)?;
                        if !seen.insert(("stream", stream)) {
                            return Err(invalid());
                        }
                    }
                }
            }
            Command::Reconcile => {
                id(payload
                    .get("subscription_record_id")
                    .and_then(Value::as_str)
                    .ok_or_else(invalid)?)?;
                if payload
                    .get("plan")
                    .and_then(Value::as_object)
                    .is_none_or(|p| p.is_empty())
                {
                    return Err(invalid());
                }
            }
            Command::Data => {
                crate::native_data::message(&self.payload)?;
            }
            Command::Subscribe => {
                let selectors = payload
                    .get("subscriptions")
                    .and_then(Value::as_array)
                    .ok_or_else(invalid)?;
                if selectors.is_empty() || selectors.len() > 128 {
                    return Err(invalid());
                }
                for selector in selectors {
                    let selector = selector.as_object().ok_or_else(invalid)?;
                    match (
                        selector.get("strm_id"),
                        selector.get("from_participant"),
                        selector.get("kinds"),
                    ) {
                        (Some(stream), None, None) => {
                            let stream = stream.as_str().ok_or_else(invalid)?;
                            id(stream)?;
                            if !seen.insert(("stream", stream)) {
                                return Err(invalid());
                            }
                        }
                        (None, Some(participant), kinds) => {
                            let participant = participant.as_str().ok_or_else(invalid)?;
                            id(participant)?;
                            if !seen.insert(("participant", participant)) {
                                return Err(invalid());
                            }
                            if let Some(kinds) = kinds {
                                let kinds = kinds.as_array().ok_or_else(invalid)?;
                                if kinds.is_empty() || kinds.len() > 4 {
                                    return Err(invalid());
                                }
                                let mut unique = BTreeSet::new();
                                for kind in kinds {
                                    let kind = kind.as_str().ok_or_else(invalid)?;
                                    if !matches!(kind, "audio" | "video" | "screen" | "data")
                                        || !unique.insert(kind)
                                    {
                                        return Err(invalid());
                                    }
                                }
                            }
                        }
                        _ => return Err(invalid()),
                    }
                }
            }
            Command::End => {
                if payload
                    .get("reason_code")
                    .and_then(Value::as_u64)
                    .is_none_or(|n| n > u16::MAX as u64)
                    || payload
                        .get("reason")
                        .and_then(Value::as_str)
                        .is_none_or(|reason| reason.len() > 1024)
                {
                    return Err(invalid());
                }
            }
            Command::Unsubscribe => {
                let streams = payload
                    .get("strm_ids")
                    .and_then(Value::as_array)
                    .ok_or_else(invalid)?;
                if streams.is_empty() || streams.len() > 128 {
                    return Err(invalid());
                }
                for stream in streams {
                    let stream = stream.as_str().ok_or_else(invalid)?;
                    id(stream)?;
                    if !seen.insert(("stream", stream)) {
                        return Err(invalid());
                    }
                }
            }
        }
        Ok(())
    }
}
/// Historical native codec commit metadata. It grants no current media rights.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodecBinding {
    pub stream_id: String,
    pub medium: String,
    pub direction: String,
    pub codec: Value,
    pub stream_local_id: u16,
    pub stream_generation: u64,
    pub connection_generation: u64,
}
/// Captured native selection result. Optional bindings distinguish unavailable
/// concurrent metadata from a known empty set; neither observation grants I/O.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionObservation {
    pub connection_id: String,
    pub command_type: Command,
    pub connection_generation: u64,
    pub expected_revision: u64,
    pub stream_revision: u64,
    pub resources_changed: bool,
    #[serde(deserialize_with = "required_capture")]
    pub stream_bindings: Option<Vec<CodecBinding>>,
}
fn required_capture<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<Vec<CodecBinding>>, D::Error> {
    Option::deserialize(d)
}
impl SelectionObservation {
    pub fn validate(&self) -> Result<()> {
        id(&self.connection_id)?;
        let safe = |n| n > 0 && n <= 9_007_199_254_740_991;
        if !matches!(
            self.command_type,
            Command::Subscribe | Command::Unsubscribe | Command::Reconcile
        ) || !safe(self.connection_generation)
            || !safe(self.expected_revision)
            || !safe(self.stream_revision)
            || self.stream_revision < self.expected_revision
            || self.stream_revision > self.expected_revision.saturating_add(1)
            || (self.resources_changed
                && self.expected_revision.checked_add(1) != Some(self.stream_revision))
        {
            return Err(invalid());
        }
        if let Some(bindings) = &self.stream_bindings {
            validate_stream_bindings(bindings)?;
            if bindings
                .iter()
                .any(|b| b.connection_generation != self.connection_generation)
            {
                return Err(invalid());
            }
        }
        Ok(())
    }
}
/// A complete captured set may be empty after retiring receive resources.
pub fn validate_stream_bindings(bindings: &[CodecBinding]) -> Result<()> {
    if bindings.is_empty() {
        return Ok(());
    }
    validate_codec_bindings(bindings)
}

pub fn validate_codec_bindings(bindings: &[CodecBinding]) -> Result<()> {
    if bindings.is_empty()
        || bindings.len() > 32
        || serde_json::to_vec(bindings).map_err(|_| invalid())?.len() > 32 * 1024
    {
        return Err(invalid());
    }
    let mut ids = BTreeSet::new();
    let mut handles = BTreeSet::new();
    for b in bindings {
        id(&b.stream_id)?;
        let name = b
            .codec
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(invalid)?;
        if !ids.insert(&b.stream_id)
            || !handles.insert(b.stream_local_id)
            || b.stream_local_id == 0
            || !matches!(b.medium.as_str(), "audio" | "video" | "screen" | "data")
            || !matches!(b.direction.as_str(), "sendonly" | "recvonly" | "sendrecv")
            || name.is_empty()
            || name.len() > 64
            || !name
                .bytes()
                .all(|v| v.is_ascii_alphanumeric() || b"-._/".contains(&v))
            || b.codec.get("params").and_then(Value::as_object).is_none()
            || b.stream_generation == 0
            || b.stream_generation > 9_007_199_254_740_991
            || b.connection_generation == 0
            || b.connection_generation > 9_007_199_254_740_991
        {
            return Err(invalid());
        }
    }
    Ok(())
}
fn invalid() -> Error {
    Error::new("invalid-request", "Invalid native stream selection intent")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn codec_update_preserves_exact_preferences_targets_extensions_and_revision() {
        let mut wire = intent(
            json!({"action":"codec-renegotiate","streams":["voice"],"codec_preferences":["pcma","pcmu"],"future":{"keep":true}}),
        );
        wire.command = Command::Update;
        wire.validate().unwrap();
        assert_eq!(wire.command.name(), "connection.update");
        assert_eq!(wire.command.required_scope(), "connection.update");
        assert_eq!(
            serde_json::from_value::<Intent>(serde_json::to_value(&wire).unwrap()).unwrap(),
            wire
        );
        wire.payload["codec_preferences"] = json!(["pcma", "pcma"]);
        assert!(wire.validate().is_err());
        wire.payload["codec_preferences"] = json!(["pcma"]);
        wire.expected_revision = None;
        assert!(wire.validate().is_err());
        wire.expected_revision = Some(1);
        wire.payload["action"] = json!("hold");
        assert!(wire.validate().is_err());
    }
    #[test]
    fn quality_updates_keep_exact_wire_identity_and_reject_mixed_or_unbounded_controls() {
        let mut wire =
            intent(json!({"action":"quality","quality_interval_ms":800,"future":{"exact":true}}));
        wire.command = Command::Update;
        wire.validate().unwrap();
        assert_eq!(wire.quality_interval_ms(), Some(800));
        assert_eq!(wire.command.name(), "connection.update");
        assert_eq!(
            serde_json::from_value::<Intent>(serde_json::to_value(&wire).unwrap()).unwrap(),
            wire
        );
        for invalid in [
            json!(0),
            json!(99),
            json!(60001),
            json!(800.5),
            json!("800"),
            json!(null),
        ] {
            let mut bad = wire.clone();
            bad.payload["quality_interval_ms"] = invalid;
            assert!(bad.validate().is_err());
        }
        for field in ["streams", "codec_preferences", "details"] {
            let mut bad = wire.clone();
            bad.payload[field] = json!(null);
            assert!(bad.validate().is_err());
        }
        wire.expected_revision = None;
        assert!(wire.validate().is_err());
        wire.expected_revision = Some(1);
        wire.payload = json!({"action":"codec-renegotiate","codec_preferences":["pcmu"],"quality_interval_ms":800});
        assert!(wire.validate().is_err());
    }
    #[test]
    fn captured_codec_bindings_are_unique_bounded_observations() {
        let b = CodecBinding {
            stream_id: "voice".into(),
            medium: "audio".into(),
            direction: "sendrecv".into(),
            codec: json!({"name":"pcma","params":{"sample_rate":8000,"channels":1,"payload_type":8}}),
            stream_local_id: 2,
            stream_generation: 2,
            connection_generation: 1,
        };
        validate_codec_bindings(std::slice::from_ref(&b)).unwrap();
        let mut other = b.clone();
        other.stream_id = "another".into();
        assert!(validate_codec_bindings(&[b.clone(), other.clone()]).is_err());
        other.stream_local_id = 3;
        validate_codec_bindings(&[b.clone(), other]).unwrap();
        let mut data = b.clone();
        data.stream_id = "chat".into();
        data.medium = "data".into();
        data.stream_local_id = 4;
        data.codec = json!({"name":"json","params":{"reliability":"reliable_ordered"}});
        let mut more = data.clone();
        more.stream_id = "binary".into();
        more.stream_local_id = 5;
        validate_codec_bindings(&[b.clone(), data.clone(), more.clone()]).unwrap();
        assert!(validate_codec_bindings(&[data.clone(), data.clone()]).is_err());
        data.stream_local_id = 0;
        assert!(validate_codec_bindings(&[b.clone(), data]).is_err());
        let mut zero = b.clone();
        zero.stream_local_id = 0;
        assert!(validate_codec_bindings(&[zero]).is_err());
        let mut bad = b.clone();
        bad.stream_generation = 0;
        assert!(validate_codec_bindings(&[bad]).is_err());
        let mut bad = b;
        bad.codec["params"] = json!("not parameters");
        assert!(validate_codec_bindings(&[bad]).is_err());
    }
    #[test]
    fn selection_capture_distinguishes_known_empty_unavailable_and_missing() {
        let raw = json!({"connection_id":"receiver","command_type":"stream.unsubscribe","connection_generation":2,"expected_revision":7,"stream_revision":8,"resources_changed":false,"stream_bindings":[]});
        let known: SelectionObservation = serde_json::from_value(raw.clone()).unwrap();
        known.validate().unwrap();
        assert_eq!(known.stream_bindings.unwrap().len(), 0);
        let mut unavailable = raw.clone();
        unavailable["stream_bindings"] = Value::Null;
        let v: SelectionObservation = serde_json::from_value(unavailable).unwrap();
        v.validate().unwrap();
        assert!(v.stream_bindings.is_none());
        let mut missing = raw;
        missing.as_object_mut().unwrap().remove("stream_bindings");
        assert!(serde_json::from_value::<SelectionObservation>(missing).is_err());
    }
    #[test]
    fn selection_capture_rejects_wrong_counter_kind_and_foreign_or_aliased_ports() {
        let b = json!({"stream_id":"camera","medium":"video","direction":"sendonly","codec":{"name":"vp8","params":{}},"stream_local_id":17,"stream_generation":1,"connection_generation":2});
        let base = json!({"connection_id":"receiver","command_type":"stream.subscribe","connection_generation":2,"expected_revision":7,"stream_revision":8,"resources_changed":true,"stream_bindings":[b.clone()]});
        serde_json::from_value::<SelectionObservation>(base.clone())
            .unwrap()
            .validate()
            .unwrap();
        for (field, value) in [
            ("command_type", json!("connection.update")),
            ("expected_revision", json!(0)),
            ("stream_revision", json!(7)),
            ("connection_generation", json!(3)),
        ] {
            let mut v = base.clone();
            v[field] = value;
            assert!(
                serde_json::from_value::<SelectionObservation>(v)
                    .unwrap()
                    .validate()
                    .is_err(),
                "{field}"
            );
        }
        let mut v = base.clone();
        v["stream_bindings"] = json!([b.clone(), b]);
        assert!(serde_json::from_value::<SelectionObservation>(v)
            .unwrap()
            .validate()
            .is_err());
        let mut v = base;
        v["stream_bindings"][0]["stream_local_id"] = json!(0);
        assert!(serde_json::from_value::<SelectionObservation>(v)
            .unwrap()
            .validate()
            .is_err());
    }
    fn intent(payload: Value) -> Intent {
        Intent {
            operation_id: "selection-one".into(),
            conversation_id: "conversation".into(),
            session_id: "session".into(),
            connection_id: "receiver".into(),
            expected_revision: Some(1),
            by_participant: None,
            command: Command::Subscribe,
            payload,
        }
    }
    #[test]
    fn server_reconciliation_retains_exact_selector_record_plan_and_required_permission() {
        let mut value = intent(
            json!({"subscription_record_id":"opr_original","plan":{"streams":[{"id":"camera","kind":"video"}],"future":{"frozen":true}}}),
        );
        value.command = Command::Reconcile;
        value.validate().unwrap();
        assert_eq!(value.command.name(), "stream.reconcile");
        assert_eq!(value.command.required_scope(), "stream.subscribe");
        let encoded = serde_json::to_value(&value).unwrap();
        let decoded: Intent = serde_json::from_value(encoded).unwrap();
        assert_eq!(value, decoded);
        value.payload["subscription_record_id"] = json!("");
        assert!(value.validate().is_err());
        value.payload["subscription_record_id"] = json!("opr_original");
        value.payload["plan"] = json!({});
        assert!(value.validate().is_err());
    }
    #[test]
    fn original_selector_order_extensions_and_scope_survive_storage_round_trip() {
        let original = intent(
            json!({"subscriptions":[{"from_participant":"source","kinds":["audio"],"future-selector":{"weight":3}},{"strm_id":"camera"}],"future-plan":{"mode":"pinned"}}),
        );
        original.validate().unwrap();
        let saved: Intent =
            serde_json::from_slice(&serde_json::to_vec(&original).unwrap()).unwrap();
        saved.validate().unwrap();
        assert_eq!(saved, original);
        let mut changed = saved.clone();
        changed.payload["future-plan"]["mode"] = json!("other");
        changed.validate().unwrap();
        assert_ne!(saved, changed);
        changed = saved.clone();
        changed.connection_id = "another-receiver".into();
        assert_ne!(saved, changed);
    }
    #[test]
    fn contradictory_duplicate_or_unbounded_selectors_cannot_enter_the_ledger() {
        for payload in [
            json!({"subscriptions":[]}),
            json!({"subscriptions":[{"strm_id":"s","from_participant":"p"}]}),
            json!({"subscriptions":[{"strm_id":"s","kinds":["audio"]}]}),
            json!({"subscriptions":[{"from_participant":"p","kinds":[]}]}),
            json!({"subscriptions":[{"from_participant":"p","kinds":["audio","audio"]}]}),
            json!({"subscriptions":[{"from_participant":"p","kinds":["unknown"]}]}),
            json!({"subscriptions":[{"strm_id":"s"},{"strm_id":"s"}]}),
            json!({"subscriptions":[{"from_participant":"p"},{"from_participant":"p","kinds":["audio"]}]}),
            json!({"subscriptions":[{"strm_id":null}]}),
            json!({"subscriptions":vec![json!({"strm_id":"s"});129]}),
        ] {
            assert!(intent(payload).validate().is_err());
        }
    }
    #[test]
    fn actor_assertions_revision_and_unsubscribe_have_one_exact_shape() {
        let mut input = intent(json!({"by_participant":"actor","subscriptions":[{"strm_id":"s"}]}));
        assert!(input.validate().is_err());
        input.by_participant = Some("actor".into());
        input.validate().unwrap();
        input.by_participant = Some("other".into());
        assert!(input.validate().is_err());
        input = intent(json!({"subscriptions":[{"strm_id":"s"}]}));
        input.expected_revision = Some(9_007_199_254_740_992);
        assert!(input.validate().is_err());
        input.command = Command::Unsubscribe;
        input.expected_revision = Some(2);
        input.payload = json!({"strm_ids":["s"],"future":true});
        input.validate().unwrap();
        input.payload = json!({"strm_ids":["s","s"]});
        assert!(input.validate().is_err());
    }
    #[test]
    fn connection_end_preserves_optional_revision_and_bounded_original_reason() {
        let mut end = intent(json!({"reason_code":1000,"reason":"finished","future":{"x":1}}));
        end.command = Command::End;
        end.expected_revision = None;
        end.validate().unwrap();
        let original = serde_json::to_value(&end).unwrap();
        end.expected_revision = Some(1);
        assert_ne!(original, serde_json::to_value(&end).unwrap());
        for payload in [
            json!({}),
            json!({"reason_code":65536,"reason":"no"}),
            json!({"reason_code":-1,"reason":"no"}),
            json!({"reason_code":1000,"reason":null}),
            json!({"reason_code":1000,"reason":"x".repeat(1025)}),
        ] {
            end.payload = payload;
            assert!(end.validate().is_err());
        }
    }
}

#[cfg(test)]
mod routing_hint_tests {
    use super::*;
    fn hint() -> ReconciliationHint {
        ReconciliationHint {
            operation_id: "reconcile_one".into(),
            record_id: "opr_one".into(),
            command_type: Command::Reconcile,
            expected_revision: 2,
            state: crate::transitions::OperationState::Completed,
            revision: 2,
            subscription_record_id: "opr_selector".into(),
            stream_revision: 3,
            journal_hint: true,
            resources_changed: false,
        }
    }
    #[test]
    fn routing_hint_roundtrips_original_identity_without_result_or_binding_grant() {
        let h = hint();
        h.validate().unwrap();
        let value = serde_json::to_value(&h).unwrap();
        assert!(value.get("result").is_none());
        assert!(value.get("stream_bindings").is_none());
        assert_eq!(
            serde_json::from_value::<ReconciliationHint>(value).unwrap(),
            h
        );
    }
    #[test]
    fn routing_hint_rejects_private_plan_and_capture_fields() {
        for field in ["result", "stream_bindings", "plan", "intent"] {
            let mut value = serde_json::to_value(hint()).unwrap();
            value[field] = serde_json::json!({});
            assert!(serde_json::from_value::<ReconciliationHint>(value).is_err());
        }
    }
    #[test]
    fn routing_hint_rejects_noncommitted_nonrouting_foreign_or_unsafe_identity() {
        for field in 0..9 {
            let mut h = hint();
            match field {
                0 => h.command_type = Command::Subscribe,
                1 => h.state = crate::transitions::OperationState::Unknown,
                2 => h.expected_revision = 0,
                3 => h.expected_revision = 9_007_199_254_740_991,
                4 => h.stream_revision = 4,
                5 => h.revision = 9_007_199_254_740_992,
                6 => h.resources_changed = true,
                7 => h.journal_hint = false,
                _ => h.subscription_record_id = "foreign/record".into(),
            };
            assert!(h.validate().is_err(), "case {field}");
        }
    }
}
