//! Configured canonical transport metadata. Discovery installs no trust or grant.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};

pub const PATH: &str = "/v2/discovery";
pub const MAX_RESPONSE_BYTES: usize = 8192;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransportDiscovery {
    pub profile: String,
    pub max_control_envelope_bytes: u32,
    pub max_physical_peers: u32,
    pub webtransport: Option<WebTransportEndpoint>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebTransportEndpoint {
    pub url: String,
    pub origin_required: bool,
    pub datagrams: DatagramNegotiation,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DatagramNegotiation {
    /// Actual availability/MTU is established on each WebTransport session.
    PerSession,
}
impl TransportDiscovery {
    pub fn validate(&self) -> Result<()> {
        let invalid = || Error::new("invalid-request", "Invalid canonical transport discovery");
        if self.profile != crate::canonical_http::PROFILE
            || !(1..=1_048_576).contains(&self.max_control_envelope_bytes)
            || !(1..=1024).contains(&self.max_physical_peers)
        {
            return Err(invalid());
        }
        if let Some(wt) = &self.webtransport {
            if !wt.origin_required
                || wt.url.len() > 2048
                || !wt.url.is_ascii()
                || wt
                    .url
                    .bytes()
                    .any(|b| b.is_ascii_control() || b.is_ascii_whitespace())
            {
                return Err(invalid());
            }
            let url = url::Url::parse(&wt.url).map_err(|_| invalid())?;
            if url.scheme() != "https"
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
                || url.port() == Some(0)
            {
                return Err(invalid());
            }
        }
        Ok(())
    }
}
