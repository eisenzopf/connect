//! Configured outbound telephone replacement on the canonical Core.
use anyhow::{Context, Result};
use connect_service::{InitialTelephoneRoute, Service, TelephoneReplacementRoute};
use connect_store::{InitialTelephonePolicy, Store, TelephoneReplacementPolicy};
use rvoip_sip::{
    api::unified::Config as SipConfig, SipAdapter, SipOriginateContext, UnifiedCoordinator,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    net::SocketAddr,
    sync::{Arc, Weak},
    time::Duration,
};

/// Local canonical SIP listener and bounded administrator-selected routes.
/// Destinations come from verified Store Endpoints, never this listener's callers.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalTelephoneConfig {
    pub bind: SocketAddr,
    pub media_port_start: u16,
    pub media_port_end: u16,
    #[serde(default)]
    pub outbound: Option<crate::config::SipOutboundConfig>,
    #[serde(default)]
    pub routes: Vec<CanonicalTelephoneRoute>,
    #[serde(default)]
    pub initial: Vec<InitialTelephonePolicy>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalTelephoneRoute {
    pub credential_id: String,
    pub policy: TelephoneReplacementPolicy,
}
impl CanonicalTelephoneConfig {
    pub(crate) fn validate(&self, capacity: usize) -> Result<()> {
        anyhow::ensure!(
            self.bind.ip().is_loopback() && self.bind.port() != 0,
            "Canonical telephone listener requires a concrete loopback address"
        );
        anyhow::ensure!(
            self.media_port_start >= 1024 && self.media_port_start <= self.media_port_end,
            "Invalid canonical telephone media port range"
        );
        anyhow::ensure!(
            !self.routes.is_empty() || !self.initial.is_empty(),
            "Invalid canonical telephone route capacity"
        );
        anyhow::ensure!(
            self.routes
                .len()
                .checked_add(self.initial.len())
                .is_some_and(|n| n <= capacity),
            "Canonical telephone profile capacity exceeded"
        );
        let mut targets = BTreeSet::new();
        for policy in &self.initial {
            policy
                .validate()
                .map_err(|_| anyhow::anyhow!("Invalid initial telephone policy"))?;
            anyhow::ensure!(
                targets.insert(&policy.endpoint_id),
                "Duplicate initial telephone Endpoint"
            );
        }
        let mut references = BTreeSet::new();
        for route in &self.routes {
            rvoip_uctp::v2::Identifier::try_from(route.credential_id.clone())
                .map_err(|_| anyhow::anyhow!("Invalid canonical telephone credential reference"))?;
            route
                .policy
                .validate()
                .map_err(|_| anyhow::anyhow!("Invalid canonical telephone route policy"))?;
            anyhow::ensure!(
                references.insert(&route.policy.route_reference),
                "Duplicate canonical telephone route reference"
            );
        }
        if let Some(outbound) = &self.outbound {
            outbound.validate()?;
        }
        Ok(())
    }
}

/// Also guards startup failures and dropping the server without explicit shutdown.
/// Cleanup needs no new dial authority and never publishes a replacement result.
pub(crate) struct TelephoneTransport {
    service: Weak<Service>,
    coordinator: Option<Arc<UnifiedCoordinator>>,
    registration: Option<crate::registration_monitor::Monitor>,
}
impl TelephoneTransport {
    pub(crate) async fn start(
        config: &CanonicalTelephoneConfig,
        store: &Arc<Store>,
        service: &Arc<Service>,
    ) -> Result<Self> {
        config.validate(store.limits().connections)?;
        // Reject stale/missing accounts before binding any listener. The service
        // independently repeats these checks at installation and every operation.
        for route in &config.routes {
            store
                .revalidate(&route.credential_id)?
                .require("sip_dial")?;
        }
        for policy in &config.initial {
            store
                .revalidate(&policy.endpoint_credential_id)?
                .require("connection.offer")?;
            store
                .revalidate(&policy.endpoint_credential_id)?
                .require("session.accept")?;
            store
                .revalidate(&policy.route_credential_id)?
                .require("sip_dial")?;
        }
        let mut listener = SipConfig::local("connect-canonical", config.bind.port());
        listener.bind_addr = config.bind;
        listener.local_ip = config.bind.ip();
        listener.local_uri = format!("sip:connect@{}", config.bind);
        listener.media_port_start = config.media_port_start;
        listener.media_port_end = config.media_port_end;
        if let Some(outbound) = &config.outbound {
            crate::media_transports::apply_outbound_trunk(&mut listener, outbound, |name| {
                crate::secrets::resolve(name).map(|value| value.expose().to_owned())
            })?;
        }
        let coordinator = UnifiedCoordinator::new(listener)
            .await
            .context("Cannot bind canonical telephone listener")?;
        let mut transport = Self {
            service: Arc::downgrade(service),
            coordinator: Some(coordinator.clone()),
            registration: None,
        };
        if let Some(outbound) = &config.outbound {
            if let Some(handle) = crate::sip_registration::register(&coordinator, outbound).await? {
                transport.registration = Some(crate::registration_monitor::Monitor::start(
                    coordinator.clone(),
                    handle,
                    service.readiness.clone(),
                ));
            }
        }
        let adapter = SipAdapter::new_with_inbound_context_policy_and_queue_capacity(
            coordinator,
            Default::default(),
            connect_protocol::media_profile::AUDIO_QUEUE_CAPACITY,
        )
        .await
        .context("Cannot create canonical telephone adapter")?;
        if !config.routes.is_empty() {
            service
                .install_telephone_replacement_adapter(
                    adapter.clone(),
                    config
                        .routes
                        .iter()
                        .map(|route| TelephoneReplacementRoute {
                            credential_id: route.credential_id.clone(),
                            policy: route.policy.clone(),
                            originate: SipOriginateContext::default(),
                        })
                        .collect(),
                )
                .context("Cannot install canonical telephone routes")?;
        }
        if !config.initial.is_empty() {
            service
                .install_initial_telephone_host(
                    adapter,
                    config
                        .initial
                        .iter()
                        .map(|policy| InitialTelephoneRoute {
                            policy: policy.clone(),
                            originate: SipOriginateContext::default(),
                        })
                        .collect(),
                )
                .context("Cannot install initial telephone profiles")?;
        }
        Ok(transport)
    }
    pub(crate) async fn shutdown(&mut self) -> Result<()> {
        let registration = match self.registration.take() {
            Some(monitor) => monitor.shutdown().await,
            None => Ok(()),
        };
        let signalling = match self.coordinator.take() {
            Some(coordinator) => coordinator
                .shutdown_gracefully(Some(Duration::from_secs(5)))
                .await
                .map_err(|_| anyhow::anyhow!("Canonical telephone shutdown failed")),
            None => Ok(()),
        };
        registration?;
        signalling
    }
}
impl Drop for TelephoneTransport {
    fn drop(&mut self) {
        self.registration.take();
        if let Some(coordinator) = self.coordinator.take() {
            let service = self.service.upgrade();
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    if let Some(service) = service {
                        service.drain_canonical_runtime().await;
                    }
                    let _ = coordinator
                        .shutdown_gracefully(Some(Duration::from_secs(5)))
                        .await;
                });
            }
        }
    }
}
