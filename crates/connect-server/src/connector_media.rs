//! Trusted composition observes the existing rvoip adapter; declarations never
//! install routes, confer a principal, or authorize a packet/control operation.
use connect_connector_api::{
    manifest::{ContractError, Manifest},
    media::{AudioCodec, LiveBinding, LiveCodecs, MediaRuntime, VideoCodec},
};
use rvoip_core::adapter::ConnectionAdapter;
use std::collections::BTreeSet;

pub fn adapter_runtime(adapter: &dyn ConnectionAdapter) -> Result<MediaRuntime, ContractError> {
    let transport = serde_json::to_value(adapter.transport())
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .ok_or(ContractError::MediaRuntime)?;
    let caps = adapter.capabilities();
    let lifecycle = adapter.lifecycle_capabilities();
    let audio = caps
        .audio_codecs
        .iter()
        .map(|c| AudioCodec {
            name: c.name.clone(),
            clock_rate_hz: c.clock_rate_hz,
            channels: c.channels,
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let video = caps
        .video_codecs
        .iter()
        .map(|c| VideoCodec {
            name: c.name.clone(),
            clock_rate_hz: c.clock_rate_hz,
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let data = caps
        .data_protocols
        .iter()
        .map(|v| {
            serde_json::to_value(v)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .ok_or(ContractError::MediaRuntime)
        })
        .collect::<Result<BTreeSet<_>, _>>()?
        .into_iter()
        .collect::<Vec<_>>();
    let codecs = if audio.is_empty() && video.is_empty() && data.is_empty() {
        LiveCodecs::Negotiated
    } else {
        LiveCodecs::Advertised { audio, video, data }
    };
    Ok(MediaRuntime {
        binding: LiveBinding::Transport { transport },
        codecs,
        authoritative_liveness: lifecycle.authoritative_liveness,
        atomic_inbound_handoff: lifecycle.atomic_inbound_handoff,
        terminal_fallback: lifecycle.terminal_fallback,
        staged_outbound_activation: lifecycle.staged_outbound_activation,
        device_replacement: adapter.supports_device_replacement_preparation(),
    })
}

/// Validation precedes registration. Core registration, endpoint provisioning,
/// current Store authority, media negotiation, and lifecycle ownership remain
/// the existing host's responsibilities. This function makes no network effect.
pub fn validate_adapter(
    manifest: &Manifest,
    adapter: &dyn ConnectionAdapter,
) -> Result<(), ContractError> {
    manifest.validate_media_runtime(&adapter_runtime(adapter)?)
}
