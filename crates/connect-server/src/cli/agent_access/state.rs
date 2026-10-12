//! Regular agent host state retains work/session intent without task-grant binding.
use crate::cli::{
    init::{private_replace, private_write},
    task_access::{codex_host::process::TurnState, read_bounded, require_private},
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct State {
    pub profile: String,
    pub installation: String,
    pub identity_id: String,
    pub credential_id: String,
    pub session_id: Option<String>,
    pub run_id: String,
    pub prompt: String,
    pub completed: bool,
    pub attempts: u32,
}

impl State {
    pub(super) fn save(&self, directory: &Path) -> Result<()> {
        private_replace(
            &directory.join("host-state.json"),
            &serde_json::to_vec(self)?,
        )
    }
    pub(super) fn load(directory: &Path) -> Result<Option<Self>> {
        let path = directory.join("host-state.json");
        if !path.try_exists()? {
            return Ok(None);
        }
        require_private(&path)?;
        let state: Self = serde_json::from_slice(&read_bounded(&path)?)
            .map_err(|_| anyhow::anyhow!("Invalid protected agent host state"))?;
        anyhow::ensure!(
            state.profile == "connect-codex-agent-host/1",
            "Host state uses a different access mode"
        );
        uuid::Uuid::parse_str(&state.run_id).context("Invalid retained agent run ID")?;
        if let Some(id) = &state.session_id {
            uuid::Uuid::parse_str(id).context("Invalid retained agent session ID")?;
        }
        Ok(Some(state))
    }
    pub(super) fn create(&self, directory: &Path) -> Result<()> {
        private_write(
            &directory.join("host-state.json"),
            &serde_json::to_vec(self)?,
        )?;
        std::fs::File::open(directory)?.sync_all()?;
        Ok(())
    }
}

impl TurnState for State {
    fn run_id(&self) -> &str {
        &self.run_id
    }
    fn attempts(&self) -> u32 {
        self.attempts
    }
    fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }
    fn prompt(&self) -> &str {
        &self.prompt
    }
    fn remember_session(&mut self, directory: &Path, id: &str) -> Result<()> {
        self.session_id = Some(id.to_owned());
        self.save(directory)
    }
    fn mark_completed(&mut self, directory: &Path) -> Result<()> {
        self.completed = true;
        self.save(directory)
    }
}
