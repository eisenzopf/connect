//! Protected canonical SDP host using the exact Core-reserved RTC context.
use super::*;
use async_trait::async_trait;
use connect_service::canonical_webrtc::{Attempt, Host, Preparation};
use rvoip_uctp::v2::payloads::ConnectionAnswer;
use serde_json::json;

/// Bounded mono audio or mono audio plus one VP8 camera signaling profile.
/// It is deliberately not installed or advertised by the default server config.
pub struct SignalingHost {
    config: WebRtcConfig,
    slots: Arc<tokio::sync::Semaphore>,
    adapter: Arc<rvoip_webrtc::canonical::CanonicalWebRtcAdapter>,
}
impl SignalingHost {
    /// One exact mono audio codec; Opus retains its required SDP /2 mapping.
    /// The explicit AV flag admits one duplex VP8 camera alongside audio.
    pub fn new(config: WebRtcConfig, capacity: usize) -> Result<Arc<Self>> {
        let audio = &config.capabilities.audio_codecs;
        let supported = audio.len() == 1
            && audio[0].channels == 1
            && audio[0].fmtp.as_ref().is_none_or(|p| p.is_empty())
            && match audio[0].name.as_str() {
                "g.711-mu" | "pcmu" | "g.711-a" | "pcma" => audio[0].clock_rate_hz == 8000,
                "opus" => {
                    audio[0].clock_rate_hz == 48000
                        && !config.opus_settings.stereo
                        && (1..=20).contains(&config.opus_settings.min_ptime_ms)
                        && (config.opus_settings.max_average_bitrate_bps == 0
                            || (32000..=510000)
                                .contains(&config.opus_settings.max_average_bitrate_bps))
                }
                _ => false,
            };
        let video = &config.capabilities.video_codecs;
        let supported_video = !config.require_vp8_audio_video_profile
            || (video.len() == 1
                && video[0].name.eq_ignore_ascii_case("vp8")
                && video[0].clock_rate_hz == 90000
                && video[0].channels == 0
                && video[0].fmtp.as_ref().is_none_or(|p| p.is_empty()));
        if capacity == 0 || capacity > 512 || !supported || config.trickle_ice || !supported_video {
            return Err(Error::new(
                "unsupported-media-profile",
                "Canonical WebRTC requires supported mono audio and an optional explicit VP8 camera profile",
            ));
        }
        Ok(Arc::new(Self {
            config,
            adapter: rvoip_webrtc::canonical::CanonicalWebRtcAdapter::new(),
            slots: Arc::new(tokio::sync::Semaphore::new(capacity)),
        }))
    }
    pub fn available_slots(&self) -> usize {
        self.slots.available_permits()
    }
}
struct Owned(Peer, bool);
impl Host for SignalingHost {
    fn adapter(&self) -> Arc<rvoip_webrtc::canonical::CanonicalWebRtcAdapter> {
        self.adapter.clone()
    }
    fn reserve_physical(
        &self,
        peer: &Arc<rvoip_core::peer_admission::PeerAdmission>,
        physical: CancellationToken,
    ) -> Result<Arc<WebRtcPhysicalBinding>> {
        let permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::new("busy", "Canonical WebRTC peers at capacity"))?;
        Ok(WebRtcPhysicalBinding::with_capacity(peer, physical, permit))
    }
    fn start(&self, p: Preparation) -> Arc<dyn Attempt> {
        Arc::new(Owned(
            Peer::start_scoped(
                p.store,
                p.origin,
                p.attempt,
                self.config.clone(),
                &p.physical,
                p.binding,
                p.deadline,
                ReadContext::new(Some(p.read)),
                #[cfg(test)]
                None,
            ),
            self.config.require_vp8_audio_video_profile,
        ))
    }
}
#[async_trait]
impl Attempt for Owned {
    fn original_attempt(&self) -> &NativeOfferLease {
        self.0.original_attempt()
    }
    fn release_invocation(&self) {
        self.0.release_invocation();
    }
    async fn answer(&self, deadline: Instant) -> Result<ConnectionAnswer> {
        let peer = &self.0;
        let read = peer.source_read(deadline).await?;
        let offer = &read.target().offer;
        validate_offer(offer, self.1)?;
        let answer = peer.answer_sdp(deadline).await?;
        selected_answer(peer.origin.participant_id(), offer, &answer, self.1)
    }
    async fn shutdown(&self) -> Result<()> {
        self.0.shutdown().await
    }
    async fn closed(&self) -> Result<()> {
        self.0.completion.clone().await
    }
}

fn unsupported() -> Error {
    Error::new("unsupported-media-profile", "Canonical WebRTC logical streams and negotiated SDP must match the configured duplex audio/VP8 profile")
}

/// Validate the original logical/SDP shape before observing an engine answer.
/// SDP alone cannot opt an audio-only host into AV or admit extra Core media.
fn validate_offer(offer: &serde_json::Value, av: bool) -> Result<()> {
    let streams = offer["streams_offered"]
        .as_array()
        .ok_or_else(unsupported)?;
    if streams.len() != if av { 2 } else { 1 } {
        return Err(unsupported());
    }
    let mut kinds = std::collections::BTreeSet::new();
    let mut ids = std::collections::BTreeSet::new();
    for stream in streams {
        let kind = stream["kind"].as_str().ok_or_else(unsupported)?;
        let id = stream["id"].as_str().ok_or_else(unsupported)?;
        if id.is_empty()
            || !ids.insert(id)
            || !kinds.insert(kind)
            || (kind != "audio" && !(av && kind == "video"))
            || stream["direction"] != "sendrecv"
        {
            return Err(unsupported());
        }
    }
    let sdp = offer["substrate_setup"]["sdp"]
        .as_str()
        .ok_or_else(unsupported)?;
    let parsed =
        rvoip_sip_core::types::sdp::SdpSession::from_str(sdp).map_err(|_| unsupported())?;
    let mut actual = std::collections::BTreeSet::new();
    let mut mids = std::collections::BTreeSet::new();
    for m in parsed.media_descriptions.iter().filter(|m| m.port != 0) {
        if !actual.insert(m.media.as_str())
            || !kinds.contains(m.media.as_str())
            || direction(m, &parsed) != "sendrecv"
        {
            return Err(unsupported());
        }
        if av && !mids.insert(media_mid(m)?.ok_or_else(unsupported)?) {
            return Err(unsupported());
        }
    }
    if parsed.media_descriptions.len() > 32 || actual != kinds {
        return Err(unsupported());
    }
    Ok(())
}
fn media_mid(media: &rvoip_sip_core::types::sdp::MediaDescription) -> Result<Option<&str>> {
    use rvoip_sip_core::types::sdp::ParsedAttribute;
    let mut values = media.generic_attributes.iter().filter_map(|a| match a {
        ParsedAttribute::Mid(v) => Some(v.as_str()),
        _ => None,
    });
    let mid = values.next();
    if values.next().is_some()
        || mid.is_some_and(|v| {
            v.is_empty() || v.len() > 16 || !v.bytes().all(|b| b.is_ascii_graphic())
        })
    {
        return Err(unsupported());
    }
    Ok(mid)
}
fn direction(
    media: &rvoip_sip_core::types::sdp::MediaDescription,
    session: &rvoip_sip_core::types::sdp::SdpSession,
) -> String {
    media
        .direction
        .as_ref()
        .or(session.direction.as_ref())
        .map(ToString::to_string)
        .unwrap_or_else(|| "sendrecv".into())
}

/// The response carries original logical IDs and the exact actual negotiated PT,
/// codec and local SDP. It grants no media readiness or implicit subscription.
fn selected_answer(
    participant: &str,
    offer: &serde_json::Value,
    answer: &str,
    av: bool,
) -> Result<ConnectionAnswer> {
    use rvoip_webrtc::media::{core_audio_profile, core_video_profile};
    validate_offer(offer, av)?;
    let original = offer["substrate_setup"]["sdp"]
        .as_str()
        .ok_or_else(unsupported)?;
    let o =
        rvoip_sip_core::types::sdp::SdpSession::from_str(original).map_err(|_| unsupported())?;
    let a = rvoip_sip_core::types::sdp::SdpSession::from_str(answer).map_err(|_| unsupported())?;
    if o.media_descriptions.len() != a.media_descriptions.len() {
        return Err(unsupported());
    }
    for (original, selected) in o.media_descriptions.iter().zip(&a.media_descriptions) {
        if original.media != selected.media
            || original.protocol != selected.protocol
            || (original.port == 0) != (selected.port == 0)
            || media_mid(original)? != media_mid(selected)?
            || (selected.port != 0 && direction(selected, &a) != "sendrecv")
        {
            return Err(unsupported());
        }
    }
    let (audio, codec) =
        core_audio_profile::negotiated_profile(original, answer).map_err(|_| unsupported())?;
    let video = av
        .then(|| core_video_profile::negotiated_profile(original, answer))
        .transpose()
        .map_err(|_| unsupported())?;
    let mut selected = Vec::new();
    for stream in offer["streams_offered"]
        .as_array()
        .ok_or_else(unsupported)?
    {
        let codec = match stream["kind"].as_str() {
            Some("audio") => {
                json!({"name":codec.name,"params":{"sample_rate":codec.clock_rate_hz,"channels":codec.channels,"payload_type":audio.payload_type}})
            }
            Some("video") => {
                json!({"name":"vp8","params":{"sample_rate":90000,"channels":0,"payload_type":video.as_ref().ok_or_else(unsupported)?.payload_type}})
            }
            _ => return Err(unsupported()),
        };
        selected.push(
            json!({"id":stream["id"],"kind":stream["kind"],"direction":"sendrecv","codec":codec}),
        );
    }
    let mut capabilities = json!({"audio_codecs":[{"name":codec.name,"params":{"sample_rate":codec.clock_rate_hz,"channels":codec.channels}}],"max_streams_per_connection":selected.len()});
    if av {
        capabilities["video_codecs"] =
            json!([{"name":"vp8","params":{"sample_rate":90000,"channels":0}}]);
    }
    serde_json::from_value(
        json!({"by_participant":participant,"substrate":"websocket+webrtc",
        "capabilities":capabilities,"streams_answered":selected,
        "substrate_setup":{"kind":"websocket+webrtc","sdp":answer}}),
    )
    .map_err(|_| engine_error())
}
