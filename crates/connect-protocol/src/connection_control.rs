//! Durable native Connection intent; transport resources remain server-owned.
use crate::{transitions::id, Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfferIntent {
    pub operation_id: String,
    pub conversation_id: String,
    pub session_id: String,
    pub connection_id: String,
    pub by_participant: Option<String>,
    pub expected_revision: Option<u64>,
    pub offer: Value,
}
impl OfferIntent {
    pub fn normalize(&mut self) -> Result<()> {
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
        if self
            .expected_revision
            .is_some_and(|v| v > 9_007_199_254_740_991)
        {
            return Err(invalid());
        }
        if !self.offer.is_object()
            || serde_json::to_vec(&self.offer)
                .map_err(|_| invalid())?
                .len()
                > 32 * 1024
        {
            return Err(invalid());
        }
        if !matches!(
            self.offer.get("substrate").and_then(Value::as_str),
            Some("quic" | "webtransport")
        ) {
            return Err(Error::new(
                "unsupported-media-profile",
                "Native Connection substrate is unavailable",
            ));
        }
        if !self.offer.get("capabilities").is_some_and(Value::is_object) {
            return Err(invalid());
        }
        if let Some(by) = self.offer.get("by_participant").filter(|v| !v.is_null()) {
            if by.as_str() != self.by_participant.as_deref() {
                return Err(invalid());
            }
        }
        let streams = self
            .offer
            .get("streams_offered")
            .and_then(Value::as_array)
            .ok_or_else(invalid)?;
        if streams.is_empty() || streams.len() > 32 {
            return Err(invalid());
        }
        let mut ids = std::collections::BTreeSet::new();
        for stream in streams {
            let stream = stream.as_object().ok_or_else(invalid)?;
            let stream_id = stream
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(invalid)?;
            id(stream_id)?;
            if !ids.insert(stream_id) {
                return Err(invalid());
            }
            if !["audio", "video", "screen", "data"].contains(
                &stream
                    .get("kind")
                    .and_then(Value::as_str)
                    .ok_or_else(invalid)?,
            ) {
                return Err(invalid());
            }
            if !["sendrecv", "sendonly", "recvonly", "inactive"].contains(
                &stream
                    .get("direction")
                    .and_then(Value::as_str)
                    .ok_or_else(invalid)?,
            ) {
                return Err(invalid());
            }
            let codecs = stream
                .get("codec_preferences")
                .and_then(Value::as_array)
                .ok_or_else(invalid)?;
            if codecs.is_empty()
                || codecs.len() > 16
                || codecs.iter().any(|v| {
                    v.as_str().is_none_or(|s| {
                        s.is_empty() || s.len() > 64 || s.chars().any(char::is_control)
                    })
                })
            {
                return Err(invalid());
            }
        }
        Ok(())
    }
}
/// The endpoint is resolved by the trusted native backend, then asserted again
/// under the same current Device/key fence. It is not a wire-selected grant.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrepareRequest {
    pub preparation_id: String,
    pub intent: OfferIntent,
    pub endpoint_id: String,
}
impl PrepareRequest {
    /// A private local phase has its own stable ledger ID. Completing it must
    /// never report the public connection.offer/answer exchange as completed.
    pub fn new(intent: OfferIntent, endpoint_id: String) -> Self {
        let preparation_id = format!("nprep_{:x}", Sha256::digest(intent.operation_id.as_bytes()));
        Self {
            preparation_id,
            intent,
            endpoint_id,
        }
    }
    pub fn normalize(&mut self) -> Result<()> {
        self.intent.normalize()?;
        id(&self.preparation_id)?;
        if self.preparation_id
            != Self::new(self.intent.clone(), self.endpoint_id.clone()).preparation_id
        {
            return Err(invalid());
        }
        id(&self.endpoint_id)
    }
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedConnection {
    pub connection_id: String,
    pub conversation_id: String,
    pub session_id: String,
    pub participant_id: String,
    pub membership_epoch: u64,
    pub endpoint_id: String,
    pub generation: u64,
    pub revision: u64,
    pub state: String,
}
fn invalid() -> Error {
    Error::new("invalid-request", "Invalid native Connection intent")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn intent() -> OfferIntent {
        OfferIntent {
            operation_id: "offer".into(),
            conversation_id: "conversation".into(),
            session_id: "session".into(),
            connection_id: "connection".into(),
            by_participant: None,
            expected_revision: Some(0),
            offer: json!({"substrate":"quic","capabilities":{},"streams_offered":[{"id":"audio","kind":"audio","direction":"sendrecv","codec_preferences":["opus","g.711-mu"]}]}),
        }
    }
    #[test]
    fn private_preparation_id_preserves_wire_operation_and_cannot_complete_its_exchange() {
        let wire = intent();
        let mut request = PrepareRequest::new(wire.clone(), "endpoint".into());
        request.normalize().unwrap();
        assert_ne!(request.preparation_id, wire.operation_id);
        let operation = crate::Operation::ConnectionPrepare {
            request: request.clone(),
        };
        assert_eq!(
            operation.request_id(),
            Some(request.preparation_id.as_str())
        );
        assert_eq!(operation.required_scope(), "connection.offer");
        assert!(crate::application_api::response_kind(&operation).is_none());
        let mut changed = wire.clone();
        changed.connection_id = "different".into();
        assert_eq!(
            PrepareRequest::new(changed, "endpoint".into()).preparation_id,
            request.preparation_id
        );
        request.preparation_id = wire.operation_id;
        assert!(request.normalize().is_err());
    }
    #[test]
    fn native_offer_profile_limits_and_required_stream_fields_are_checked() {
        let valid = intent();
        for (field, bad) in [
            ("direction", json!("both")),
            ("id", json!("bad space")),
            ("kind", json!("camera")),
            ("codec_preferences", json!([])),
        ] {
            let mut value = valid.clone();
            value.offer["streams_offered"][0][field] = bad;
            assert!(value.normalize().is_err(), "{field}");
        }
        let mut duplicate = valid.clone();
        duplicate.offer["streams_offered"] = json!([
            valid.offer["streams_offered"][0],
            valid.offer["streams_offered"][0]
        ]);
        assert!(duplicate.normalize().is_err());
        let mut oversized = valid.clone();
        oversized.offer["extension"] = json!("x".repeat(32 * 1024));
        assert!(oversized.normalize().is_err());
        let mut unsupported = valid.clone();
        unsupported.offer["substrate"] = json!("websocket");
        assert_eq!(
            unsupported.normalize().unwrap_err().code,
            "unsupported-media-profile"
        );
        let mut revision = valid;
        revision.expected_revision = Some(9_007_199_254_740_992);
        assert!(revision.normalize().is_err());
    }
    #[test]
    fn normalized_offer_preserves_optional_extensions_and_codec_preference_order() {
        let mut wire = intent();
        wire.offer["extension"] = json!({"peer_observation":true});
        let before = wire.clone();
        wire.normalize().unwrap();
        assert!(wire == before);
        let request = PrepareRequest::new(wire, "endpoint".into());
        let mut encoded = serde_json::to_value(request).unwrap();
        encoded["ready"] = json!(true);
        assert!(serde_json::from_value::<PrepareRequest>(encoded).is_err());
    }
}
