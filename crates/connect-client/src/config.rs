//! Protected client configuration and mandatory local trust.
use crate::{ClientError, Result};
use serde::{Deserialize, Serialize};
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientConfig {
    pub endpoint: SocketAddr,
    pub server_name: String,
    pub trust_certificate: PathBuf,
    pub credential: String,
    pub identity_id: String,
    pub credential_id: String,
    #[serde(default = "default_deadline")]
    pub timeout_ms: u64,
}
fn default_deadline() -> u64 {
    10_000
}
impl ClientConfig {
    pub fn load(path: &Path) -> std::result::Result<Self, ClientError> {
        let content = std::fs::read_to_string(path).map_err(|_| ClientError::Configuration)?;
        let mut config: Self = toml::from_str(&content).map_err(|_| ClientError::Configuration)?;
        if config.trust_certificate.is_relative() {
            config.trust_certificate = path
                .parent()
                .unwrap_or(Path::new("."))
                .join(&config.trust_certificate);
        }
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<()> {
        if !self.endpoint.ip().is_loopback()
            || self.timeout_ms == 0
            || self.timeout_ms > 60_000
            || self.server_name.is_empty()
            || self.credential.len() != 64
        {
            return Err(ClientError::Configuration);
        }
        Ok(())
    }
}
