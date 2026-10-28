//! Explicit native configuration and optional named application read services.
//!
//! ```no_run
//! # async fn configured(path: &std::path::Path) -> connect_client::sdk::Result<()> {
//! use connect_client::sdk::{Client, ClientConfig, RequestOptions};
//! use std::time::Duration;
//! // Local/remote native trust is explicit. Optional application HTTP is a
//! // separately configured loopback endpoint using the exact same credential.
//! let client = Client::connect_configured(ClientConfig::load(path)?).await?;
//! client.close(RequestOptions::new(Duration::from_secs(5))?).await?;
//! # Ok(()) }
//! ```
use super::*;
use crate::{
    application::{ApplicationClient, ApplicationClientConfig},
    canonical::CanonicalClientConfig,
};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientConfig {
    pub control: CanonicalClientConfig,
    pub tenant_id: TenantId,
    /// Participant Client uses named application reads only. Explicit
    /// AdministrativeClient also requires this table for trusted provisioning.
    /// Never a canonical transport fallback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub application: Option<ApplicationClientConfig>,
}
impl std::fmt::Debug for ClientConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientConfig").finish_non_exhaustive()
    }
}
impl ClientConfig {
    /// Synchronous filesystem read; call on an appropriate blocking executor.
    /// Native certificate/key paths resolve relative to this configuration file.
    pub fn load(path: &Path) -> Result<Self> {
        let raw =
            std::fs::read_to_string(path).map_err(|_| Error::local(ErrorKind::Configuration))?;
        let mut value: Self =
            toml::from_str(&raw).map_err(|_| Error::local(ErrorKind::Configuration))?;
        for p in [
            &mut value.control.trust_certificate,
            &mut value.control.signing_private_key,
            &mut value.control.server_public_key,
        ] {
            if p.is_relative() {
                *p = path.parent().unwrap_or(Path::new(".")).join(&*p);
            }
        }
        value.validate()?;
        Ok(value)
    }
    /// All profile/endpoint/credential checks run before transport construction.
    pub fn validate(&self) -> Result<()> {
        self.control
            .validate()
            .map_err(|_| Error::local(ErrorKind::Configuration))?;
        if let Some(application) = &self.application {
            application
                .validate()
                .map_err(|_| Error::local(ErrorKind::Configuration))?;
            if application.credential != self.control.credential {
                return Err(Error::local(ErrorKind::Configuration));
            }
        }
        Ok(())
    }
}
impl Client {
    pub async fn connect_configured(config: ClientConfig) -> Result<Self> {
        config.validate()?;
        let application = match config.application {
            Some(config) => Some(
                ApplicationClient::open(config)
                    .await
                    .map_err(|e| Error::transport(e, None))?,
            ),
            None => None,
        };
        let client = Self::connect(config.control, config.tenant_id).await?;
        Ok(match application {
            Some(application) => client.with_application_reads(application),
            None => client,
        })
    }
    // Only validated same-credential configuration may attach public services.
    // This private seam also supports isolated signed/native+HTTP fixtures.
    pub(crate) fn with_application_reads(mut self, application: ApplicationClient) -> Self {
        self.application = Some(application_reads::OwnedReads::start(
            application,
            self.shared.closed.clone(),
        ));
        self
    }
}
