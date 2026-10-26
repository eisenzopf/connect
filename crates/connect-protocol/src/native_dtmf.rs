//! Bounded DTMF intent. Payload fields never supply source identity or permission.
use crate::{Error, Result};
use serde_json::Value;

/// Trusted ingress facts, separate from the original request payload.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceObservation {
    pub envelope_id: String,
    pub received_at: chrono::DateTime<chrono::Utc>,
    pub stream_revision: u64,
    pub credential_revision: u64,
    pub signing_key_id: String,
    pub signing_algorithm: String,
}
impl std::fmt::Debug for SourceObservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DtmfSourceObservation(redacted)")
    }
}
impl SourceObservation {
    pub fn validate(&self) -> Result<()> {
        crate::transitions::id(&self.envelope_id)?;
        crate::transitions::id(&self.signing_key_id)?;
        if self.stream_revision == 0
            || self.stream_revision > 9_007_199_254_740_991
            || self.credential_revision == 0
            || self.credential_revision > 9_007_199_254_740_991
            || !matches!(self.signing_algorithm.as_str(), "EdDSA" | "ES256")
        {
            return Err(Error::new(
                "invalid-request",
                "Invalid DTMF source observation",
            ));
        }
        Ok(())
    }
}

/// Local transport facts. None of these proves remote playback.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionDisposition {
    NotStarted,
    LocalSubmitted,
    OutcomeUnknown,
}
impl SubmissionDisposition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotStarted => "not_started",
            Self::LocalSubmitted => "local_submitted",
            Self::OutcomeUnknown => "outcome_unknown",
        }
    }
}

/// Match the owned engine's complete sequence bounds before journaling anything.
/// Preserve exact spelling, method and extension values in the original payload;
/// validation neither normalizes digits nor fabricates a received observation.
pub fn validate_payload(payload: &Value) -> Result<()> {
    let invalid = || Error::new("invalid-request", "Invalid native DTMF intent");
    let object = payload.as_object().ok_or_else(invalid)?;
    let digits = object
        .get("digits")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    let duration = object
        .get("duration_ms")
        .and_then(Value::as_u64)
        .ok_or_else(invalid)?;
    let bounds = match object.get("method").and_then(Value::as_str) {
        Some("rfc4733") => 40..=6000,
        Some("info") => 100..=5000,
        _ => return Err(invalid()),
    };
    if digits.is_empty()
        || digits.len() > 32
        || !digits
            .bytes()
            .all(|b| b.is_ascii_digit() || b"*#ABCDabcd".contains(&b))
        || !bounds.contains(&duration)
        || duration * digits.len() as u64 + 70 * (digits.len() as u64 - 1) > 30_000
        || serde_json::to_vec(payload).map_err(|_| invalid())?.len() > 32 * 1024
    {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_streams::{Command, Intent};
    use serde_json::json;

    fn intent() -> Intent {
        Intent {
            operation_id: "tone-original".into(),
            conversation_id: "conversation".into(),
            session_id: "session".into(),
            connection_id: "source".into(),
            expected_revision: None,
            by_participant: None,
            command: Command::Dtmf,
            payload: json!({"digits":"12aB*#", "duration_ms":100, "method":"info", "future":{"exact":true}}),
        }
    }
    #[test]
    fn dtmf_original_sequence_method_optional_revision_and_extensions_survive_round_trip() {
        let mut wire = intent();
        wire.validate().unwrap();
        assert_eq!(wire.command.required_scope(), "dtmf.send");
        assert_eq!(wire.command.name(), "dtmf.send");
        let original = serde_json::to_value(&wire).unwrap();
        assert_eq!(
            serde_json::from_value::<Intent>(original.clone()).unwrap(),
            wire
        );
        wire.expected_revision = Some(1);
        wire.validate().unwrap();
        assert_ne!(serde_json::to_value(wire).unwrap(), original);
    }
    #[test]
    fn dtmf_rejects_invalid_suffixes_methods_numeric_coercion_and_oversize_before_effects() {
        for payload in [
            json!({"digits":"", "duration_ms":100, "method":"info"}),
            json!({"digits":"123x", "duration_ms":100, "method":"info"}),
            json!({"digits":"１２", "duration_ms":100, "method":"info"}),
            json!({"digits":"12\u{0}3", "duration_ms":100, "method":"info"}),
            json!({"digits":"1".repeat(33), "duration_ms":100, "method":"info"}),
            json!({"digits":"1", "duration_ms":"100", "method":"info"}),
            json!({"digits":"1", "duration_ms":100.5, "method":"info"}),
            json!({"digits":"1", "duration_ms":u64::MAX, "method":"info"}),
            json!({"digits":"1", "duration_ms":100, "method":"INFO"}),
            json!({"digits":"1", "duration_ms":100, "method":"inband"}),
            json!({"digits":"1", "duration_ms":100}),
            json!({"digits":"1", "duration_ms":100, "method":"info", "future":"x".repeat(32768)}),
        ] {
            let mut wire = intent();
            wire.payload = payload;
            assert!(wire.validate().is_err());
        }
    }
    #[test]
    fn dtmf_whole_schedule_and_method_specific_duration_limits_are_checked() {
        for (method, low, high) in [("rfc4733", 40, 6000), ("info", 100, 5000)] {
            for duration in [low, high] {
                validate_payload(&json!({"digits":"A", "duration_ms":duration, "method":method}))
                    .unwrap();
            }
            for duration in [low - 1, high + 1] {
                assert!(validate_payload(
                    &json!({"digits":"A", "duration_ms":duration, "method":method})
                )
                .is_err());
            }
        }
        validate_payload(&json!({"digits":"12345", "duration_ms":5944, "method":"rfc4733"}))
            .unwrap();
        assert!(validate_payload(
            &json!({"digits":"12345", "duration_ms":5945, "method":"rfc4733"})
        )
        .is_err());
        validate_payload(&json!({"digits":"d".repeat(32), "duration_ms":100, "method":"info"}))
            .unwrap();
    }
}
