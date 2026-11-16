//! Server-created setup metadata within the authenticated original operation.
//! This intent enables no media, chooses no audience and proves no readiness.
use crate::{transitions::id, Error, Result};
use serde::{Deserialize, Serialize};

pub const PROFILE: &str = "connect/webrtc-public-video-reservation/1";

/// Persist once before the original target offer. Negotiation and the original
/// post-cut decoder observation require distinct later phases; an intent cannot
/// substitute for either. The enclosing operation binds the actor and target.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Intent {
    pub profile: String,
    pub phase: String,
    pub reservation_id: String,
    pub nonce: String,
    pub preview_stream_id: String,
    /// Receive role within A. A may remain one sendrecv camera/preview MID.
    pub preview_role: String,
    /// Distinct B receive stream; its MID is not known before negotiation.
    pub public_stream_id: String,
}

impl Intent {
    pub fn validate(&self) -> Result<()> {
        for value in [
            &self.reservation_id,
            &self.nonce,
            &self.preview_stream_id,
            &self.public_stream_id,
        ] {
            id(value)?;
        }
        if self.profile != PROFILE
            || self.phase != "intent"
            || self.preview_role != "receive"
            || self.preview_stream_id == self.public_stream_id
        {
            return Err(invalid());
        }
        Ok(())
    }

    /// Caller supplies the original frozen mapping, not values from this intent.
    /// Current authorization and native resource custody remain separate.
    pub fn validate_mapping(&self, preview: &str, public: &str) -> Result<()> {
        self.validate()?;
        if self.preview_stream_id != preview || self.public_stream_id != public {
            return Err(invalid());
        }
        Ok(())
    }
}

fn invalid() -> Error {
    Error::new("invalid-request", "Invalid public video reservation intent")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn value() -> Value {
        json!({
            "profile": PROFILE,
            "phase": "intent",
            "reservation_id": "reservation",
            "nonce": "original-nonce",
            "preview_stream_id": "camera-a",
            "preview_role": "receive",
            "public_stream_id": "public-b"
        })
    }

    #[test]
    fn intent_roundtrip_binds_distinct_receive_roles_without_claiming_a_direction() {
        let wire = value();
        let intent: Intent = serde_json::from_value(wire.clone()).unwrap();
        intent.validate_mapping("camera-a", "public-b").unwrap();
        assert_eq!(serde_json::to_value(intent).unwrap(), wire);
    }

    #[test]
    fn frozen_mapping_and_phase_cannot_be_substituted() {
        let intent: Intent = serde_json::from_value(value()).unwrap();
        assert!(intent.validate_mapping("other-a", "public-b").is_err());
        assert!(intent.validate_mapping("camera-a", "other-b").is_err());
        assert!(intent.validate_mapping("public-b", "camera-a").is_err());
        for (field, bad) in [
            ("profile", json!("connect/replacement-media-v1")),
            ("phase", json!("cut-bound-post-f")),
            ("preview_role", json!("recvonly")),
            ("public_stream_id", json!("camera-a")),
            ("reservation_id", json!("")),
            ("nonce", json!("bad\nnonce")),
        ] {
            let mut changed = value();
            changed[field] = bad;
            let changed: Intent = serde_json::from_value(changed).unwrap();
            assert!(changed.validate().is_err(), "{field}");
        }
    }

    #[test]
    fn intent_refuses_readiness_cut_transport_and_authority_fields() {
        for field in [
            "source_cut_id",
            "mid",
            "target_primary_ssrc",
            "stream_generation",
            "connection_generation",
            "physical_writer_incarnation",
            "ready",
            "decoded_frames",
            "permission",
            "audience",
            "track_id",
        ] {
            let mut changed = value();
            changed[field] = json!("forged");
            assert!(
                serde_json::from_value::<Intent>(changed).is_err(),
                "{field}"
            );
        }
        for field in [
            "profile",
            "phase",
            "reservation_id",
            "nonce",
            "preview_stream_id",
            "preview_role",
            "public_stream_id",
        ] {
            let mut changed = value();
            changed.as_object_mut().unwrap().remove(field);
            assert!(
                serde_json::from_value::<Intent>(changed).is_err(),
                "{field}"
            );
        }
    }
}
