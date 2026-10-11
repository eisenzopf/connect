//! Private operator inputs; models cannot choose a task or backing credential.
use crate::{
    cli::task_access::{read_bounded, require_private},
    vapi_bootstrap::Definition,
};
use anyhow::{ensure, Result};
use connect_connectors::vapi_origin::OriginKind;
use serde::Deserialize;
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Manifest {
    pub profile: String,
    pub definition: Definition,
    pub state_directory: PathBuf,
    pub session_id: Option<String>,
    pub callback_url: Option<String>,
    pub callback_bind: Option<SocketAddr>,
}
impl Manifest {
    pub fn load(path: &Path) -> Result<Self> {
        require_private(path)?;
        let mut value: Self = serde_json::from_slice(&read_bounded(path)?)
            .map_err(|_| anyhow::anyhow!("Invalid protected Vapi task host manifest"))?;
        ensure!(
            value.profile == "connect-vapi-task-host/1",
            "Unsupported Vapi task host profile"
        );
        if let Some(url) = &value.callback_url {
            crate::vapi_bootstrap::validate_url(url)?;
        }
        ensure!(
            !value.state_directory.as_os_str().is_empty(),
            "Vapi setup directory is required"
        );
        if value.state_directory.is_relative() {
            value.state_directory = path
                .parent()
                .unwrap_or(Path::new("."))
                .join(&value.state_directory);
        }
        match value.definition.origin {
            OriginKind::Call => {
                let id = value
                    .session_id
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("Voice requires an accepted task session"))?;
                connect_protocol::tasks::validate_task_identifier(id)?;
                ensure!(
                    value.callback_bind.is_none(),
                    "Voice callback binding belongs to the media lifecycle"
                );
            }
            OriginKind::ChatSession => {
                ensure!(
                    value.session_id.is_none(),
                    "Chat session setup cannot borrow a voice session"
                );
                let bind = value.callback_bind.ok_or_else(|| {
                    anyhow::anyhow!("Chat requires a stable loopback callback binding")
                })?;
                ensure!(
                    bind.ip().is_loopback() && bind.port() != 0,
                    "Chat callback requires a nonzero loopback port"
                );
            }
            OriginKind::Chat => {
                anyhow::bail!("Standalone chat setup is unsupported; choose chat_session")
            }
        }
        Ok(value)
    }
}

#[cfg(test)]
#[path = "manifest_tests.rs"]
mod tests;
