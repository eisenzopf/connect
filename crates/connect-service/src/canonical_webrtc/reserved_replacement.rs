//! Resource class derived only from an actual Store-issued original intent.
//! No JSON constructor, media authority, cut, readiness or publication proof.
use super::*;
use connect_protocol::webrtc_video_receive_reservation::Intent;
use connect_store::NativeReplacementPublicVideoReservation;
use rvoip_core::{
    ids::StreamId,
    stream_lifecycle::{MediaFormat, Medium, NegotiatedStream},
};
use rvoip_webrtc::canonical::ReservedVideoProfile;

pub struct ReservedReplacementVideo {
    original: Arc<NativeReplacementPublicVideoReservation>,
    profile: ReservedVideoProfile,
}
impl ReservedReplacementVideo {
    pub(crate) fn from_recorded(
        original: Arc<NativeReplacementPublicVideoReservation>,
        record: &str,
        offer: &serde_json::Value,
    ) -> Result<Arc<Self>> {
        if original.operation_record_id() != record {
            return Err(Error::forbidden());
        }
        original.intent().validate()?;
        let streams = offer["streams_offered"]
            .as_array()
            .ok_or_else(Error::forbidden)?;
        let audio = streams
            .iter()
            .filter(|s| s["kind"] == "audio")
            .collect::<Vec<_>>();
        let [audio] = audio.as_slice() else {
            return Err(Error::forbidden());
        };
        if audio["direction"] != "sendrecv" {
            return Err(Error::forbidden());
        }
        let audio = audio["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or_else(Error::forbidden)?;
        let profile = ReservedVideoProfile::new(
            StreamId::from_string(audio),
            StreamId::from_string(&original.intent().preview_stream_id),
            StreamId::from_string(&original.intent().public_stream_id),
        )
        .map_err(|_| Error::forbidden())?;
        let value = Arc::new(Self { original, profile });
        value.validate_offer(offer)?;
        Ok(value)
    }
    pub fn intent(&self) -> &Intent {
        self.original.intent()
    }
    pub fn operation_record_id(&self) -> &str {
        self.original.operation_record_id()
    }
    pub fn profile(&self) -> &ReservedVideoProfile {
        &self.profile
    }
    pub fn validate_offer(&self, offer: &serde_json::Value) -> Result<()> {
        validate_recorded_echo(offer, self.intent(), self.operation_record_id())?;
        validate_roles(offer, self.profile())
    }
    pub fn validate_selected(&self, streams: &[NegotiatedStream]) -> Result<()> {
        self.profile
            .validate_streams(streams.iter().map(|s| &s.requested))
            .map_err(|_| Error::forbidden())?;
        for stream in streams
            .iter()
            .filter(|s| s.requested.medium == Medium::Video)
        {
            if !matches!(&stream.format,MediaFormat::Encoded {name,clock_rate_hz:90000,channels:0,fmtp} if name.eq_ignore_ascii_case("vp8") && fmtp.as_ref().is_none_or(|v|v.is_empty()))
            {
                return Err(Error::forbidden());
            }
        }
        Ok(())
    }
}
/// Pure role check reused before allocation and by actual server signaling.
/// Inputs are already scoped by the opaque original class above; no opt-in.
fn validate_roles(offer: &serde_json::Value, profile: &ReservedVideoProfile) -> Result<()> {
    let streams = offer["streams_offered"]
        .as_array()
        .ok_or_else(Error::forbidden)?;
    if streams.len() != 3 {
        return Err(Error::forbidden());
    }
    for (id, kind, direction) in [
        (profile.audio_stream_id(), "audio", "sendrecv"),
        (profile.preview_stream_id(), "video", "sendrecv"),
        (profile.public_stream_id(), "video", "recvonly"),
    ] {
        let found = streams
            .iter()
            .filter(|s| s["id"].as_str() == Some(id.as_str()))
            .collect::<Vec<_>>();
        if found.len() != 1 || found[0]["kind"] != kind || found[0]["direction"] != direction {
            return Err(Error::forbidden());
        }
    }
    // Native SDP/codec parsing belongs to the server resource worker after
    // these original Store/Core control Readers have been released.
    Ok(())
}

fn validate_recorded_echo(
    offer: &serde_json::Value,
    expected: &Intent,
    record: &str,
) -> Result<()> {
    let echo: Intent = serde_json::from_value(offer["webrtc_video_receive_reservation"].clone())
        .map_err(|_| Error::forbidden())?;
    echo.validate()?;
    if &echo != expected
        || offer["replacement_preparation"]["operation_record_id"].as_str() != Some(record)
    {
        return Err(Error::forbidden());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recorded_intent_echo_is_closed_exact_and_cannot_be_swapped_for_foreign_root_or_new_phase() {
        let expected:Intent=serde_json::from_value(serde_json::json!({"profile":"connect/webrtc-public-video-reservation/1","phase":"intent","reservation_id":"reservation-original","nonce":"intent-original","preview_stream_id":"preview-a","preview_role":"receive","public_stream_id":"public-b"})).unwrap();
        let offer = serde_json::json!({"replacement_preparation":{"operation_record_id":"original-root"},"webrtc_video_receive_reservation":expected});
        validate_recorded_echo(&offer, &expected, "original-root").unwrap();
        for (key, value) in [
            ("nonce", "foreign"),
            ("phase", "cut-bound-post-f"),
            ("public_stream_id", "preview-a"),
            ("preview_role", "send"),
            ("reservation_id", "foreign-reservation"),
        ] {
            let mut bad = offer.clone();
            bad["webrtc_video_receive_reservation"][key] = serde_json::json!(value);
            assert!(validate_recorded_echo(&bad, &expected, "original-root").is_err());
        }
        let mut foreign = offer.clone();
        foreign["replacement_preparation"]["operation_record_id"] =
            serde_json::json!("foreign-root");
        assert!(validate_recorded_echo(&foreign, &expected, "original-root").is_err());
        let mut extra = offer.clone();
        extra["webrtc_video_receive_reservation"]["ready"] = serde_json::json!(true);
        assert!(validate_recorded_echo(&extra, &expected, "original-root").is_err());
        assert!(
            validate_recorded_echo(&serde_json::json!({}), &expected, "original-root").is_err()
        );
    }
}
