use crate::{config::ServerConfig, tls};
use anyhow::{Context, Result};
use connect_service::{Service, StoreSessionBindingResolver, StoreValidator};
use connect_store::Store;
use rvoip_core::{adapter::ConnectionAdapter, Orchestrator};
use rvoip_quic::{UctpQuicAdapter, UctpQuicConfig};
use rvoip_vapi::{VapiAdapter, VapiApiKey};
use std::{sync::Arc, time::Duration};

/// Owned server runtime with bounded shutdown for every listener and worker.
pub struct RunningServer {
    _installation: crate::installation::InstallationLock,
    endpoint: Arc<quinn::Endpoint>,
    adapter: Arc<UctpQuicAdapter>,
    /// Actual native UCTP/QUIC listener address.
    pub address: std::net::SocketAddr,
    /// Actual optional WebSocket control-parity listener address.
    pub websocket_address: Option<std::net::SocketAddr>,
    websocket: Option<crate::transports::websocket::Listener>,
    /// Shared durable store.
    pub store: Arc<Store>,
    /// Shared authenticated application service.
    pub service: Arc<Service>,
    /// Actual optional loopback HTTP API address.
    pub http_address: Option<std::net::SocketAddr>,
    http: Option<crate::transports::http::Listener>,
    /// Actual optional Telnyx webhook-only listener address.
    pub webhook_address: Option<std::net::SocketAddr>,
    webhook: Option<crate::transports::http::Listener>,
    providers: Option<crate::providers::Worker>,
    voice: Option<crate::voice::Worker>,
    orchestrator: Arc<Orchestrator>,
    media_lifecycle: Option<crate::media_lifecycle::MediaLifecycle>,
    sip_outbound: Option<crate::sip_outbound::Worker>,
    media_transports: crate::media_transports::MediaTransports,
    /// Actual optional SIP listener address.
    pub sip_address: Option<std::net::SocketAddr>,
    /// Actual optional authenticated WebRTC signaling address.
    pub webrtc_address: Option<std::net::SocketAddr>,
}
impl RunningServer {
    /// Safe aggregate diagnostic: queue occupancy only, no message contents.
    pub fn websocket_output_high_water(&self) -> usize {
        self.websocket.as_ref().map_or(0, |w| w.output_high_water())
    }
    /// Start the native listener without provider workers or optional facades.
    pub async fn start(config: ServerConfig) -> Result<Self> {
        Self::start_local(config, None, false).await
    }
    /// Additional local interfaces are opt-in. No public listener is opened.
    /// Start native and optional HTTP interfaces with explicit provider-worker policy.
    pub async fn start_local(
        config: ServerConfig,
        http_bind: Option<std::net::SocketAddr>,
        providers: bool,
    ) -> Result<Self> {
        Self::start_interfaces(config, http_bind, None, providers).await
    }
    /// Start native, HTTP, and webhook interfaces under one owned runtime.
    pub async fn start_interfaces(
        config: ServerConfig,
        http_bind: Option<std::net::SocketAddr>,
        webhook_bind: Option<std::net::SocketAddr>,
        providers: bool,
    ) -> Result<Self> {
        Self::start_interfaces_inner(config, http_bind, webhook_bind, providers, None).await
    }

    /// Dependency-injected Vapi media path for the bounded local provider
    /// harness. Provider polling remains disabled, so this cannot affect an
    /// external account.
    #[doc(hidden)]
    pub async fn start_with_vapi_media(
        config: ServerConfig,
        http_bind: Option<std::net::SocketAddr>,
        vapi_media: Arc<VapiAdapter>,
    ) -> Result<Self> {
        Self::start_interfaces_inner(config, http_bind, None, false, Some(vapi_media)).await
    }

    async fn start_interfaces_inner(
        config: ServerConfig,
        http_bind: Option<std::net::SocketAddr>,
        webhook_bind: Option<std::net::SocketAddr>,
        providers: bool,
        injected_vapi_media: Option<Arc<VapiAdapter>>,
    ) -> Result<Self> {
        config.validate()?;
        let installation = crate::installation::InstallationLock::acquire(&config.database)?;
        let store = Arc::new(Store::open(&config.database, config.limits.clone())?);
        store.integrity_check()?;
        let mut transport = quinn::TransportConfig::default();
        transport.max_idle_timeout(Some(Duration::from_secs(30).try_into()?));
        let endpoint = Arc::new(
            rvoip_uctp::substrate::make_server_endpoint(
                config.bind,
                tls::server_config(&config.certificate, &config.private_key)?,
                transport,
            )
            .context("Cannot bind QUIC listener; choose an unused local port")?,
        );
        let address = endpoint.local_addr()?;
        let mut routes = rvoip_uctp::substrate::dispatch_by_alpn(
            endpoint.clone(),
            &[rvoip_uctp::UCTP_RAW_QUIC_ALPN_BYTES],
        )?;
        let rx = routes
            .take(rvoip_uctp::UCTP_RAW_QUIC_ALPN_BYTES)
            .context("Missing UCTP ALPN route")?;
        let mut discovery = connect_store::DiscoveryRuntime {
            native_audio: true,
            sip: config
                .sip
                .as_ref()
                .is_some_and(|sip| sip.outbound.is_some()),
            providers: if providers {
                [("telnyx", "TELNYX_API_KEY"), ("vapi", "VAPI_PRIVATE_KEY")]
                    .into_iter()
                    .filter(|(_, key)| std::env::var_os(key).is_some())
                    .map(|(name, _)| name.to_owned())
                    .collect()
            } else {
                Vec::new()
            },
            vapi_audio: injected_vapi_media.is_some()
                || (providers && std::env::var_os("VAPI_PRIVATE_KEY").is_some()),
            browser: config.webrtc.as_ref().map(|w| connect_store::BrowserRoute {
                credential_id: w.route.credential_id.clone(),
                binding: connect_protocol::BrowserBinding {
                    conversation_id: w.route.conversation_id.clone(),
                    participant_id: w.route.assistant_identity_id.clone(),
                    session_id: w.route.session_id.clone(),
                    signaling_url: format!("ws://{}", w.bind),
                },
            }),
        };
        let readiness = Arc::new(connect_service::readiness::OperationalReadiness::default());
        let orchestrator = Orchestrator::new(crate::media_profile::orchestrator_config());
        let session_resolver =
            StoreSessionBindingResolver::new(store.clone(), orchestrator.clone())?;
        let vapi_media = if injected_vapi_media.is_some() {
            injected_vapi_media
        } else if providers {
            std::env::var("VAPI_PRIVATE_KEY")
                .ok()
                .map(|value| {
                    let key = VapiApiKey::new(value).map_err(anyhow::Error::from)?;
                    VapiAdapter::new(crate::media_profile::vapi_config(key))
                        .map_err(anyhow::Error::from)
                })
                .transpose()?
        } else {
            None
        };
        let media_lifecycle = crate::media_lifecycle::MediaLifecycle::start(
            orchestrator.clone(),
            store.clone(),
            readiness.clone(),
            vapi_media,
            crate::media_ingress::InboundMediaRoutes {
                sip: config.sip.as_ref().map(|value| value.route.clone()),
                webrtc: config.webrtc.as_ref().map(|value| value.route.clone()),
            },
        )?;
        let media_transports = crate::media_transports::MediaTransports::start(
            &config,
            &orchestrator,
            store.clone(),
            readiness.clone(),
        )
        .await?;
        if let (Some(browser), Some(address)) =
            (&mut discovery.browser, media_transports.webrtc_address)
        {
            browser.binding.signaling_url = format!("ws://{address}");
        }
        let service = Service::with_readiness(store.clone(), discovery, readiness.clone());
        let sip_outbound = if config.sip.is_some() {
            Some(crate::sip_outbound::Worker::start(
                store.clone(),
                readiness.clone(),
                orchestrator.clone(),
                session_resolver.clone(),
                media_lifecycle.registrar(),
            )?)
        } else {
            None
        };
        let sip_address = media_transports.sip_address;
        let webrtc_address = media_transports.webrtc_address;
        let public_key = if let Ok(key) = std::env::var("TELNYX_PUBLIC_KEY") {
            Some(key)
        } else if providers
            && (http_bind.is_some() || webhook_bind.is_some())
            && std::env::var_os("TELNYX_API_KEY").is_some()
        {
            Some(connect_connectors::setup::public_key().await?)
        } else {
            None
        };
        let http = if let Some(bind) = http_bind {
            Some(
                crate::transports::http::Listener::bind(
                    bind,
                    service.clone(),
                    public_key.as_deref(),
                )
                .await?,
            )
        } else {
            None
        };
        let http_address = http.as_ref().map(|h| h.address);
        let webhook = if let Some(bind) = webhook_bind {
            Some(
                crate::transports::http::Listener::bind_webhooks(
                    bind,
                    service.clone(),
                    public_key.as_deref(),
                )
                .await?,
            )
        } else {
            None
        };
        let webhook_address = webhook.as_ref().map(|h| h.address);
        let voice = if providers && std::env::var_os("VAPI_PRIVATE_KEY").is_some() {
            Some(crate::voice::Worker::start(service.clone())?)
        } else {
            None
        };
        let providers = if providers {
            Some(crate::providers::Worker::start(service.clone())?)
        } else {
            None
        };
        let validator = StoreValidator::new(store.clone());
        let websocket = if let Some(address) = config.websocket_bind {
            Some(
                crate::transports::websocket::Listener::bind(
                    address,
                    service.clone(),
                    validator.clone(),
                    config.limits.connections,
                    config.limits.request_timeout_ms,
                )
                .await
                .context("Cannot bind loopback WS parity listener")?,
            )
        } else {
            None
        };
        let websocket_address = websocket.as_ref().map(|w| w.address);
        let mut cfg = UctpQuicConfig::new(endpoint.clone(), rx, validator)
            .with_application_handler(service.clone())
            .with_session_binding_resolver(session_resolver)
            .with_subscription_handler(rvoip_uctp::state::OrchestratorSubscriptionHandler::new(
                orchestrator.clone(),
                orchestrator.publisher_registry(),
            ))
            .with_orchestrator(orchestrator.clone());
        cfg.max_concurrent_connections = config.limits.connections;
        cfg.coordinator_caps.signaling_send_timeout =
            Duration::from_millis(config.limits.request_timeout_ms);
        let adapter = UctpQuicAdapter::new(cfg).await?;
        orchestrator.register(adapter.clone() as Arc<dyn ConnectionAdapter>)?;
        readiness.set(
            "native-control",
            connect_protocol::ReadinessState::Ready,
            "listener-bound",
        );
        Ok(Self {
            _installation: installation,
            endpoint,
            adapter,
            address,
            websocket_address,
            websocket,
            store,
            service,
            webhook,
            webhook_address,
            http_address,
            http,
            providers,
            voice,
            orchestrator,
            media_lifecycle: Some(media_lifecycle),
            sip_outbound,
            media_transports,
            sip_address,
            webrtc_address,
        })
    }
    /// Stop intake, terminate media legs, and drain all owned resources.
    pub async fn shutdown(mut self) -> Result<()> {
        self.adapter.begin_drain();
        let mut failures = Vec::new();
        if let Some(voice) = self.voice.take() {
            if let Err(error) = voice.shutdown().await {
                failures.push(error.to_string());
            }
        }
        if let Some(webhook) = self.webhook.take() {
            if let Err(error) = webhook.shutdown().await {
                failures.push(error.to_string());
            }
        }
        if let Some(http) = self.http.take() {
            if let Err(error) = http.shutdown().await {
                failures.push(error.to_string());
            }
        }
        if let Some(providers) = self.providers.take() {
            if let Err(error) = providers.shutdown().await {
                failures.push(error.to_string());
            }
        }
        if let Some(sip_outbound) = self.sip_outbound.take() {
            if let Err(error) = sip_outbound.shutdown().await {
                failures.push(error.to_string());
            }
        }
        if let Some(media_lifecycle) = self.media_lifecycle.take() {
            if let Err(error) = media_lifecycle.shutdown().await {
                failures.push(error.to_string());
            }
        }
        self.media_transports.shutdown(&mut failures).await;
        self.orchestrator
            .drain_prepared_outbound_connections()
            .await;
        self.orchestrator.drain_connection_lifecycle_tasks().await;
        self.endpoint.close(0u32.into(), b"server shutdown");
        if let Some(websocket) = self.websocket.take() {
            if let Err(error) = websocket.shutdown().await {
                failures.push(error.to_string());
            }
        }
        if tokio::time::timeout(Duration::from_secs(5), self.endpoint.wait_idle())
            .await
            .is_err()
        {
            failures.push("QUIC shutdown exceeded 5 seconds".into());
        }
        anyhow::ensure!(failures.is_empty(), "{}", failures.join("; "));
        Ok(())
    }
}
impl Drop for RunningServer {
    fn drop(&mut self) {
        self.adapter.begin_drain();
        self.endpoint.close(0u32.into(), b"server dropped");
    }
}
