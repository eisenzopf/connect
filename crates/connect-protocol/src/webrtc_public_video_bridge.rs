//! Explicit signed pre-F B decode negotiation. No value grants media, F,
//! post-F readiness, publication, OLD retirement or public presentation.
use crate::{
    transitions::{id, TransitionIntent, TransitionRequest},
    webrtc_public_video_phase::{Binding, Context},
    Error, Result,
};
use serde::{Deserialize, Serialize};
pub const EXTENSION: &str = "receiver_video_bridge";
pub const PROFILE: &str = "connect/webrtc-public-video-precut-bridge/1";
pub const READY_PROFILE: &str = "connect-replacement-public-video-bridge-ready-v1";
pub const DECODER_STARTUP: &str = "fresh-b-receiver-precut-keyframe";
pub const MAX_CHALLENGE_SECONDS: i64 = 5;

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OptIn {
    pub profile: String,
    pub carrier: String,
    pub presentation: String,
    pub feed: String,
}
impl OptIn {
    pub fn validate(&self) -> Result<()> {
        if self.profile != PROFILE
            || self.carrier != "webrtc-rtp"
            || self.presentation != "exclusive-public"
            || self.feed != "public-programme-private-decode"
        {
            return Err(invalid());
        }
        Ok(())
    }
    /// This is the immutable original SIGNED extension, never a later offer.
    pub fn from_request(request: &TransitionRequest) -> Result<Option<Self>> {
        let Some(value) = request.payload_extensions.get(EXTENSION) else {
            return Ok(None);
        };
        let actual: Self = serde_json::from_value(value.clone()).map_err(|_| invalid())?;
        actual.validate()?;
        let TransitionIntent::Replace {
            video: Some(video), ..
        } = &request.intent
        else {
            return Err(invalid());
        };
        video.validate()?;
        if video.publications.len() != 1
            || video.receivers.len() != 1
            || video.publications[0].replacement == video.receivers[0].replacement
        {
            return Err(invalid());
        }
        Ok(Some(actual))
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Challenge {
    pub nonce: String,
    /// Absolute Unix seconds, selected ONCE by Store and bounded by the original.
    pub expires_at: i64,
    pub decoder_startup: String,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Feeding {
    pub profile: String,
    pub phase: String,
    pub reservation_id: String,
    pub operation_record_id: String,
    pub operation_id: String,
    pub target_connection_id: String,
    pub target_generation: u64,
    pub binding: Binding,
    pub bridge_readiness: Challenge,
}
impl Feeding {
    /// Historical shape/identity validation does not renew or authorize IO.
    pub fn validate(&self, context: &Context<'_>, original_expiry: i64) -> Result<()> {
        context.intent.validate()?;
        id(context.private_readiness_nonce)?;
        if context.target_generation == 0 || context.target_generation > 9_007_199_254_740_991 {
            return Err(invalid());
        }
        for v in [
            &self.reservation_id,
            &self.operation_record_id,
            &self.operation_id,
            &self.target_connection_id,
            &self.bridge_readiness.nonce,
        ] {
            id(v)?;
        }
        self.binding.validate(context.intent)?;
        if self.profile != PROFILE
            || self.phase != "feeding"
            || self.reservation_id != context.intent.reservation_id
            || self.operation_record_id != context.operation_record_id
            || self.operation_id != context.operation_id
            || self.target_connection_id != context.target_connection_id
            || self.target_generation != context.target_generation
            || self.bridge_readiness.nonce == context.intent.reservation_id
            || self.bridge_readiness.nonce == context.intent.nonce
            || self.bridge_readiness.nonce == context.private_readiness_nonce
            || self.bridge_readiness.decoder_startup != DECODER_STARTUP
            || self.bridge_readiness.expires_at <= 0
            || self.bridge_readiness.expires_at > 9_007_199_254_740_991
            || self.bridge_readiness.expires_at > original_expiry
        {
            return Err(invalid());
        }
        Ok(())
    }
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecodedObservation {
    pub reservation_id: String,
    pub nonce: String,
    pub binding: Binding,
    /// Exact receiver rVFC RTP coordinate. No wall-clock or counter estimate.
    pub decoded_rtp_timestamp: u32,
    pub decoded_frames: u64,
    pub decoder_startup: String,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadyMarker {
    pub profile: String,
    pub operation_record_id: String,
    pub bridge_readiness: DecodedObservation,
}
impl ReadyMarker {
    pub fn validate_shape(&self) -> Result<()> {
        id(&self.operation_record_id)?;
        let r = &self.bridge_readiness;
        id(&r.reservation_id)?;
        id(&r.nonce)?;
        r.binding.validate_shape()?;
        if self.profile != READY_PROFILE
            || r.decoded_frames == 0
            || r.decoded_frames > 9_007_199_254_740_991
            || r.decoder_startup != DECODER_STARTUP
        {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn validate(&self, feeding: &Feeding) -> Result<()> {
        self.validate_shape()?;
        let r = &self.bridge_readiness;
        if self.operation_record_id != feeding.operation_record_id
            || r.reservation_id != feeding.reservation_id
            || r.nonce != feeding.bridge_readiness.nonce
            || r.binding != feeding.binding
            || r.decoder_startup != feeding.bridge_readiness.decoder_startup
        {
            return Err(invalid());
        }
        Ok(())
    }
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub opt_in: OptIn,
    pub feeding: Option<Feeding>,
    pub decoded_ready: Option<ReadyMarker>,
}
fn invalid() -> Error {
    Error::new(
        "invalid-request",
        "Invalid signed pre-cut public video bridge",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_bridge_opt_in_and_decoded_marker_are_closed_and_not_post_f_ready() {
        let o = serde_json::json!({"profile":PROFILE,"carrier":"webrtc-rtp","presentation":"exclusive-public","feed":"public-programme-private-decode"});
        serde_json::from_value::<OptIn>(o.clone())
            .unwrap()
            .validate()
            .unwrap();
        let mut bad = o.clone();
        bad["source_cut_id"] = serde_json::json!("invented");
        assert!(serde_json::from_value::<OptIn>(bad).is_err());
        let b = Binding {
            stream_id: "b".into(),
            preview_stream_id: "a".into(),
            mid: "1".into(),
            stream_generation: 2,
            physical_writer_incarnation: "writer".into(),
            connection_generation: 2,
            primary_ssrc: 7,
            payload_type: 96,
            clock_rate_hz: 90000,
            codec: "vp8".into(),
        };
        let f = Feeding {
            profile: PROFILE.into(),
            phase: "feeding".into(),
            reservation_id: "res".into(),
            operation_record_id: "record".into(),
            operation_id: "op".into(),
            target_connection_id: "target".into(),
            target_generation: 2,
            binding: b.clone(),
            bridge_readiness: Challenge {
                nonce: "challenge".into(),
                expires_at: 100,
                decoder_startup: DECODER_STARTUP.into(),
            },
        };
        let m = ReadyMarker {
            profile: READY_PROFILE.into(),
            operation_record_id: "record".into(),
            bridge_readiness: DecodedObservation {
                reservation_id: "res".into(),
                nonce: "challenge".into(),
                binding: b,
                decoded_rtp_timestamp: 0,
                decoded_frames: 1,
                decoder_startup: DECODER_STARTUP.into(),
            },
        };
        m.validate(&f).unwrap();
        let mut wrong = m.clone();
        wrong.bridge_readiness.binding.primary_ssrc += 1;
        assert!(wrong.validate(&f).is_err());
        let mut wrong = m.clone();
        wrong.bridge_readiness.nonce = "other".into();
        assert!(wrong.validate(&f).is_err());
        let mut wrong = m.clone();
        wrong.bridge_readiness.decoded_frames = 0;
        assert!(wrong.validate(&f).is_err());
        let value = serde_json::to_value(&m).unwrap();
        assert!(
            serde_json::from_value::<crate::webrtc_public_video_phase::ReadyMarker>(value.clone())
                .is_err()
        );
        let mut unknown = value;
        unknown["source_cut_id"] = serde_json::json!("none");
        assert!(serde_json::from_value::<ReadyMarker>(unknown).is_err());
    }
}
