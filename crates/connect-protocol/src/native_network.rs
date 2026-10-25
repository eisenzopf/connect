//! Explicit native deployment policy. TLS trust and current application
//! authorization remain independent requirements in both profiles.
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NativeNetworkProfile {
    #[default]
    Local,
    Remote,
}
impl NativeNetworkProfile {
    fn unicast_or_unspecified(ip: IpAddr) -> bool {
        !ip.is_multicast() && ip != IpAddr::V4(Ipv4Addr::BROADCAST)
    }
    pub fn permits_listener(self, address: SocketAddr) -> bool {
        Self::unicast_or_unspecified(address.ip())
            && (self == Self::Remote || address.ip().is_loopback())
    }
    pub fn permits_endpoint(self, address: SocketAddr) -> bool {
        address.port() != 0 && !address.ip().is_unspecified() && self.permits_listener(address)
    }
    pub fn client_bind(self, ipv6: bool) -> SocketAddr {
        let ip = match (self, ipv6) {
            (Self::Local, false) => IpAddr::V4(Ipv4Addr::LOCALHOST),
            (Self::Local, true) => IpAddr::V6(Ipv6Addr::LOCALHOST),
            (Self::Remote, false) => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            (Self::Remote, true) => IpAddr::V6(Ipv6Addr::UNSPECIFIED),
        };
        SocketAddr::new(ip, 0)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_is_default_and_remote_is_explicit_without_weakening_endpoint_shape() {
        let local = NativeNetworkProfile::default();
        assert!(local.permits_endpoint("127.0.0.1:443".parse().unwrap()));
        assert!(local.permits_endpoint("[::1]:443".parse().unwrap()));
        assert!(!local.permits_endpoint("192.0.2.20:443".parse().unwrap()));
        let remote = NativeNetworkProfile::Remote;
        assert!(remote.permits_endpoint("192.0.2.20:443".parse().unwrap()));
        assert!(remote.permits_endpoint("[2001:db8::20]:443".parse().unwrap()));
        for profile in [local, remote] {
            for invalid in [
                "0.0.0.0:443",
                "[::]:443",
                "224.0.0.1:443",
                "[ff02::1]:443",
                "255.255.255.255:443",
                "127.0.0.1:0",
            ] {
                assert!(
                    !profile.permits_endpoint(invalid.parse().unwrap()),
                    "{invalid}"
                );
            }
        }
        assert!(remote.permits_listener("0.0.0.0:443".parse().unwrap()));
        assert!(!local.permits_listener("0.0.0.0:443".parse().unwrap()));
        assert!(local.client_bind(false).ip().is_loopback());
        assert!(remote.client_bind(false).ip().is_unspecified());
        assert!(remote.client_bind(true).ip().is_unspecified());
    }
}
