//! Protected WebSocket listener configuration. Canonical peer authentication
//! and resource authorization remain shared with the native listener.
use anyhow::{ensure, Result};
use connect_protocol::native_network::NativeNetworkProfile;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, net::SocketAddr};
use url::{Host, Url};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalWebSocketConfig {
    pub bind: SocketAddr,
    pub mount_path: String,
    pub allowed_origins: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advertised_url: Option<String>,
}
impl CanonicalWebSocketConfig {
    pub fn validate(&self, profile: NativeNetworkProfile) -> Result<()> {
        ensure!(
            profile.permits_listener(self.bind),
            "WebSocket bind does not match the selected network profile"
        );
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
            "Invalid WebSocket mount path"
        );
        ensure!(
            (1..=32).contains(&self.allowed_origins.len()),
            "WebSocket requires bounded explicit browser origins"
        );
        let mut origins = HashSet::new();
        for origin in &self.allowed_origins {
            ensure!(
                origin.len() <= 2048 && origin.is_ascii(),
                "Invalid WebSocket origin"
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
                "WebSocket origins must be exact serialized origins"
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
                "WebSocket requires secure origins except explicit local development"
            );
            ensure!(origins.insert(origin), "Duplicate WebSocket origin");
        }
        ensure!(
            profile != NativeNetworkProfile::Remote || self.advertised_url.is_some(),
            "Remote WebSocket requires an explicit advertised WSS URL"
        );
        if let Some(value) = &self.advertised_url {
            ensure!(
                value.len() <= 2048
                    && value.is_ascii()
                    && !value
                        .bytes()
                        .any(|b| b.is_ascii_control() || b.is_ascii_whitespace()),
                "Invalid advertised WebSocket URL"
            );
            let url = Url::parse(value)?;
            ensure!(
                url.scheme() == "wss"
                    && url.host().is_some()
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.query().is_none()
                    && url.fragment().is_none()
                    && url.path() == self.mount_path
                    && url.port() != Some(0),
                "Advertised WebSocket URL must name the exact WSS mount"
            );
            match url.host() {
                Some(Host::Ipv4(ip)) => ensure!(
                    profile.permits_endpoint(SocketAddr::new(ip.into(), 1)),
                    "Invalid advertised WebSocket IP"
                ),
                Some(Host::Ipv6(ip)) => ensure!(
                    profile.permits_endpoint(SocketAddr::new(ip.into(), 1)),
                    "Invalid advertised WebSocket IP"
                ),
                Some(Host::Domain(name)) => {
                    ensure!(!name.contains('*'), "Invalid advertised WebSocket host")
                }
                None => unreachable!(),
            }
        }
        Ok(())
    }
    pub fn contact(&self, address: SocketAddr) -> String {
        self.advertised_url
            .clone()
            .unwrap_or_else(|| format!("wss://{address}{}", self.mount_path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> CanonicalWebSocketConfig {
        CanonicalWebSocketConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            mount_path: "/connect".into(),
            allowed_origins: vec!["https://browser.fixture.invalid".into()],
            advertised_url: Some("wss://server.fixture.invalid/connect".into()),
        }
    }
    #[test]
    fn bind_policy_allows_remote_wildcards_but_never_multicast_or_broadcast() {
        for address in ["0.0.0.0:443", "[::]:443", "192.0.2.5:443"] {
            let mut value = config();
            value.bind = address.parse().unwrap();
            assert!(value.validate(NativeNetworkProfile::Local).is_err());
            value.validate(NativeNetworkProfile::Remote).unwrap();
        }
        for address in ["224.0.0.1:443", "[ff02::1]:443", "255.255.255.255:443"] {
            let mut value = config();
            value.bind = address.parse().unwrap();
            for profile in [NativeNetworkProfile::Local, NativeNetworkProfile::Remote] {
                assert!(value.validate(profile).is_err());
            }
        }
        let mut value = config();
        value.bind = "[::ffff:127.0.0.1]:0".parse().unwrap();
        value.validate(NativeNetworkProfile::Local).unwrap();
    }
    #[test]
    fn derived_local_contact_preserves_bound_ipv4_and_ipv6_address_family() {
        let mut value = config();
        value.advertised_url = None;
        assert_eq!(
            value.contact("127.0.0.1:1234".parse().unwrap()),
            "wss://127.0.0.1:1234/connect"
        );
        assert_eq!(
            value.contact("[::1]:1234".parse().unwrap()),
            "wss://[::1]:1234/connect"
        );
    }
    #[test]
    fn exact_origins_mount_and_remote_contact_are_required_before_startup() {
        let good = config();
        good.validate(NativeNetworkProfile::Remote).unwrap();
        assert_eq!(
            good.contact("127.0.0.1:1234".parse().unwrap()),
            "wss://server.fixture.invalid/connect"
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
            " wss://server.fixture.invalid/connect",
            "wss://server.fixture.invalid/connect ",
            "wss://server.fixture.invalid/connect\n",
            "wss://ser\tver.fixture.invalid/connect",
            "wss://server.fixture.invalid/co\rnnect",
            "wss://user:secret@server.fixture.invalid/connect",
            "ws://server.fixture.invalid/connect",
            "wss://server.fixture.invalid/wrong",
            "wss://server.fixture.invalid/connect?x=1",
            "wss://server.fixture.invalid/connect#fragment",
            "wss://server.fixture.invalid:0/connect",
            "wss://0.0.0.0/connect",
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
