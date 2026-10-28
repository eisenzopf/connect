//! Explicit HTTPS exposure. Forwarded metadata never supplies application authority.
use anyhow::{ensure, Result};
use connect_protocol::native_network::NativeNetworkProfile;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    net::{IpAddr, SocketAddr},
};
use url::{Host, Url};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HttpSurface {
    Application,
    Administration,
    Callbacks,
    Health,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelnyxCallbacks {
    /// Provider verification public key, never an account API credential.
    pub public_key: String,
    pub routes: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpsListenerConfig {
    pub bind: SocketAddr,
    pub surface: HttpSurface,
    /// Exact HTTPS origin advertised by deployment. Required for Remote.
    /// Local omission uses the actual bound loopback address.
    #[serde(default)]
    pub public_origin: Option<String>,
    #[serde(default)]
    pub allowed_origins: Vec<String>,
    /// Exact immediate socket-peer IPs. Supports one-hop X-Forwarded-* only.
    #[serde(default)]
    pub trusted_proxies: Vec<IpAddr>,
    #[serde(default)]
    pub telnyx_callbacks: Option<TelnyxCallbacks>,
}
pub(crate) fn origin(value: &str, profile: NativeNetworkProfile, browser: bool) -> Result<Url> {
    ensure!(
        value.len() <= 2048 && value.is_ascii(),
        "Invalid hosting origin"
    );
    let parsed = Url::parse(value).map_err(|_| anyhow::anyhow!("Invalid hosting origin"))?;
    let local_http = browser
        && profile == NativeNetworkProfile::Local
        && parsed.scheme() == "http"
        && match parsed.host() {
            Some(Host::Domain("localhost")) => true,
            Some(Host::Ipv4(ip)) => ip.is_loopback(),
            Some(Host::Ipv6(ip)) => ip.is_loopback(),
            _ => false,
        };
    ensure!(
        parsed.host().is_some()
            && (parsed.scheme() == "https" || local_http)
            && parsed.username().is_empty()
            && parsed.password().is_none()
            && parsed.path() == "/"
            && parsed.query().is_none()
            && parsed.fragment().is_none()
            && parsed.port() != Some(0)
            && !value.contains('*')
            && parsed.origin().ascii_serialization() == value,
        "Hosting requires an exact secure origin"
    );
    if let Some(Host::Ipv4(ip)) = parsed.host() {
        ensure!(
            !ip.is_unspecified() && !ip.is_multicast(),
            "Invalid hosting IP"
        );
    }
    if let Some(Host::Ipv6(ip)) = parsed.host() {
        ensure!(
            !ip.is_unspecified() && !ip.is_multicast(),
            "Invalid hosting IP"
        );
    }
    Ok(parsed)
}
impl HttpsListenerConfig {
    pub fn validate(&self, profile: NativeNetworkProfile) -> Result<()> {
        ensure!(
            profile.permits_listener(self.bind),
            "HTTPS bind requires its explicit network profile"
        );
        ensure!(
            profile != NativeNetworkProfile::Remote || self.public_origin.is_some(),
            "Remote HTTPS requires an explicit public origin"
        );
        if let Some(value) = &self.public_origin {
            origin(value, profile, false)?;
        }
        ensure!(
            self.allowed_origins.len() <= 32 && self.trusted_proxies.len() <= 32,
            "Hosting policy capacity exceeded"
        );
        ensure!(
            self.surface == HttpSurface::Application || self.allowed_origins.is_empty(),
            "Only application HTTPS permits browser origins"
        );
        let mut seen = HashSet::new();
        for value in &self.allowed_origins {
            origin(value, profile, true)?;
            ensure!(seen.insert(value), "Duplicate hosting origin");
        }
        let mut seen = HashSet::new();
        for ip in &self.trusted_proxies {
            ensure!(
                !ip.is_unspecified() && !ip.is_multicast() && seen.insert(ip),
                "Invalid or duplicate trusted proxy"
            );
        }
        ensure!(
            (self.surface == HttpSurface::Callbacks) == self.telnyx_callbacks.is_some(),
            "Callback HTTPS requires an explicit verifier and route ceiling"
        );
        if let Some(callback) = &self.telnyx_callbacks {
            ensure!(
                callback.public_key.len() <= 256 && (1..=64).contains(&callback.routes.len()),
                "Invalid callback configuration"
            );
            telnyx::webhooks::Verifier::new(&callback.public_key)
                .map_err(|_| anyhow::anyhow!("Invalid callback verifier"))?;
            let mut seen = HashSet::new();
            for route in &callback.routes {
                ensure!(
                    !route.is_empty()
                        && route.len() <= 256
                        && route.bytes().all(|b| b.is_ascii_graphic())
                        && seen.insert(route),
                    "Invalid callback route ceiling"
                );
            }
        }
        Ok(())
    }
    pub(crate) fn contact(&self, address: SocketAddr) -> String {
        self.public_origin
            .clone()
            .unwrap_or_else(|| format!("https://{address}"))
    }
    pub(crate) fn name(&self) -> String {
        if let Some(value) = &self.public_origin {
            match Url::parse(value)
                .expect("validated origin")
                .host()
                .expect("validated host")
            {
                Host::Domain(name) => name.to_owned(),
                Host::Ipv4(ip) => ip.to_string(),
                Host::Ipv6(ip) => ip.to_string(),
            }
        } else {
            self.bind.ip().to_string()
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct HttpsEndpoint {
    pub surface: HttpSurface,
    pub address: SocketAddr,
    pub origin: String,
}
