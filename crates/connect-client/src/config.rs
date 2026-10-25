//! Protected client configuration and native network policy.
use crate::{ClientError, Result};
pub use connect_protocol::native_network::NativeNetworkProfile;
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
    /// Build protected SDK configuration with only the issued Native grant.
    /// The backing identity and trust settings remain metadata; the parent
    /// credential is replaced. Host/provider grants use their own adapters.
    pub fn with_task_grant(
        mut self,
        grant: &connect_protocol::tasks::ProtectedTaskGrant,
    ) -> Result<Self> {
        use connect_protocol::tasks::{task_grant_subject, InvocationKind};
        if grant.grant.binding.kind != InvocationKind::Native || grant.grant.revoked {
            return Err(ClientError::Configuration);
        }
        self.credential_id =
            task_grant_subject(&grant.grant.id).map_err(|_| ClientError::Configuration)?;
        self.credential = grant.expose_secret().to_owned();
        self.validate()?;
        Ok(self)
    }
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
            || !self.valid_credential_shape()
        {
            return Err(ClientError::Configuration);
        }
        Ok(())
    }
    fn valid_credential_shape(&self) -> bool {
        use connect_protocol::tasks::{task_grant_id, TASK_PRINCIPAL_PREFIX};
        if self.credential_id.starts_with(TASK_PRINCIPAL_PREFIX) {
            task_grant_id(&self.credential_id).is_ok()
                && !self.credential.is_empty()
                && self.credential.len() <= 256
                && self.credential.is_ascii()
        } else {
            self.credential.len() == 64
        }
    }
}
