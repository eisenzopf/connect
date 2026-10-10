//! Durable exact intents; uncertainty never changes their IDs or CAS revision.
use super::state::TaskStore;
use anyhow::{Context, Result};
use connect_client::tasks::{TaskCommand, TaskOutcome};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

/// A protected controller lane cannot be selected by model input.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Lane {
    Controller,
    Invocation,
}
impl Lane {
    fn name(self) -> &'static str {
        match self {
            Self::Controller => "controller",
            Self::Invocation => "invocation",
        }
    }
}

pub(super) struct Prepared {
    pub key: String,
    pub lane: Lane,
    pub command: TaskCommand,
    pub outcome: Option<TaskOutcome>,
}

impl TaskStore {
    /// Existing intent wins on restart. Fresh UUIDs are used only for new keys.
    pub(super) fn prepare(&self, key: &str, lane: Lane, command: TaskCommand) -> Result<Prepared> {
        insert(&self.connection, key, lane, &command)?;
        self.command(key)?.context("Prepared intent disappeared")
    }

    pub(super) fn command(&self, key: &str) -> Result<Option<Prepared>> {
        let row: Option<(String, String, Option<String>)> = self
            .connection
            .query_row(
                "SELECT lane,command,outcome FROM runner_commands WHERE key=?1",
                [key],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        row.map(|(lane, command, outcome)| {
            Ok(Prepared {
                key: key.into(),
                lane: match lane.as_str() {
                    "controller" => Lane::Controller,
                    "invocation" => Lane::Invocation,
                    _ => anyhow::bail!("Invalid persisted authority lane"),
                },
                command: serde_json::from_str(&command)?,
                outcome: outcome.map(|s| serde_json::from_str(&s)).transpose()?,
            })
        })
        .transpose()
    }

    pub(super) fn accepted(&self, key: &str, outcome: &TaskOutcome) -> Result<()> {
        let prior = self.command(key)?.context("Unknown command acceptance")?;
        if let Some(saved) = prior.outcome {
            anyhow::ensure!(
                saved == *outcome,
                "Acceptance changed; reconcile without rewriting"
            );
        } else {
            self.connection.execute(
                "UPDATE runner_commands SET outcome=?1 WHERE key=?2 AND outcome IS NULL",
                params![serde_json::to_string(outcome)?, key],
            )?;
        }
        Ok(())
    }

    /// Record a known application rejection separately from unknown acceptance.
    /// Retain the original command; restarting never silently rebases a CAS.
    pub(super) fn rejected(&self, key: &str, code: &str) -> Result<()> {
        self.connection.execute(
            "INSERT INTO runner_rejections(command_key,code) VALUES(?1,?2)",
            params![key, code],
        )?;
        Ok(())
    }

    pub(super) fn pending_invocations(&self) -> Result<Vec<Prepared>> {
        let mut statement = self.connection.prepare(
            "SELECT key FROM runner_commands WHERE lane='invocation' AND outcome IS NULL ORDER BY sequence LIMIT 100")?;
        let keys = statement
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        keys.into_iter()
            .map(|key| self.command(&key)?.context("Missing command"))
            .collect()
    }
}

pub(super) fn insert(c: &Connection, key: &str, lane: Lane, command: &TaskCommand) -> Result<()> {
    c.execute(
        "INSERT OR IGNORE INTO runner_commands(key,lane,command) VALUES(?1,?2,?3)",
        params![key, lane.name(), serde_json::to_string(command)?],
    )?;
    Ok(())
}

pub(super) fn logical_id() -> String {
    uuid::Uuid::new_v4().to_string()
}
