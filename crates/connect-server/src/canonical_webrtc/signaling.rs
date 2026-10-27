//! Protected canonical SDP host using the exact Core-reserved RTC context.
use super::*;
use async_trait::async_trait;
use connect_service::canonical_webrtc::{Attempt, Host, Preparation};
use rvoip_uctp::v2::payloads::ConnectionAnswer;
use serde_json::json;

/// Bounded engine composition for the first canonical audio signaling profile.
/// It is deliberately not installed or advertised by the default server config.
pub struct SignalingHost {
    config: WebRtcConfig,
    slots: Arc<tokio::sync::Semaphore>,
    adapter: Arc<rvoip_webrtc::canonical::CanonicalWebRtcAdapter>,
}
impl SignalingHost {
    /// A single G.711 codec preserves an exact canonical/SDP channel binding.
    /// Opus mono, video and SCTP need their Core media adapters before enabling.
    pub fn new(config: WebRtcConfig, capacity: usize) -> Result<Arc<Self>> {
        let audio = &config.capabilities.audio_codecs;
        if capacity == 0
            || capacity > 512
            || audio.len() != 1
            || !matches!(
                audio[0].name.as_str(),
                "g.711-mu" | "pcmu" | "g.711-a" | "pcma"
            )
            || audio[0].clock_rate_hz != 8000
            || audio[0].channels != 1
            || config.trickle_ice
            || config.require_vp8_audio_video_profile
        {
            return Err(Error::new(
                "unsupported-media-profile",
                "Canonical WebRTC signaling requires one exact G.711 audio profile",
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
struct Owned(Peer);
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
        Arc::new(Owned(Peer::start_scoped(
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
        )))
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
        let answer = peer.answer_sdp(deadline).await?;
        let read = peer.source_read(deadline).await?;
        let offer = &read.target().offer;
        let streams = offer["streams_offered"]
            .as_array()
            .ok_or_else(Error::forbidden)?;
        // This protected first profile cannot silently answer unsupported media.
        if streams.len() != 1 || streams[0]["kind"] != "audio" {
            return Err(Error::new(
                "unsupported-media-profile",
                "Canonical WebRTC audio signaling requires one logical audio stream",
            ));
        }
        let original = offer["substrate_setup"]["sdp"]
            .as_str()
            .ok_or_else(Error::forbidden)?;
        let negotiated = rvoip_webrtc::sdp::negotiated_single_audio_payload(original, &answer)
            .map_err(|_| {
                Error::new(
                    "incompatible-capabilities",
                    "WebRTC answer has no unique negotiated audio mapping",
                )
            })?;
        if negotiated.codec.channels != 1 || negotiated.codec.clock_rate_hz != 8000 {
            return Err(Error::new(
                "unsupported-media-profile",
                "Canonical WebRTC selected audio mapping is unsupported",
            ));
        }
        let parsed = rvoip_sip_core::types::sdp::SdpSession::from_str(&answer)
            .map_err(|_| engine_error())?;
        let media = parsed
            .media_descriptions
            .iter()
            .find(|m| m.media == "audio" && m.port != 0)
            .ok_or_else(engine_error)?;
        let direction = media
            .direction
            .as_ref()
            .or(parsed.direction.as_ref())
            .map(ToString::to_string)
            .unwrap_or_else(|| "sendrecv".into());
        let value = json!({"by_participant":peer.origin.participant_id(),"substrate":"websocket+webrtc",
            "capabilities":{"audio_codecs":[{"name":negotiated.codec.name,"params":{"sample_rate":8000,"channels":1}}],"max_streams_per_connection":1},
            "streams_answered":[{"id":streams[0]["id"],"kind":"audio","direction":direction,
                "codec":{"name":negotiated.codec.name,"params":{"sample_rate":8000,"channels":1,"payload_type":negotiated.payload_type}}}],
            "substrate_setup":{"kind":"websocket+webrtc","sdp":answer}});
        serde_json::from_value(value).map_err(|_| engine_error())
    }
    async fn shutdown(&self) -> Result<()> {
        self.0.shutdown().await
    }
    async fn closed(&self) -> Result<()> {
        self.0.completion.clone().await
    }
}
