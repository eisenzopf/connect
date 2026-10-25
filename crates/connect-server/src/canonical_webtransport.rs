//! Protected WebTransport listener configuration. Canonical peer authentication
//! and resource authorization remain shared with the native listener.
use anyhow::{ensure, Result};
use connect_protocol::native_network::NativeNetworkProfile;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, net::SocketAddr};
use url::{Host, Url};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalWebTransportConfig {
    pub mount_path: String,
    pub allowed_origins: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advertised_url: Option<String>,
}
impl CanonicalWebTransportConfig {
    pub fn validate(&self, profile: NativeNetworkProfile) -> Result<()> {
        ensure!(
            self.mount_path.starts_with('/')
                && self.mount_path.len() <= 256
                && self
                    .mount_path
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"/-_.~".contains(&b))
                && !self
                    .mount_path
                    .split('/')
                    .any(|part| matches!(part, "." | ".."))
                && !self.mount_path.contains("//"),
            "Invalid WebTransport mount path"
        );
        ensure!(
            (1..=32).contains(&self.allowed_origins.len()),
            "WebTransport requires bounded explicit browser origins"
        );
        let mut origins = HashSet::new();
        for origin in &self.allowed_origins {
            ensure!(
                origin.len() <= 2048 && origin.is_ascii(),
                "Invalid WebTransport origin"
            );
            let parsed = Url::parse(origin)?;
            ensure!(
                parsed.host().is_some()
                    && parsed.username().is_empty()
                    && parsed.password().is_none()
                    && parsed.query().is_none()
                    && parsed.fragment().is_none()
                    && parsed.path() == "/"
                    && !origin.contains('*')
                    && parsed.origin().ascii_serialization() == *origin,
                "WebTransport origins must be exact serialized origins"
            );
            let local_http = profile == NativeNetworkProfile::Local
                && parsed.scheme() == "http"
                && match parsed.host() {
                    Some(Host::Domain(name)) => name == "localhost",
                    Some(Host::Ipv4(ip)) => ip.is_loopback(),
                    Some(Host::Ipv6(ip)) => ip.is_loopback(),
                    None => false,
                };
            ensure!(
                parsed.scheme() == "https" || local_http,
                "WebTransport requires secure origins except explicit local development"
            );
            ensure!(origins.insert(origin), "Duplicate WebTransport origin");
        }
        ensure!(
            profile != NativeNetworkProfile::Remote || self.advertised_url.is_some(),
            "Remote WebTransport requires an explicit advertised HTTPS URL"
        );
        if let Some(value) = &self.advertised_url {
            ensure!(
                value.len() <= 2048
                    && value.is_ascii()
                    && !value
                        .bytes()
                        .any(|b| b.is_ascii_control() || b.is_ascii_whitespace()),
                "Invalid advertised WebTransport URL"
            );
            let url = Url::parse(value)?;
            ensure!(
                url.scheme() == "https"
                    && url.host().is_some()
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.query().is_none()
                    && url.fragment().is_none()
                    && url.path() == self.mount_path
                    && url.port() != Some(0),
                "Advertised WebTransport URL must name the exact HTTPS mount"
            );
            match url.host() {
                Some(Host::Ipv4(ip)) => ensure!(
                    profile.permits_endpoint(SocketAddr::new(ip.into(), 1)),
                    "Invalid advertised WebTransport IP"
                ),
                Some(Host::Ipv6(ip)) => ensure!(
                    profile.permits_endpoint(SocketAddr::new(ip.into(), 1)),
                    "Invalid advertised WebTransport IP"
                ),
                Some(Host::Domain(name)) => {
                    ensure!(!name.contains('*'), "Invalid advertised WebTransport host")
                }
                None => unreachable!(),
            }
        }
        Ok(())
    }
    pub fn contact(&self, address: SocketAddr) -> String {
        self.advertised_url
            .clone()
            .unwrap_or_else(|| format!("https://localhost:{}{}", address.port(), self.mount_path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> CanonicalWebTransportConfig {
        CanonicalWebTransportConfig {
            mount_path: "/connect".into(),
            allowed_origins: vec!["https://browser.fixture.invalid".into()],
            advertised_url: Some("https://server.fixture.invalid/connect".into()),
        }
    }
    #[test]
    fn exact_origins_mount_and_remote_contact_are_required_before_startup() {
        let good = config();
        good.validate(NativeNetworkProfile::Remote).unwrap();
        assert_eq!(
            good.contact("127.0.0.1:1234".parse().unwrap()),
            "https://server.fixture.invalid/connect"
        );
        let mut value = config();
        value.advertised_url = None;
        assert!(value.validate(NativeNetworkProfile::Remote).is_err());
        value.validate(NativeNetworkProfile::Local).unwrap();
        for path in [
            "",
            "connect",
            "/../connect",
            "/a/./connect",
            "//connect",
            "/connect?x=1",
            "/%63onnect",
        ] {
            let mut value = config();
            value.mount_path = path.into();
            assert!(
                value.validate(NativeNetworkProfile::Remote).is_err(),
                "accepted mount {path:?}"
            );
        }
        for origin in [
            "null",
            "*",
            "https://*.fixture.invalid",
            "https://browser.fixture.invalid/",
            "https://browser.fixture.invalid/path",
            "https://user@browser.fixture.invalid",
            "https://browser.fixture.invalid?x=1",
            "https://browser.fixture.invalid#fragment",
            "https://browser.fixture.invalid\n",
            "http://browser.fixture.invalid",
        ] {
            let mut value = config();
            value.allowed_origins = vec![origin.into()];
            assert!(
                value.validate(NativeNetworkProfile::Remote).is_err(),
                "accepted origin {origin:?}"
            );
        }
        for origins in [
            vec![],
            vec![good.allowed_origins[0].clone(); 2],
            vec![good.allowed_origins[0].clone(); 33],
        ] {
            let mut value = config();
            value.allowed_origins = origins;
            assert!(value.validate(NativeNetworkProfile::Remote).is_err());
        }
        for origin in [
            "http://localhost:8080",
            "http://127.0.0.1:8080",
            "http://[::1]:8080",
        ] {
            let mut value = config();
            value.allowed_origins = vec![origin.into()];
            value.validate(NativeNetworkProfile::Local).unwrap();
            assert!(value.validate(NativeNetworkProfile::Remote).is_err());
        }
    }
    #[test]
    fn advertised_contact_refuses_parser_normalization_of_whitespace_and_controls() {
        for url in [
            " https://server.fixture.invalid/connect",
            "https://server.fixture.invalid/connect ",
            "https://server.fixture.invalid/connect\n",
            "https://ser\tver.fixture.invalid/connect",
            "https://server.fixture.invalid/co\rnnect",
            "https://user:secret@server.fixture.invalid/connect",
            "http://server.fixture.invalid/connect",
            "https://server.fixture.invalid/wrong",
            "https://server.fixture.invalid/connect?x=1",
            "https://server.fixture.invalid/connect#fragment",
            "https://server.fixture.invalid:0/connect",
            "https://0.0.0.0/connect",
        ] {
            let mut value = config();
            value.advertised_url = Some(url.into());
            assert!(
                value.validate(NativeNetworkProfile::Remote).is_err(),
                "accepted contact {url:?}"
            );
        }
    }
}
