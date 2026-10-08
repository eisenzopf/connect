use crate::{config::ServerConfig, tls};
use anyhow::{Context, Result};
use connect_service::{Service, StoreValidator};
use connect_store::Store;
use rvoip_quic::{UctpQuicAdapter, UctpQuicConfig};
use std::{sync::Arc, time::Duration};

pub struct RunningServer {
    _installation: crate::installation::InstallationLock,
    endpoint: Arc<quinn::Endpoint>,
    adapter: Arc<UctpQuicAdapter>,
    pub address: std::net::SocketAddr,
    pub websocket_address: Option<std::net::SocketAddr>,
    websocket: Option<crate::transports::websocket::Listener>,
    pub store: Arc<Store>,
}
impl RunningServer {
    /// Safe aggregate diagnostic: queue occupancy only, no message contents.
    pub fn websocket_output_high_water(&self) -> usize {
        self.websocket.as_ref().map_or(0, |w| w.output_high_water())
    }
    pub async fn start(config: ServerConfig) -> Result<Self> {
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
        let service = Service::new(store.clone());
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
        let mut cfg =
            UctpQuicConfig::new(endpoint.clone(), rx, validator).with_application_handler(service);
        cfg.max_concurrent_connections = config.limits.connections;
        cfg.coordinator_caps.signaling_send_timeout =
            Duration::from_millis(config.limits.request_timeout_ms);
        let adapter = UctpQuicAdapter::new(cfg).await?;
        Ok(Self {
            _installation: installation,
            endpoint,
            adapter,
            address,
            websocket_address,
            websocket,
            store,
        })
    }
    pub async fn shutdown(mut self) -> Result<()> {
        self.adapter.begin_drain();
        self.endpoint.close(0u32.into(), b"server shutdown");
        if let Some(websocket) = self.websocket.take() {
            websocket.shutdown().await?;
        }
        tokio::time::timeout(Duration::from_secs(5), self.endpoint.wait_idle())
            .await
            .context("QUIC shutdown exceeded 5 seconds")?;
        Ok(())
    }
}
impl Drop for RunningServer {
    fn drop(&mut self) {
        self.adapter.begin_drain();
        self.endpoint.close(0u32.into(), b"server dropped");
    }
}
