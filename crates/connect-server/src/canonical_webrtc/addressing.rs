//! Explicit media sockets and ICE services. Serialized configuration holds only
//! TURN secret references; no public STUN service is installed implicitly.
use anyhow::{ensure, Result};
use connect_protocol::native_network::NativeNetworkProfile;
use rvoip_webrtc::{config::IceTransportPolicy, IceServerConfig, UdpPortRangeConfig, WebRtcConfig};
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, SocketAddr};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum IcePolicy {
    #[default]
    All,
    Relay,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IceService {
    pub urls: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_ref: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Addressing {
    pub bind_ip: IpAddr,
    pub port_start: u16,
    pub port_end: u16,
    /// One-to-one, port-preserving NAT. The owned UDP driver supports one IP.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_ip: Option<IpAddr>,
    #[serde(default)]
    pub ice_policy: IcePolicy,
    #[serde(default)]
    pub ice_services: Vec<IceService>,
}

fn canonical(ip: IpAddr) -> IpAddr {
    NativeNetworkProfile::canonical_address(SocketAddr::new(ip, 1)).ip()
}
fn reference(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
fn ice_url(value: &str, network: NativeNetworkProfile) -> Result<bool> {
    ensure!(value.len() <= 2048 && value.is_ascii(), "Invalid ICE URL");
    let (scheme, rest) = value
        .split_once(':')
        .ok_or_else(|| anyhow::anyhow!("Invalid ICE URL"))?;
    ensure!(
        matches!(scheme, "stun" | "stuns" | "turn" | "turns"),
        "Invalid ICE URL scheme"
    );
    let turn = matches!(scheme, "turn" | "turns");
    let (authority, query) = rest
        .split_once('?')
        .map_or((rest, None), |(a, q)| (a, Some(q)));
    ensure!(
        query.is_none() || (turn && matches!(query, Some("transport=udp" | "transport=tcp"))),
        "Invalid ICE transport query"
    );
    ensure!(
        !authority.is_empty()
            && authority
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-:[]".contains(&b)),
        "Invalid ICE authority"
    );
    let url = url::Url::parse(&format!("https://{authority}"))
        .map_err(|_| anyhow::anyhow!("Invalid ICE authority"))?;
    ensure!(url.port() != Some(0), "Invalid ICE port");
    let local = match url
        .host()
        .ok_or_else(|| anyhow::anyhow!("Missing ICE host"))?
    {
        url::Host::Domain(name) => {
            ensure!(
                name.len() <= 253
                    && name.trim_end_matches('.').split('.').all(|label| {
                        !label.is_empty()
                            && label.len() <= 63
                            && !label.starts_with('-')
                            && !label.ends_with('-')
                    }),
                "Invalid ICE hostname"
            );
            name == "localhost"
        }
        url::Host::Ipv4(ip) => {
            ensure!(
                NativeNetworkProfile::Remote.permits_endpoint(SocketAddr::new(ip.into(), 1)),
                "Invalid ICE address"
            );
            ip.is_loopback()
        }
        url::Host::Ipv6(ip) => {
            let ip = canonical(ip.into());
            ensure!(
                NativeNetworkProfile::Remote.permits_endpoint(SocketAddr::new(ip, 1)),
                "Invalid ICE address"
            );
            ip.is_loopback()
        }
    };
    ensure!(
        network == NativeNetworkProfile::Remote || local,
        "Nonlocal ICE requires explicit Remote profile"
    );
    Ok(turn)
}

impl Addressing {
    pub(super) fn validate(&self, network: NativeNetworkProfile, peers: usize) -> Result<()> {
        let bind = canonical(self.bind_ip);
        ensure!(
            network.permits_listener(SocketAddr::new(bind, self.port_start)),
            "Media bind requires its explicit network profile"
        );
        ensure!(
            self.port_start >= 1024 && self.port_start <= self.port_end,
            "Invalid WebRTC UDP port range"
        );
        let ports = usize::from(self.port_end) - usize::from(self.port_start) + 1;
        ensure!(
            ports <= 4096 && ports >= peers,
            "WebRTC UDP range must cover its bounded peer capacity"
        );
        if let Some(public) = self.public_ip.map(canonical) {
            ensure!(
                network.permits_endpoint(SocketAddr::new(public, 1))
                    && public.is_ipv4() == bind.is_ipv4(),
                "Invalid WebRTC public address/family"
            );
        }
        ensure!(
            self.ice_services.len() <= 8,
            "ICE service capacity exceeded"
        );
        let mut urls = std::collections::HashSet::new();
        let mut has_turn = false;
        for service in &self.ice_services {
            ensure!(
                (1..=4).contains(&service.urls.len()),
                "Invalid ICE service URL capacity"
            );
            let mut turn = false;
            for url in &service.urls {
                turn |= ice_url(url, network)?;
                ensure!(
                    urls.insert(url) && urls.len() <= 16,
                    "Duplicate or excessive ICE URLs"
                );
            }
            let references = match (&service.username_ref, &service.credential_ref) {
                (Some(user), Some(password)) => reference(user) && reference(password),
                (None, None) => !turn,
                _ => false,
            };
            ensure!(
                references && (turn || service.username_ref.is_none()),
                "TURN requires paired secret references; STUN does not use credentials"
            );
            has_turn |= turn;
        }
        ensure!(
            self.ice_policy != IcePolicy::Relay || has_turn,
            "Relay-only ICE requires an explicit TURN service"
        );
        Ok(())
    }

    pub(super) fn apply(
        &self,
        network: NativeNetworkProfile,
        peers: usize,
        config: &mut WebRtcConfig,
        mut secret: impl FnMut(&str) -> Result<String>,
    ) -> Result<()> {
        self.validate(network, peers)?;
        let mut services = Vec::new();
        for service in &self.ice_services {
            let (username, credential) = if let (Some(user), Some(password)) =
                (&service.username_ref, &service.credential_ref)
            {
                let user = secret(user)
                    .map_err(|_| anyhow::anyhow!("WebRTC TURN username unavailable"))?;
                let password = secret(password)
                    .map_err(|_| anyhow::anyhow!("WebRTC TURN credential unavailable"))?;
                ensure!(
                    !user.trim().is_empty()
                        && user.len() <= 512
                        && !user.contains('\0')
                        && !password.trim().is_empty()
                        && password.len() <= 2048
                        && !password.contains('\0'),
                    "Invalid bounded TURN credentials"
                );
                (Some(user), Some(password))
            } else {
                (None, None)
            };
            services.push(IceServerConfig {
                urls: service.urls.clone(),
                username,
                credential,
            });
        }
        let bind = canonical(self.bind_ip);
        config.udp_bind = SocketAddr::new(bind, 0).to_string();
        config.udp_port_range = Some(UdpPortRangeConfig {
            bind_ip: bind,
            port_start: self.port_start,
            port_end: self.port_end,
        });
        config.nat_1to1_ips = self
            .public_ip
            .map(canonical)
            .map(|ip| vec![ip.to_string()])
            .unwrap_or_default();
        config.nat_1to1_candidate_type = rvoip_webrtc::Nat1To1CandidateType::Host;
        config.ice_servers = services;
        config.ice_transport_policy = match self.ice_policy {
            IcePolicy::All => IceTransportPolicy::All,
            IcePolicy::Relay => IceTransportPolicy::Relay,
        };
        Ok(())
    }
}

#[cfg(test)]
mod tests;
