//! Configured outbound telephone replacement on the canonical Core.
use anyhow::{Context, Result};
use connect_protocol::native_network::NativeNetworkProfile;
use connect_service::{InitialTelephoneRoute, Service, TelephoneReplacementRoute};
use connect_store::{InitialTelephonePolicy, Store, TelephoneReplacementPolicy};
use rvoip_sip::{
    api::unified::Config as SipConfig, SipAdapter, SipOriginateContext, UnifiedCoordinator,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Weak},
    time::Duration,
};

/// Explicit canonical SIP listener and bounded administrator-selected routes.
/// Destinations come from verified Store Endpoints, never this listener's callers.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalTelephoneConfig {
    pub bind: SocketAddr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_bind_ip: Option<IpAddr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advertised: Option<SocketAddr>,
    /// Port-preserving one-to-one RTP NAT; each call keeps its allocated port.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_public_ip: Option<IpAddr>,
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
    pub(crate) fn validate(&self, network: NativeNetworkProfile, capacity: usize) -> Result<()> {
        let bind = NativeNetworkProfile::canonical_address(self.bind);
        anyhow::ensure!(
            network.permits_listener(bind) && bind.port() != 0,
            "Canonical telephone bind requires its explicit network profile and port"
        );
        let media = NativeNetworkProfile::canonical_address(SocketAddr::new(
            self.media_bind_ip.unwrap_or(bind.ip()),
            1,
        ))
        .ip();
        anyhow::ensure!(
            network.permits_endpoint(SocketAddr::new(media, 1))
                && media.is_ipv4() == bind.is_ipv4(),
            "SIP media requires a concrete same-family interface within its network profile"
        );
        anyhow::ensure!(
            !bind.ip().is_unspecified() || self.advertised.is_some(),
            "Wildcard SIP bind requires an explicit advertised signaling address"
        );
        if let Some(advertised) = self.advertised.map(NativeNetworkProfile::canonical_address) {
            anyhow::ensure!(
                network.permits_endpoint(advertised)
                    && advertised.port() != 0
                    && advertised.is_ipv4() == bind.is_ipv4(),
                "Invalid SIP advertised address, port or family"
            );
        }
        if let Some(public) = self.media_public_ip {
            let public = NativeNetworkProfile::canonical_address(SocketAddr::new(public, 1));
            anyhow::ensure!(
                network.permits_endpoint(public) && public.is_ipv4() == media.is_ipv4(),
                "Invalid SIP public RTP address or family"
            );
        }
        anyhow::ensure!(
            self.media_port_start >= 1024
                && self.media_port_start <= self.media_port_end
                && u32::from(self.media_port_end) - u32::from(self.media_port_start) < 4096,
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

    fn listener(&self) -> SipConfig {
        let bind = NativeNetworkProfile::canonical_address(self.bind);
        let mut listener = SipConfig::local("connect-canonical", bind.port());
        listener.bind_addr = bind;
        listener.sip_port = bind.port();
        listener.local_ip = NativeNetworkProfile::canonical_address(SocketAddr::new(
            self.media_bind_ip.unwrap_or(bind.ip()),
            1,
        ))
        .ip();
        listener.sip_advertised_addr = self.advertised.map(NativeNetworkProfile::canonical_address);
        listener.local_uri = format!(
            "sip:connect@{}",
            listener.sip_advertised_addr.unwrap_or(bind)
        );
        listener.media_port_start = self.media_port_start;
        listener.media_port_end = self.media_port_end;
        if let Some(ip) = self.media_public_ip {
            let public = NativeNetworkProfile::canonical_address(SocketAddr::new(ip, 0));
            listener = listener.with_media_public_addr(public);
        }
        listener
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
        network: NativeNetworkProfile,
        store: &Arc<Store>,
        service: &Arc<Service>,
    ) -> Result<Self> {
        config.validate(network, store.limits().connections)?;
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
        let mut listener = config.listener();
        // Refuse an unavailable local media interface before registration or
        // any future authorized dial. This is configuration admission, not
        // evidence that RTP can reach the remote endpoint.
        let media_probe = tokio::net::UdpSocket::bind(SocketAddr::new(listener.local_ip, 0))
            .await
            .context("Cannot bind configured SIP media interface")?;
        drop(media_probe);
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
        let manifest = connect_connectors::provider_mapping::sip_live_manifest();
        crate::connector_media::validate_adapter(&manifest, adapter.as_ref())
            .context("Canonical telephone adapter does not satisfy its connector contract")?;
        store.register_extension_manifest(&manifest)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> CanonicalTelephoneConfig {
        CanonicalTelephoneConfig {
            bind: "127.0.0.1:5060".parse().unwrap(),
            media_bind_ip: None,
            advertised: None,
            media_public_ip: None,
            media_port_start: 20000,
            media_port_end: 20063,
            outbound: None,
            initial: Vec::new(),
            routes: vec![CanonicalTelephoneRoute {
                credential_id: "credential_fixture".into(),
                policy: TelephoneReplacementPolicy {
                    route_reference: "route_fixture".into(),
                    sip_authority: "127.0.0.1:5099".into(),
                    confirmation_digits: "55#".into(),
                    timeout_seconds: 30,
                },
            }],
        }
    }
    #[test]
    fn remote_signaling_and_media_mapping_preserves_each_allocated_rtp_port() {
        let mut config = config();
        config.bind = "0.0.0.0:5060".parse().unwrap();
        config.media_bind_ip = Some("10.0.0.2".parse().unwrap());
        config.advertised = Some("203.0.113.44:15060".parse().unwrap());
        config.media_public_ip = Some("203.0.113.45".parse().unwrap());
        config.validate(NativeNetworkProfile::Remote, 4).unwrap();
        assert!(config.validate(NativeNetworkProfile::Local, 4).is_err());
        let listener = config.listener();
        listener.validate().unwrap();
        assert_eq!(listener.bind_addr, config.bind);
        assert_eq!(listener.sip_port, 5060);
        assert_eq!(listener.local_ip, config.media_bind_ip.unwrap());
        assert_eq!(listener.sip_advertised_addr, config.advertised);
        assert_eq!(listener.local_uri, "sip:connect@203.0.113.44:15060");
        assert_eq!(
            listener.media_public_addr,
            Some("203.0.113.45:0".parse().unwrap())
        );
        assert_eq!(
            (listener.media_port_start, listener.media_port_end),
            (20000, 20063)
        );
        let loaded: CanonicalTelephoneConfig =
            toml::from_str(&toml::to_string(&config).unwrap()).unwrap();
        loaded.validate(NativeNetworkProfile::Remote, 4).unwrap();
    }
    #[test]
    fn local_defaults_remain_concrete_loopback_without_public_contacts() {
        let config = config();
        config.validate(NativeNetworkProfile::Local, 4).unwrap();
        let listener = config.listener();
        listener.validate().unwrap();
        assert!(listener.local_ip.is_loopback());
        assert!(listener.bind_addr.ip().is_loopback());
        assert_eq!(listener.local_uri, "sip:connect@127.0.0.1:5060");
        assert!(listener.sip_advertised_addr.is_none());
        assert!(listener.media_public_addr.is_none());
    }
    #[test]
    fn ambiguous_wildcard_contacts_families_and_ranges_refuse() {
        let base = config();
        let mut cases = Vec::new();
        let mut value = base.clone();
        value.bind = "0.0.0.0:5060".parse().unwrap();
        cases.push(value);
        let mut value = base.clone();
        value.bind = "0.0.0.0:5060".parse().unwrap();
        value.media_bind_ip = Some("10.0.0.2".parse().unwrap());
        cases.push(value);
        let mut value = base.clone();
        value.advertised = Some("0.0.0.0:5060".parse().unwrap());
        cases.push(value);
        let mut value = base.clone();
        value.advertised = Some("203.0.113.44:0".parse().unwrap());
        cases.push(value);
        let mut value = base.clone();
        value.advertised = Some("[2001:db8::44]:5060".parse().unwrap());
        cases.push(value);
        let mut value = base.clone();
        value.media_bind_ip = Some("0.0.0.0".parse().unwrap());
        cases.push(value);
        let mut value = base.clone();
        value.media_bind_ip = Some("::1".parse().unwrap());
        cases.push(value);
        let mut value = base.clone();
        value.media_public_ip = Some("224.0.0.1".parse().unwrap());
        cases.push(value);
        let mut value = base.clone();
        value.media_port_end = 10000;
        cases.push(value);
        let mut value = base.clone();
        value.media_port_end = 30000;
        cases.push(value);
        for value in cases {
            assert!(value.validate(NativeNetworkProfile::Remote, 4).is_err());
        }
    }
}
