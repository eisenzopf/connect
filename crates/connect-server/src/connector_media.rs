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

/// Protected composition binds media and cleanup to the very same typed object.
/// Intentionally neither serializable nor Debug: configuration contains refs.
pub struct LiveTransport {
    manifest: Manifest,
    adapter: std::sync::Arc<dyn ConnectionAdapter>,
    lifecycle: std::sync::Arc<dyn connect_connector_api::media::LiveMediaLifecycle>,
}
impl LiveTransport {
    pub fn new<A>(
        manifest: Manifest,
        configuration: std::collections::BTreeMap<String, String>,
        adapter: std::sync::Arc<A>,
    ) -> Result<Self, ContractError>
    where
        A: ConnectionAdapter + connect_connector_api::media::LiveMediaLifecycle + 'static,
    {
        // Own cancellation even if declaration/configuration validation refuses.
        let owned = Self {
            manifest,
            adapter: adapter.clone(),
            lifecycle: adapter,
        };
        owned.manifest.validate_configuration(&configuration)?;
        validate_adapter(&owned.manifest, owned.adapter.as_ref())?;
        let flags = owned.adapter.lifecycle_capabilities();
        if !flags.supports_fail_closed_inbound() || !flags.staged_outbound_activation {
            return Err(ContractError::MediaRuntime);
        }
        Ok(owned)
    }
}
impl Drop for LiveTransport {
    fn drop(&mut self) {
        self.lifecycle.cancel();
    }
}

#[derive(Default)]
pub(crate) struct Host(Vec<LiveTransport>);
impl Host {
    pub fn new(transports: Vec<LiveTransport>) -> Self {
        Self(transports)
    }
    fn validate(&self) -> anyhow::Result<()> {
        let owned = self;
        anyhow::ensure!(owned.0.len() <= 16, "Too many contributed media transports");
        let mut names = BTreeSet::new();
        let mut bindings = BTreeSet::new();
        for transport in &owned.0 {
            anyhow::ensure!(
                names.insert(&transport.manifest.name)
                    && bindings.insert(serde_json::to_string(&transport.adapter.transport())?),
                "Duplicate contributed media registration"
            );
        }
        Ok(())
    }
    pub fn install(&self, service: &connect_service::Service) -> anyhow::Result<()> {
        self.validate()?;
        for transport in &self.0 {
            validate_adapter(&transport.manifest, transport.adapter.as_ref())?;
            // Immutable metadata is not a successful installation/readiness fact.
            service
                .store
                .register_extension_manifest(&transport.manifest)?;
            service.install_canonical_transport(transport.adapter.clone())?;
        }
        Ok(())
    }
    pub fn cancel(&self) {
        for transport in &self.0 {
            transport.lifecycle.cancel();
        }
    }
    pub async fn drain(&self) -> anyhow::Result<()> {
        self.cancel();
        let outcomes = futures_util::future::join_all(self.0.iter().map(|transport| async move {
            tokio::time::timeout(
                std::time::Duration::from_millis(u64::from(transport.manifest.limits.teardown_ms)),
                transport.lifecycle.drain(),
            )
            .await
        }))
        .await;
        anyhow::ensure!(
            outcomes.iter().all(|result| matches!(result, Ok(Ok(())))),
            "Contributed media cleanup is unconfirmed"
        );
        Ok(())
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        self.cancel();
    }
}
