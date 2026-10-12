//! Protected adapter intent is saved before a possibly uncertain provider POST.
use super::Definition;
use crate::cli::{
    init::private_replace,
    task_access::{read_bounded, require_private},
};
use anyhow::{Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    path::{Path, PathBuf},
};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct State {
    pub profile: String,
    pub definition: Definition,
    pub nonce: String,
    pub secret: String,
    pub mcp_url: String,
    pub posted: bool,
    pub external_id: Option<String>,
}
pub(super) struct Locked {
    _file: File,
    directory: PathBuf,
    pub state: State,
}
impl Locked {
    pub fn load(directory: &Path) -> Result<Self> {
        let meta = std::fs::symlink_metadata(directory)?;
        anyhow::ensure!(
            meta.file_type().is_dir(),
            "Expected private bootstrap directory"
        );
        require_private(directory)?;
        let lock = directory.join("bootstrap.lock");
        require_private(&lock)?;
        read_bounded(&lock)?;
        let file = std::fs::OpenOptions::new().write(true).open(lock)?;
        file.try_lock_exclusive()
            .context("Provider bootstrap is busy; retain the original intent")?;
        let path = directory.join("intent.json");
        require_private(&path)?;
        let state: State = serde_json::from_slice(&read_bounded(&path)?)
            .map_err(|_| anyhow::anyhow!("Invalid protected bootstrap state"))?;
        anyhow::ensure!(
            state.profile == "connect-vapi-bootstrap/1",
            "Unsupported bootstrap state"
        );
        uuid::Uuid::parse_str(&state.nonce)?;
        anyhow::ensure!(
            state.secret.len() == 64 && state.secret.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid bootstrap credential"
        );
        super::validate_url(&state.mcp_url)?;
        Ok(Self {
            _file: file,
            directory: directory.into(),
            state,
        })
    }
    pub fn save(&self) -> Result<()> {
        private_replace(
            &self.directory.join("intent.json"),
            &serde_json::to_vec(&self.state)?,
        )
    }
}
