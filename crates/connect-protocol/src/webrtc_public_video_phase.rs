//! Closed public B phase metadata within one authenticated original operation.
//! Wire metadata alone grants no authority, native custody, source cut or readiness.
use crate::{transitions::id, webrtc_video_receive_reservation::Intent, Error, Result};
use serde::{Deserialize, Serialize};

pub const PROFILE: &str = "connect/webrtc-public-video-phase/1";
pub const READY_PROFILE: &str = "connect-replacement-public-video-ready-v1";
pub const DECODER_STARTUP: &str = "fresh-receiver-first-decoded-frame";

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub stream_id: String,
    pub preview_stream_id: String,
    pub mid: String,
    pub stream_generation: u64,
    pub physical_writer_incarnation: String,
    pub connection_generation: u64,
    pub primary_ssrc: u32,
    pub payload_type: u8,
    pub clock_rate_hz: u32,
    pub codec: String,
}
impl Binding {
    pub fn validate(&self, intent: &Intent) -> Result<()> {
        intent.validate_mapping(&self.preview_stream_id, &self.stream_id)?;
        opaque(&self.physical_writer_incarnation)?;
        if self.mid.is_empty()
            || self.mid.len() > 256
            || !self.mid.bytes().all(|b| (0x21..=0x7e).contains(&b))
            || !positive(self.stream_generation)
            || !positive(self.connection_generation)
            || self.payload_type > 127
            || self.clock_rate_hz != 90_000
            || self.codec != "vp8"
        {
            return Err(invalid());
        }
        Ok(())
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Challenge {
    pub nonce: String,
    pub decoder_startup: String,
}
impl Challenge {
    pub fn validate(&self, intent: &Intent, private_nonce: &str) -> Result<()> {
        id(&self.nonce)?;
        if self.nonce == intent.nonce
            || self.nonce == private_nonce
            || self.decoder_startup != DECODER_STARTUP
        {
            return Err(invalid());
        }
        Ok(())
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DiscardReason {
    Cancelled,
    Expired,
    Refused,
    OwnerRetired,
    BindingChanged,
    CutUnavailable,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", deny_unknown_fields)]
pub enum Phase {
    #[serde(rename = "negotiated-never-fed")]
    NegotiatedNeverFed {
        profile: String,
        reservation_id: String,
        operation_record_id: String,
        operation_id: String,
        target_connection_id: String,
        target_generation: u64,
        binding: Binding,
    },
    #[serde(rename = "cut-bound-post-f")]
    CutBoundPostF {
        profile: String,
        reservation_id: String,
        operation_record_id: String,
        operation_id: String,
        target_connection_id: String,
        target_generation: u64,
        binding: Binding,
        source_cut_id: String,
        public_readiness: Challenge,
    },
    #[serde(rename = "decoded-ready")]
    DecodedReady {
        profile: String,
        reservation_id: String,
        operation_record_id: String,
        operation_id: String,
        target_connection_id: String,
        target_generation: u64,
        binding: Binding,
        source_cut_id: String,
        public_readiness: Challenge,
    },
    #[serde(rename = "activated")]
    Activated {
        profile: String,
        reservation_id: String,
        operation_record_id: String,
        operation_id: String,
        target_connection_id: String,
        target_generation: u64,
        binding: Binding,
        source_cut_id: String,
        public_readiness: Challenge,
    },
    #[serde(rename = "discarded")]
    Discarded {
        profile: String,
        reservation_id: String,
        operation_record_id: String,
        operation_id: String,
        target_connection_id: String,
        target_generation: u64,
        binding: Binding,
        #[serde(deserialize_with = "required_nullable")]
        source_cut_id: Option<String>,
        reason: DiscardReason,
    },
}

/// Exact original server-owned context; generation namespaces are independent.
pub struct Context<'a> {
    pub intent: &'a Intent,
    pub operation_record_id: &'a str,
    pub operation_id: &'a str,
    pub target_connection_id: &'a str,
    pub target_generation: u64,
    pub private_readiness_nonce: &'a str,
    pub completed: bool,
}
impl Phase {
    /// Shape/context validation only. The owner must additionally enforce
    /// monotonic persisted phases, immutable native binding/cut and actual custody.
    pub fn validate(&self, context: &Context<'_>) -> Result<()> {
        context.intent.validate()?;
        for value in [
            context.operation_record_id,
            context.operation_id,
            context.target_connection_id,
            context.private_readiness_nonce,
        ] {
            id(value)?;
        }
        if !positive(context.target_generation) {
            return Err(invalid());
        }
        match self {
            Self::NegotiatedNeverFed {
                profile,
                reservation_id,
                operation_record_id,
                operation_id,
                target_connection_id,
                target_generation,
                binding,
            } => {
                if profile != PROFILE
                    || reservation_id != &context.intent.reservation_id
                    || operation_record_id != context.operation_record_id
                    || operation_id != context.operation_id
                    || target_connection_id != context.target_connection_id
                    || *target_generation != context.target_generation
                {
                    return Err(invalid());
                }
                binding.validate(context.intent)?;
            }
            Self::CutBoundPostF {
                profile,
                reservation_id,
                operation_record_id,
                operation_id,
                target_connection_id,
                target_generation,
                binding,
                source_cut_id,
                public_readiness,
            } => {
                if profile != PROFILE
                    || reservation_id != &context.intent.reservation_id
                    || operation_record_id != context.operation_record_id
                    || operation_id != context.operation_id
                    || target_connection_id != context.target_connection_id
                    || *target_generation != context.target_generation
                {
                    return Err(invalid());
                }
                binding.validate(context.intent)?;
                opaque(source_cut_id)?;
                public_readiness.validate(context.intent, context.private_readiness_nonce)?;
            }
            Self::DecodedReady {
                profile,
                reservation_id,
                operation_record_id,
                operation_id,
                target_connection_id,
                target_generation,
                binding,
                source_cut_id,
                public_readiness,
            } => {
                if profile != PROFILE
                    || reservation_id != &context.intent.reservation_id
                    || operation_record_id != context.operation_record_id
                    || operation_id != context.operation_id
                    || target_connection_id != context.target_connection_id
                    || *target_generation != context.target_generation
                {
                    return Err(invalid());
                }
                binding.validate(context.intent)?;
                opaque(source_cut_id)?;
                public_readiness.validate(context.intent, context.private_readiness_nonce)?;
            }
            Self::Activated {
                profile,
                reservation_id,
                operation_record_id,
                operation_id,
                target_connection_id,
                target_generation,
                binding,
                source_cut_id,
                public_readiness,
            } => {
                if profile != PROFILE
                    || reservation_id != &context.intent.reservation_id
                    || operation_record_id != context.operation_record_id
                    || operation_id != context.operation_id
                    || target_connection_id != context.target_connection_id
                    || *target_generation != context.target_generation
                {
                    return Err(invalid());
                }
                binding.validate(context.intent)?;
                opaque(source_cut_id)?;
                public_readiness.validate(context.intent, context.private_readiness_nonce)?;
                if !context.completed {
                    return Err(invalid());
                }
            }
            Self::Discarded {
                profile,
                reservation_id,
                operation_record_id,
                operation_id,
                target_connection_id,
                target_generation,
                binding,
                source_cut_id,
                ..
            } => {
                if profile != PROFILE
                    || reservation_id != &context.intent.reservation_id
                    || operation_record_id != context.operation_record_id
                    || operation_id != context.operation_id
                    || target_connection_id != context.target_connection_id
                    || *target_generation != context.target_generation
                {
                    return Err(invalid());
                }
                binding.validate(context.intent)?;
                if let Some(cut) = source_cut_id {
                    opaque(cut)?;
                }
            }
        }
        Ok(())
    }
}
fn required_nullable<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<String>, D::Error> {
    Option::<String>::deserialize(d)
}
fn positive(v: u64) -> bool {
    v > 0 && v <= 9_007_199_254_740_991
}
fn opaque(v: &str) -> Result<()> {
    if v.is_empty() || v.len() > 256 || !v.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
        Err(invalid())
    } else {
        Ok(())
    }
}
fn invalid() -> Error {
    Error::new("invalid-request", "Invalid public video phase")
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadyMarker {
    pub profile: String,
    pub operation_record_id: String,
    pub public_video_readiness: DecodedObservation,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecodedObservation {
    pub reservation_id: String,
    pub source_cut_id: String,
    pub nonce: String,
    pub binding: Binding,
    pub decoded_frames: u64,
    #[serde(deserialize_with = "required_nullable_count")]
    pub decoded_keyframes: Option<u64>,
    pub decoder_startup: String,
}
impl ReadyMarker {
    /// Exact original challenge match only; a signed observation is not itself
    /// authorization or native media proof. The service retains those duties.
    pub fn validate(&self, context: &Context<'_>, original: &Phase) -> Result<()> {
        original.validate(context)?;
        let (reservation, cut, challenge, binding) = match original {
            Phase::CutBoundPostF {
                reservation_id,
                source_cut_id,
                public_readiness,
                binding,
                ..
            }
            | Phase::DecodedReady {
                reservation_id,
                source_cut_id,
                public_readiness,
                binding,
                ..
            }
            | Phase::Activated {
                reservation_id,
                source_cut_id,
                public_readiness,
                binding,
                ..
            } => (reservation_id, source_cut_id, public_readiness, binding),
            _ => return Err(invalid()),
        };
        let r = &self.public_video_readiness;
        if self.profile != READY_PROFILE
            || self.operation_record_id != context.operation_record_id
            || &r.reservation_id != reservation
            || &r.source_cut_id != cut
            || r.nonce != challenge.nonce
            || &r.binding != binding
            || r.decoder_startup != DECODER_STARTUP
            || !positive(r.decoded_frames)
            || r.decoded_keyframes
                .is_some_and(|keys| !positive(keys) || keys > r.decoded_frames)
        {
            return Err(invalid());
        }
        Ok(())
    }
}
fn required_nullable_count<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<u64>, D::Error> {
    Option::<u64>::deserialize(d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    fn intent() -> Intent {
        serde_json::from_value(json!({"profile":crate::webrtc_video_receive_reservation::PROFILE,"phase":"intent","reservation_id":"reservation","nonce":"intent-nonce","preview_stream_id":"camera-a","preview_role":"receive","public_stream_id":"public-b"})).unwrap()
    }
    fn context(intent: &Intent, completed: bool) -> Context<'_> {
        Context {
            intent,
            operation_record_id: "record",
            operation_id: "move",
            target_connection_id: "target",
            target_generation: 2,
            private_readiness_nonce: "private-nonce",
            completed,
        }
    }
    fn binding() -> Value {
        json!({"stream_id":"public-b","preview_stream_id":"camera-a","mid":"2","stream_generation":1,"physical_writer_incarnation":"writer-original","connection_generation":1,"primary_ssrc":0,"payload_type":96,"clock_rate_hz":90000,"codec":"vp8"})
    }
    fn phase(name: &str) -> Value {
        let mut v = json!({"profile":PROFILE,"phase":name,"reservation_id":"reservation","operation_record_id":"record","operation_id":"move","target_connection_id":"target","target_generation":2,"binding":binding()});
        if name == "discarded" {
            v["source_cut_id"] = Value::Null;
            v["reason"] = json!("cancelled");
        } else if name != "negotiated-never-fed" {
            v["source_cut_id"] = json!("original-cut");
            v["public_readiness"] =
                json!({"nonce":"public-nonce","decoder_startup":DECODER_STARTUP});
        }
        v
    }
    fn ready() -> Value {
        json!({"profile":READY_PROFILE,"operation_record_id":"record","public_video_readiness":{"reservation_id":"reservation","source_cut_id":"original-cut","nonce":"public-nonce","binding":binding(),"decoded_frames":1,"decoded_keyframes":null,"decoder_startup":DECODER_STARTUP}})
    }
    #[test]
    fn phases_roundtrip_keep_zero_ssrc_and_separate_generation_namespaces() {
        let i = intent();
        for name in [
            "negotiated-never-fed",
            "cut-bound-post-f",
            "decoded-ready",
            "activated",
            "discarded",
        ] {
            let value = phase(name);
            let p: Phase = serde_json::from_value(value.clone()).unwrap();
            p.validate(&context(&i, true)).unwrap();
            assert_eq!(serde_json::to_value(p).unwrap(), value);
        }
    }
    #[test]
    fn early_phase_has_no_cut_and_discard_requires_explicit_nullable_cut() {
        let mut v = phase("negotiated-never-fed");
        v["source_cut_id"] = json!("forged");
        assert!(serde_json::from_value::<Phase>(v).is_err());
        let mut v = phase("discarded");
        v.as_object_mut().unwrap().remove("source_cut_id");
        assert!(serde_json::from_value::<Phase>(v).is_err());
        let mut v = phase("discarded");
        v["reason"] = json!("unknown");
        assert!(serde_json::from_value::<Phase>(v).is_err());
    }
    #[test]
    fn activation_requires_original_completed_and_nonces_cannot_alias() {
        let i = intent();
        let p: Phase = serde_json::from_value(phase("activated")).unwrap();
        assert!(p.validate(&context(&i, false)).is_err());
        for nonce in ["private-nonce", "intent-nonce"] {
            let mut v = phase("cut-bound-post-f");
            v["public_readiness"]["nonce"] = json!(nonce);
            let p: Phase = serde_json::from_value(v).unwrap();
            assert!(p.validate(&context(&i, false)).is_err());
        }
    }
    #[test]
    fn foreign_context_binding_codec_and_unsafe_generations_are_refused() {
        let i = intent();
        for (path, bad) in [
            ("operation_record_id", json!("foreign")),
            ("operation_id", json!("foreign")),
            ("target_connection_id", json!("foreign")),
            ("target_generation", json!(0)),
            ("target_generation", json!(9_007_199_254_740_992u64)),
        ] {
            let mut v = phase("cut-bound-post-f");
            v[path] = bad;
            let p: Phase = serde_json::from_value(v).unwrap();
            assert!(p.validate(&context(&i, false)).is_err(), "{path}");
        }
        for (field, bad) in [
            ("stream_id", json!("camera-a")),
            ("preview_stream_id", json!("other")),
            ("mid", json!("bad mid")),
            ("physical_writer_incarnation", json!("")),
            ("payload_type", json!(128)),
            ("clock_rate_hz", json!(48000)),
            ("codec", json!("h264")),
            ("connection_generation", json!(0)),
        ] {
            let mut v = phase("cut-bound-post-f");
            v["binding"][field] = bad;
            let p: Phase = serde_json::from_value(v).unwrap();
            assert!(p.validate(&context(&i, false)).is_err(), "{field}");
        }
        let mut v = phase("cut-bound-post-f");
        v["binding"]["primary_ssrc"] = json!(4_294_967_296u64);
        assert!(serde_json::from_value::<Phase>(v).is_err());
    }
    #[test]
    fn closed_phase_binding_and_challenge_reject_extra_fields() {
        for path in ["phase", "binding", "public_readiness"] {
            let mut v = phase("cut-bound-post-f");
            if path == "phase" {
                v["permission"] = json!(true)
            } else {
                v[path]["permission"] = json!(true)
            }
            assert!(serde_json::from_value::<Phase>(v).is_err(), "{path}");
        }
    }
    #[test]
    fn ready_marker_binds_original_b_cut_and_requires_measurable_decoded_frame() {
        let i = intent();
        let c = context(&i, false);
        let p: Phase = serde_json::from_value(phase("cut-bound-post-f")).unwrap();
        let r: ReadyMarker = serde_json::from_value(ready()).unwrap();
        r.validate(&c, &p).unwrap();
        for (field, bad) in [
            ("source_cut_id", json!("foreign")),
            ("nonce", json!("private-nonce")),
            ("reservation_id", json!("foreign")),
            ("decoded_frames", json!(0)),
            ("decoded_keyframes", json!(0)),
            ("decoded_keyframes", json!(2)),
            ("decoder_startup", json!("preview")),
        ] {
            let mut v = ready();
            v["public_video_readiness"][field] = bad;
            let r: ReadyMarker = serde_json::from_value(v).unwrap();
            assert!(r.validate(&c, &p).is_err(), "{field}");
        }
        let mut v = ready();
        v["public_video_readiness"]
            .as_object_mut()
            .unwrap()
            .remove("decoded_keyframes");
        assert!(serde_json::from_value::<ReadyMarker>(v).is_err());
        let mut v = ready();
        v["public_video_readiness"]["binding"]["mid"] = json!("0");
        let r: ReadyMarker = serde_json::from_value(v).unwrap();
        assert!(r.validate(&c, &p).is_err());
        let n: Phase = serde_json::from_value(phase("negotiated-never-fed")).unwrap();
        assert!(r.validate(&c, &n).is_err());
    }
}
