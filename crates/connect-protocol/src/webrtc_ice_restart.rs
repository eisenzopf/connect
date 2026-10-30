//! Bounded intent for restarting ICE on an existing canonical WebRTC peer.
//!
//! This is a request-shape boundary, not SDP negotiation, authorization or
//! readiness. Public dispatch is installed only with the durable runtime path.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
mod offer;
pub use offer::{
    validate_negotiated_offer_change, validate_offer_change, validate_restart_candidate,
};

pub const MAX_SDP_BYTES: usize = 512 * 1024;
pub const MAX_UPDATE_BYTES: usize = 544 * 1024;
/// Generation observations must round-trip through the browser's JSON numbers.
pub const MAX_ICE_GENERATION: u64 = (1_u64 << 53) - 1;

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IceRestartAction {
    #[serde(rename = "ice-restart")]
    IceRestart,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WebRtcSetupKind {
    #[serde(rename = "websocket+webrtc")]
    WebSocketWebRtc,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebRtcSdpSetup {
    pub kind: WebRtcSetupKind,
    pub sdp: String,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IceRestartDetails {
    /// A caller assertion, never proof of the current physical owner. ICE
    /// generations are independent of Connection/Stream ownership generations.
    #[cfg_attr(
        feature = "schema",
        schemars(range(min = 0, max = 9_007_199_254_740_990_u64))
    )]
    pub expected_ice_generation: u64,
    pub setup: WebRtcSdpSetup,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IceRestartUpdate {
    pub action: IceRestartAction,
    pub details: IceRestartDetails,
}

/// Private correlated ACK data. This proposes an ICE generation; it is never
/// a completed-generation receipt and never belongs in shared operation data.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IceRestartAnswer {
    #[cfg_attr(
        feature = "schema",
        schemars(range(min = 1, max = 9_007_199_254_740_991_u64))
    )]
    pub proposed_ice_generation: u64,
    pub setup: WebRtcSdpSetup,
}
impl std::fmt::Debug for IceRestartAnswer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("IceRestartAnswer(redacted)")
    }
}
impl IceRestartAnswer {
    pub fn new(expected: u64, sdp: String) -> Result<Self> {
        if expected >= MAX_ICE_GENERATION {
            return Err(invalid());
        }
        let value = Self {
            proposed_ice_generation: expected + 1,
            setup: WebRtcSdpSetup {
                kind: WebRtcSetupKind::WebSocketWebRtc,
                sdp,
            },
        };
        value.validate(expected)?;
        Ok(value)
    }
    pub fn validate(&self, expected: u64) -> Result<()> {
        validate_sdp(&self.setup.sdp)?;
        if expected >= MAX_ICE_GENERATION
            || self.proposed_ice_generation != expected + 1
            || serde_json::to_vec(self).map_err(|_| invalid())?.len() > MAX_UPDATE_BYTES
        {
            return Err(invalid());
        }
        Ok(())
    }
}

impl std::fmt::Debug for WebRtcSdpSetup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WebRtcSdpSetup(redacted)")
    }
}
impl std::fmt::Debug for IceRestartDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("IceRestartDetails(redacted)")
    }
}
impl std::fmt::Debug for IceRestartUpdate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("IceRestartUpdate(redacted)")
    }
}
impl IceRestartUpdate {
    /// Retain the original SDP bytes. The engine must separately parse it,
    /// check unchanged media/DTLS identity, and confirm the new ICE generation.
    pub fn validate(&self) -> Result<()> {
        validate_sdp(&self.details.setup.sdp)?;
        if self.details.expected_ice_generation >= MAX_ICE_GENERATION
            || serde_json::to_vec(self).map_err(|_| invalid())?.len() > MAX_UPDATE_BYTES
        {
            return Err(invalid());
        }
        Ok(())
    }
}

pub fn parse_update(value: &Value) -> Result<IceRestartUpdate> {
    let sdp = value["details"]["setup"]["sdp"]
        .as_str()
        .ok_or_else(invalid)?;
    validate_sdp(sdp)?;
    // Borrow the input while deserializing; reject an oversized SDP or an
    // unknown body field before copying an unbounded caller Value tree.
    let update = IceRestartUpdate::deserialize(value).map_err(|_| invalid())?;
    update.validate()?;
    Ok(update)
}
fn validate_sdp(sdp: &str) -> Result<()> {
    if !sdp.starts_with("v=0\r\n") || sdp.contains('\0') || sdp.len() > MAX_SDP_BYTES {
        return Err(invalid());
    }
    Ok(())
}
fn invalid() -> Error {
    Error::new(
        "invalid-request",
        "Invalid bounded WebRTC ICE restart intent",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request() -> Value {
        json!({"action":"ice-restart","details":{"expected_ice_generation":0,
            "setup":{"kind":"websocket+webrtc","sdp":"v=0\r\na=ice-ufrag:abcd\r\na=ice-pwd:PRIVATE-FIXTURE-PASSWORD\r\n"}}})
    }
    #[test]
    fn private_answer_proposes_only_original_successor_without_claiming_ready() {
        let sdp = "v=0\r\na=ice-pwd:PRIVATE-ANSWER\r\n".to_owned();
        let answer = IceRestartAnswer::new(7, sdp.clone()).unwrap();
        assert_eq!(answer.proposed_ice_generation, 8);
        assert_eq!(answer.setup.sdp, sdp);
        assert!(answer.validate(6).is_err());
        assert!(!format!("{answer:?}").contains("PRIVATE"));
        let json = serde_json::to_value(&answer).unwrap();
        assert!(json.get("ice_generation").is_none());
        assert!(json.get("ready").is_none());
        let mut changed = json;
        changed["ready"] = json!(true);
        assert!(serde_json::from_value::<IceRestartAnswer>(changed).is_err());
    }
    #[test]
    fn private_answer_rejects_overflow_oversize_and_unknown_substrate() {
        assert!(IceRestartAnswer::new(MAX_ICE_GENERATION, "v=0\r\n".into()).is_err());
        assert!(
            IceRestartAnswer::new(0, "v=0\r\n".to_owned() + &"x".repeat(MAX_SDP_BYTES)).is_err()
        );
        assert!(IceRestartAnswer::new(0, "v=0\r\n\0".into()).is_err());
        assert!(serde_json::from_value::<IceRestartAnswer>(json!({"proposed_ice_generation":1,"setup":{"kind":"native-prepared/1","sdp":"v=0\r\n"}})).is_err());
    }
    #[test]
    fn original_sdp_and_generation_round_trip_without_normalization_or_authority_fields() {
        let original = request();
        let parsed = parse_update(&original).unwrap();
        assert_eq!(serde_json::to_value(&parsed).unwrap(), original);
        assert_eq!(parsed.details.expected_ice_generation, 0);
        let again = parse_update(&original).unwrap();
        assert_eq!(parsed, again);
        let mut changed = original;
        changed["details"]["setup"]["sdp"] = json!("v=0\r\na=ice-ufrag:changed\r\n");
        assert_ne!(parsed, parse_update(&changed).unwrap());
    }
    #[test]
    fn mixed_actions_resource_assertions_and_unknown_setup_fields_refuse() {
        for (field, value) in [
            ("streams", json!(["stream"])),
            ("codec_preferences", json!(["opus"])),
            ("quality_interval_ms", json!(1000)),
            ("operation_id", json!("replacement-operation")),
            ("connection_generation", json!(8)),
            ("by_participant", json!("different-owner")),
            ("approved", json!(true)),
        ] {
            let mut value_with_extra = request();
            value_with_extra[field] = value;
            assert!(parse_update(&value_with_extra).is_err(), "{field}");
        }
        let mut extra = request();
        extra["details"]["endpoint_id"] = json!("different-endpoint");
        assert!(parse_update(&extra).is_err());
        extra = request();
        extra["details"]["setup"]["candidate"] = json!("unreviewed-route");
        assert!(parse_update(&extra).is_err());
        for action in ["hold", "codec-renegotiate", "restart", "ICE-restart"] {
            extra = request();
            extra["action"] = json!(action);
            assert!(parse_update(&extra).is_err());
        }
    }
    #[test]
    fn generation_is_an_exact_safe_incrementable_integer() {
        let mut value = request();
        for generation in [
            json!(-1),
            json!(1.5),
            json!(1.0),
            json!("1"),
            json!(null),
            json!(true),
            json!(MAX_ICE_GENERATION),
            json!(u64::MAX),
        ] {
            value["details"]["expected_ice_generation"] = generation;
            assert!(parse_update(&value).is_err());
        }
        value["details"]["expected_ice_generation"] = json!(MAX_ICE_GENERATION - 1);
        assert!(parse_update(&value).is_ok());
    }
    #[test]
    fn malformed_or_missing_nested_values_refuse_without_panicking() {
        for value in [
            json!(null),
            json!([]),
            json!(1),
            json!("ice-restart"),
            json!({}),
            json!({"action":"ice-restart","details":null}),
            json!({"action":"ice-restart","details":{"setup":{"sdp":null}}}),
            json!({"action":"ice-restart","details":{"setup":{"sdp":"v=0\r\n"}}}),
        ] {
            assert!(parse_update(&value).is_err());
        }
    }
    #[cfg(feature = "schema")]
    #[test]
    fn schema_generation_limit_matches_the_incrementable_runtime_counter() {
        let schema = serde_json::to_value(schemars::schema_for!(IceRestartDetails)).unwrap();
        let generation = &schema["properties"]["expected_ice_generation"];
        assert_eq!(generation["type"], json!("integer"));
        assert_eq!(generation["minimum"].as_f64(), Some(0.0));
        assert_eq!(
            generation["maximum"].as_f64(),
            Some((MAX_ICE_GENERATION - 1) as f64)
        );
        assert_eq!(schema["additionalProperties"], json!(false));
    }
    #[test]
    fn setup_requires_the_existing_webrtc_substrate_and_bounded_version_line() {
        for kind in ["quic", "webtransport", "webrtc", "websocket"] {
            let mut value = request();
            value["details"]["setup"]["kind"] = json!(kind);
            assert!(parse_update(&value).is_err());
        }
        for sdp in ["", "v=0\n", " v=0\r\n", "v=1\r\n", "v=0\r\n\0"] {
            let mut value = request();
            value["details"]["setup"]["sdp"] = json!(sdp);
            assert!(parse_update(&value).is_err());
        }
    }
    #[test]
    fn sdp_and_serialized_payload_have_separate_byte_budgets() {
        let mut update = parse_update(&request()).unwrap();
        update.details.setup.sdp = "v=0\r\n".to_owned() + &"x".repeat(MAX_SDP_BYTES - 7) + "é";
        assert_eq!(update.details.setup.sdp.len(), MAX_SDP_BYTES);
        update.validate().unwrap();
        update.details.setup.sdp.push('é');
        assert!(update.validate().is_err());
        update.details.setup.sdp = "v=0\r\n".to_owned() + &"\r\n".repeat(200_000);
        assert!(update.details.setup.sdp.len() < MAX_SDP_BYTES);
        assert!(
            update.validate().is_err(),
            "escaped JSON must fit the frame budget too"
        );
    }
    #[test]
    fn diagnostics_do_not_copy_ice_credentials_or_sdp() {
        let update = parse_update(&request()).unwrap();
        for text in [
            format!("{update:?}"),
            format!("{:?}", update.details),
            format!("{:?}", update.details.setup),
        ] {
            assert!(!text.contains("PRIVATE-FIXTURE-PASSWORD"));
            assert!(!text.contains("ice-ufrag"));
            assert!(text.contains("redacted"));
        }
    }
}
