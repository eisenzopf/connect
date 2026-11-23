//! Frame boundary of one originally authorized receiver handoff. These wire
//! values are observations, never a routing grant or a decoder-ready assertion.
use crate::{
    replacement_activation::{NativeSequenceCut, MAX_ORDINAL},
    transitions::id,
    Error, Result,
};
use serde::{Deserialize, Serialize};

pub const PROFILE: &str = "connect-receiver-video-release-v1";
pub const POST_CUT_PROFILE: &str = "connect-receiver-video-post-cut-v1";

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "carrier", rename_all = "kebab-case", deny_unknown_fields)]
pub enum PresentationFloor {
    WebrtcRtp {
        physical_writer_incarnation: String,
        primary_ssrc: u32,
        first_public_rtp_timestamp: u32,
    },
    NativeDatagram {
        cut: NativeSequenceCut,
    },
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OriginalFrameFloor {
    pub realm: String,
    pub last_old_ordinal: String,
    pub ordinal: String,
}

/// This describes the first allowed original public source frame and its
/// carrier floor. Private decode may include older frames on the same writer.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiverVideoRelease {
    pub profile: String,
    pub operation_record_id: String,
    pub operation_id: String,
    pub reservation_id: String,
    pub source_connection_id: String,
    pub target_connection_id: String,
    pub stream_id: String,
    pub stream_generation: u64,
    pub connection_generation: u64,
    pub target_generation: u64,
    pub source_cut_id: String,
    pub original_frame: OriginalFrameFloor,
    pub floor: PresentationFloor,
}

impl ReceiverVideoRelease {
    pub fn validate(&self) -> Result<()> {
        self.validate_profile(PROFILE)
    }

    /// Exact candidate writer output at the original F. It cannot be parsed
    /// as a release and does not authorize public presentation.
    pub fn validate_post_cut(&self) -> Result<()> {
        self.validate_profile(POST_CUT_PROFILE)
    }

    fn validate_profile(&self, profile: &str) -> Result<()> {
        if self.profile != profile
            || self.source_connection_id == self.target_connection_id
            || [
                self.target_generation,
                self.connection_generation,
                self.stream_generation,
            ]
            .into_iter()
            .any(|value| !(1..=MAX_ORDINAL).contains(&value))
        {
            return Err(invalid());
        }
        for identifier in [
            &self.operation_record_id,
            &self.operation_id,
            &self.reservation_id,
            &self.source_connection_id,
            &self.target_connection_id,
            &self.stream_id,
            &self.source_cut_id,
        ] {
            id(identifier)?;
        }
        let frame = &self.original_frame;
        if !nonzero_hex(&frame.realm)
            || ordinal(&frame.ordinal)? <= ordinal(&frame.last_old_ordinal)?
        {
            return Err(invalid());
        }
        match &self.floor {
            PresentationFloor::WebrtcRtp {
                physical_writer_incarnation,
                ..
            } => {
                if !nonzero_hex(physical_writer_incarnation) {
                    return Err(invalid());
                }
            }
            PresentationFloor::NativeDatagram { cut } => {
                cut.validate()?;
                if cut.stream_id != self.stream_id
                    || cut.stream_generation != self.stream_generation
                {
                    return Err(invalid());
                }
            }
        }
        Ok(())
    }
}

fn nonzero_hex(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && value.bytes().any(|b| b != b'0')
}

fn ordinal(value: &str) -> Result<u64> {
    if value.is_empty()
        || value.len() > 20
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(invalid());
    }
    value.parse().map_err(|_| invalid())
}

fn invalid() -> Error {
    Error::new("bad-parameters", "Invalid receiver video release boundary")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provisional_coordinate_cannot_be_used_as_an_activated_release() {
        let mut value: ReceiverVideoRelease = serde_json::from_value(serde_json::json!({
            "profile": POST_CUT_PROFILE, "operation_record_id": "record", "operation_id": "move",
            "reservation_id": "reservation", "source_connection_id": "old", "target_connection_id": "new",
            "stream_id": "public-b", "stream_generation": 1, "connection_generation": 1,
            "target_generation": 2, "source_cut_id": "cut",
            "original_frame": {"realm":"00000000000000000000000000000001","last_old_ordinal":"42","ordinal":"43"},
            "floor": {"carrier":"webrtc-rtp","physical_writer_incarnation":"00000000000000000000000000000002",
                "primary_ssrc": 7,"first_public_rtp_timestamp": 0}
        })).unwrap();
        assert!(value.validate_post_cut().is_ok());
        assert!(value.validate().is_err());
        value.profile = PROFILE.into();
        assert!(value.validate_post_cut().is_err());
        assert!(value.validate().is_ok());
        value.original_frame.ordinal = "42".into();
        assert!(value.validate().is_err());
    }
}
