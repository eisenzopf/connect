use anyhow::{bail, Context, Result};
use connect_protocol::Limits;
use serde::{Deserialize, Serialize};
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    pub bind: SocketAddr,
    pub websocket_bind: Option<SocketAddr>,
    pub database: PathBuf,
    pub certificate: PathBuf,
    pub private_key: PathBuf,
    #[serde(default)]
    pub limits: Limits,
}
impl ServerConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).context("Read server configuration")?;
        let mut config: Self =
            toml::from_str(&text).map_err(|_| anyhow::anyhow!("Invalid server configuration"))?;
        config.validate()?;
        let parent = path.parent().unwrap_or(Path::new("."));
        for field in [
            &mut config.database,
            &mut config.certificate,
            &mut config.private_key,
        ] {
            if field.is_relative() {
                *field = parent.join(&*field);
            }
        }
        Ok(config)
    }
    pub fn validate(&self) -> Result<()> {
        if !self.bind.ip().is_loopback()
            || self
                .websocket_bind
                .is_some_and(|addr| !addr.ip().is_loopback())
        {
            bail!("The local profile requires loopback listener addresses")
        }
        let l = &self.limits;
        if l.message_bytes == 0
            || l.message_bytes > 64 * 1024
            || l.page_size == 0
            || l.page_size > 1000
            || l.participants == 0
            || l.participants > 128
            || l.journal_events == 0
            || l.operation_receipts == 0
            || l.message_storage_bytes == 0
            || l.wait_ms > 5000
            || l.request_timeout_ms == 0
            || l.request_timeout_ms > 60_000
            || l.connections == 0
            || l.connections > 1024
        {
            bail!("Local limits are invalid or exceed the bounded profile")
        }
        Ok(())
    }
}
