//! Original physical replacement decoder evidence. Signed endpoint observations
//! are not authority: actual current Core resources and Store admission remain
//! separate requirements. No HTTP call can supply physical readiness.
use crate::{transitions::id, Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
const MAX: u64 = 9_007_199_254_740_991;
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Challenge {
    pub nonce: String,
    pub source_generation: u64,
    pub target_generation: u64,
    pub stream_revision: u64,
}
impl Challenge {
    pub fn validate(&self) -> Result<()> {
        id(&self.nonce)?;
        if self.source_generation == 0
            || self.source_generation.checked_add(1) != Some(self.target_generation)
            || self.target_generation > MAX
            || self.stream_revision != 2
        {
            return Err(invalid());
        }
        Ok(())
    }
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioObservation {
    pub stream_id: String,
    pub decoded_samples: u64,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VideoObservation {
    pub stream_id: String,
    pub decoded_frames: u64,
    pub decoded_keyframes: Option<u64>,
    pub decoder_startup: String,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub nonce: String,
    pub source_generation: u64,
    pub target_generation: u64,
    pub stream_revision: u64,
    pub audio: AudioObservation,
    pub video: Option<VideoObservation>,
}
impl Observation {
    pub fn validate(&self) -> Result<()> {
        Challenge {
            nonce: self.nonce.clone(),
            source_generation: self.source_generation,
            target_generation: self.target_generation,
            stream_revision: self.stream_revision,
        }
        .validate()?;
        id(&self.audio.stream_id)?;
        if !(1..=MAX).contains(&self.audio.decoded_samples) {
            return Err(invalid());
        }
        if let Some(video) = &self.video {
            id(&video.stream_id)?;
            if !(1..=MAX).contains(&video.decoded_frames)
                || video
                    .decoded_keyframes
                    .is_some_and(|n| n == 0 || n > video.decoded_frames)
                || video.decoder_startup != "fresh-peer-first-decoded-frame"
            {
                return Err(invalid());
            }
        }
        Ok(())
    }
}
fn invalid() -> Error {
    Error::new("invalid-request", "Invalid original replacement readiness")
}
/// Validate the signed wire shape. Start admission separately requires a
/// two-field client media marker, so clients cannot inject a server challenge.
/// A generated server answer/offer may carry the three-field media challenge.
pub fn validate_marker(value: &Value) -> Result<()> {
    let m = value.as_object().ok_or_else(invalid)?;
    id(m.get("operation_record_id")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?)?;
    let profile = m
        .get("profile")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    match profile {
        "connect-replacement-preparation-v1" if m.len() == 2 => Ok(()),
        "connect-replacement-media-v1" if m.len() == 2 => Ok(()),
        "connect-replacement-media-v1" if m.len() == 3 => {
            serde_json::from_value::<Challenge>(m.get("readiness").ok_or_else(invalid)?.clone())
                .map_err(|_| invalid())?
                .validate()
        }
        "connect-replacement-ready-v1" if m.len() == 3 => {
            serde_json::from_value::<Observation>(m.get("readiness").ok_or_else(invalid)?.clone())
                .map_err(|_| invalid())?
                .validate()
        }
        _ => Err(invalid()),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn decoder_marker_requires_original_challenge_and_actual_positive_observations() {
        let marker = json!({"profile":"connect-replacement-ready-v1","operation_record_id":"original","readiness":{"nonce":"nonce","source_generation":1,"target_generation":2,"stream_revision":2,"audio":{"stream_id":"audio","decoded_samples":960},"video":{"stream_id":"video","decoded_frames":1,"decoded_keyframes":1,"decoder_startup":"fresh-peer-first-decoded-frame"}}});
        assert!(validate_marker(&marker).is_ok());
        for (path, value) in [
            (vec!["audio", "decoded_samples"], json!(0)),
            (vec!["video", "decoded_frames"], json!(0)),
            (vec!["video", "decoded_keyframes"], json!(0)),
            (vec!["video", "decoded_keyframes"], json!(2)),
            (vec!["target_generation"], json!(3)),
            (vec!["stream_revision"], json!(1)),
            (vec!["nonce"], json!("")),
        ] {
            let mut changed = marker.clone();
            let mut at = &mut changed["readiness"];
            for key in path {
                at = &mut at[key];
            }
            *at = value;
            assert!(validate_marker(&changed).is_err());
        }
        let mut forged = marker.clone();
        forged["readiness"]["ready"] = json!(true);
        assert!(validate_marker(&forged).is_err());
        let mut browser = marker;
        browser["readiness"]["video"]["decoded_keyframes"] = Value::Null;
        assert!(validate_marker(&browser).is_ok()); // Native policy requires its actual counter separately.
    }
    #[test]
    fn generated_challenge_is_bounded_metadata_and_never_an_observation() {
        let mut marker = json!({"profile":"connect-replacement-media-v1","operation_record_id":"original","readiness":{"nonce":"nonce","source_generation":1,"target_generation":2,"stream_revision":2}});
        assert!(validate_marker(&marker).is_ok());
        marker["profile"] = json!("connect-replacement-ready-v1");
        assert!(validate_marker(&marker).is_err());
        marker["profile"] = json!("connect-replacement-preparation-v1");
        assert!(validate_marker(&marker).is_err());
    }
}
