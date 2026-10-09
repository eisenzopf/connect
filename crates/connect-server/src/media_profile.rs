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
