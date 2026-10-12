//! Protected adapter state is correlation, never communication authority.
use crate::cli::init::{private_replace, private_write};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct State {
    pub profile: String,
    pub installation: String,
    pub task_id: String,
    pub grant_id: String,
    pub session_id: Option<String>,
    pub run_id: String,
    pub prompt: String,
    pub completed: bool,
    pub attempts: u32,
}
impl State {
    pub fn save(&self, directory: &Path) -> Result<()> {
        private_replace(
            &directory.join("host-state.json"),
            &serde_json::to_vec(self)?,
        )
    }
    pub fn load(directory: &Path) -> Result<Option<Self>> {
        let path = directory.join("host-state.json");
        if !path.try_exists()? {
            return Ok(None);
        }
        super::super::require_private(&path)?;
        let state: Self = serde_json::from_slice(&super::super::read_bounded(&path)?)
            .context("Invalid protected Codex state")?;
        anyhow::ensure!(
            state.profile == "connect-codex-host/1",
            "Unsupported host state"
        );
        uuid::Uuid::parse_str(&state.run_id).context("Invalid retained run ID")?;
        if let Some(id) = &state.session_id {
            uuid::Uuid::parse_str(id).context("Invalid retained session ID")?;
        }
        Ok(Some(state))
    }
    pub fn create(&self, directory: &Path) -> Result<()> {
        private_write(
            &directory.join("host-state.json"),
            &serde_json::to_vec(self)?,
        )?;
        std::fs::File::open(directory)?.sync_all()?;
        Ok(())
    }
}

impl super::process::TurnState for State {
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
