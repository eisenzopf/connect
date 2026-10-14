//! Explicit local audio buffering for the negotiated Gate 3 profile.

use connect_protocol::media_profile::AUDIO_QUEUE_CAPACITY;
use rvoip_core::Config;
use rvoip_vapi::{VapiApiKey, VapiConfig};

/// Configure orchestration and native QUIC queues with the shared audio budget.
pub fn orchestrator_config() -> Config {
    Config {
        audio_queue_capacity: Some(AUDIO_QUEUE_CAPACITY),
        ..Default::default()
    }
}

/// Gate 5 native negotiation explicitly opts into the one VP8 camera profile.
pub fn native_capabilities() -> rvoip_core::capability::CapabilityDescriptor {
    let mut descriptor = rvoip_uctp::state::default_v0_descriptor();
    descriptor.video_codecs = vec![rvoip_core::capability::CodecInfo {
        name: "vp8".into(),
        clock_rate_hz: connect_protocol::video_profile::VIDEO_CLOCK_HZ,
        channels: 0,
        fmtp: None,
        payload_type: Some(96),
    }];
    descriptor
}

/// Configure the real provider adapter; local fixtures use this same budget.
/// No provider operation is performed by constructing this configuration.
pub fn vapi_config(key: VapiApiKey) -> VapiConfig {
    let mut config = VapiConfig::new(key);
    let frames = AUDIO_QUEUE_CAPACITY.get();
    config.media_queue_capacity = frames;
    config.startup_audio_frames = frames;
    config.inbound_queue_capacity = frames;
    config.outbound_queue_capacity = frames;
    config.jitter_target_ceiling_ms = 200;
    config
}

/// Configure the browser audio frame channels with the negotiated queue budget.
pub fn webrtc_config() -> rvoip_webrtc::WebRtcConfig {
    let mut config = rvoip_webrtc::WebRtcConfig {
        audio_queue_capacity: Some(AUDIO_QUEUE_CAPACITY),
        ..rvoip_webrtc::WebRtcConfig::loopback()
    };
    config.capabilities.video_codecs = vec![rvoip_core::capability::CodecInfo {
        name: "vp8".into(),
        clock_rate_hz: connect_protocol::video_profile::VIDEO_CLOCK_HZ,
        channels: 0,
        fmtp: None,
        payload_type: None,
    }];
    config
}
