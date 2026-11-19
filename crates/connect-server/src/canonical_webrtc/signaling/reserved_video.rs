//! Exact original optional A/B signaling. A JSON marker never creates its class.
use super::*;
use connect_service::canonical_webrtc::ReservedReplacementVideo;
use rvoip_webrtc::canonical::ReservedVideoProfile;

pub(super) fn validate_offer(
    offer: &serde_json::Value,
    av: bool,
    original: &ReservedReplacementVideo,
) -> Result<()> {
    if !av {
        return Err(unsupported());
    }
    original.validate_offer(offer)?;
    validate_shape(offer, original.profile())
}
fn validate_shape(offer: &serde_json::Value, profile: &ReservedVideoProfile) -> Result<()> {
    if !matches!(
        offer["replacement_preparation"]["profile"].as_str(),
        Some(
            "connect-replacement-preparation-v1"
                | "connect-replacement-media-v1"
                | "connect-replacement-ready-v1"
        )
    ) {
        return Err(unsupported());
    }
    let streams = offer["streams_offered"]
        .as_array()
        .ok_or_else(unsupported)?;
    if streams.len() != 3 {
        return Err(unsupported());
    }
    for (id, kind, direction) in [
        (profile.audio_stream_id(), "audio", "sendrecv"),
        (profile.preview_stream_id(), "video", "sendrecv"),
        (profile.public_stream_id(), "video", "recvonly"),
    ] {
        let actual = streams
            .iter()
            .filter(|stream| stream["id"].as_str() == Some(id.as_str()))
            .collect::<Vec<_>>();
        if actual.len() != 1 || actual[0]["kind"] != kind || actual[0]["direction"] != direction {
            return Err(unsupported());
        }
    }
    profile
        .validate_sdp_offer(
            offer["substrate_setup"]["sdp"]
                .as_str()
                .ok_or_else(unsupported)?,
        )
        .map_err(|_| unsupported())
}
pub(super) fn selected_answer(
    participant: &str,
    offer: &serde_json::Value,
    answer: &str,
    av: bool,
    original: &ReservedReplacementVideo,
) -> Result<ConnectionAnswer> {
    validate_offer(offer, av, original)?;
    selected_shape(participant, offer, answer, original.profile())
}
fn selected_shape(
    participant: &str,
    offer: &serde_json::Value,
    answer: &str,
    profile: &ReservedVideoProfile,
) -> Result<ConnectionAnswer> {
    use rvoip_webrtc::media::core_audio_profile;
    validate_shape(offer, profile)?;
    let original = offer["substrate_setup"]["sdp"]
        .as_str()
        .ok_or_else(unsupported)?;
    let o =
        rvoip_sip_core::types::sdp::SdpSession::from_str(original).map_err(|_| unsupported())?;
    let a = rvoip_sip_core::types::sdp::SdpSession::from_str(answer).map_err(|_| unsupported())?;
    if o.media_descriptions.len() != 3 || a.media_descriptions.len() != 3 {
        return Err(unsupported());
    }
    for (index, (offered, selected)) in o
        .media_descriptions
        .iter()
        .zip(&a.media_descriptions)
        .enumerate()
    {
        if offered.media != selected.media
            || offered.protocol != selected.protocol
            || offered.port == 0
            || selected.port == 0
            || media_mid(offered)? != media_mid(selected)?
            || direction(selected, &a) != (if index == 2 { "sendonly" } else { "sendrecv" })
        {
            return Err(unsupported());
        }
    }
    let (audio, codec) =
        core_audio_profile::negotiated_profile(original, answer).map_err(|_| unsupported())?;
    let (preview, public) = profile
        .negotiated_video_profiles(original, answer)
        .map_err(|_| unsupported())?;
    let mut selected = Vec::new();
    for stream in offer["streams_offered"]
        .as_array()
        .ok_or_else(unsupported)?
    {
        let id = stream["id"].as_str().ok_or_else(unsupported)?;
        let (codec, direction) = if id == profile.audio_stream_id().as_str() {
            (
                json!({"name":codec.name,"params":{"sample_rate":codec.clock_rate_hz,"channels":codec.channels,"payload_type":audio.payload_type}}),
                "sendrecv",
            )
        } else {
            let (pt, direction) = if id == profile.preview_stream_id().as_str() {
                (preview.payload_type, "sendrecv")
            } else if id == profile.public_stream_id().as_str() {
                (public.payload_type, "sendonly")
            } else {
                return Err(unsupported());
            };
            (
                json!({"name":"vp8","params":{"sample_rate":90000,"channels":0,"payload_type":pt}}),
                direction,
            )
        };
        selected.push(json!({"id":id,"kind":stream["kind"],"direction":direction,"codec":codec}));
    }
    serde_json::from_value(json!({"by_participant":participant,"substrate":"websocket+webrtc", "capabilities":{"audio_codecs":[{"name":codec.name,"params":{"sample_rate":codec.clock_rate_hz,"channels":codec.channels}}],"video_codecs":[{"name":"vp8","params":{"sample_rate":90000,"channels":0}}],"max_streams_per_connection":3},"streams_answered":selected,"substrate_setup":{"kind":"websocket+webrtc","sdp":answer}})).map_err(|_|engine_error())
}

#[cfg(test)]
mod tests;
