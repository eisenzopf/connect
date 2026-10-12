//! Only a definitive consume-CAS rejection permits a new protected control intent.
use super::{
    commands::{insert, logical_id, Lane, Prepared},
    journal::consume_key,
    state::TaskStore,
};
use anyhow::{Context, Result};
use connect_client::{
    tasks::{TaskCommand, TaskOutcome},
    ClientError,
};
use rusqlite::OptionalExtension;

impl TaskStore {
    pub(super) fn consume_intent(&self, event: &str) -> Result<Option<Prepared>> {
        let key = self
            .metadata(&pointer(event))?
            .unwrap_or_else(|| consume_key(event));
        self.command(&key)
    }

    /// Never rebase unknown acceptance. Keep the rejected original in the ledger
    /// and atomically save a new explicit control intent plus its active pointer.
    pub(super) fn consumption(&mut self, event: &str, revision: u64) -> Result<Prepared> {
        let key = self
            .metadata(&pointer(event))?
            .unwrap_or_else(|| consume_key(event));
        let prepared = self.prepare(
            &key,
            Lane::Controller,
            TaskCommand::ConsumeReply {
                request_id: logical_id(),
                task_id: self.binding.task_id.clone(),
                event_id: event.into(),
                expected_revision: revision,
            },
        )?;
        let rejected: Option<String> = self.connection.query_row(
            "SELECT code FROM runner_rejections WHERE command_key=?1 ORDER BY sequence DESC LIMIT 1",
            [&key], |r| r.get(0)).optional()?;
        if prepared.outcome.is_some() || rejected.as_deref() != Some("task-state-conflict") {
            return Ok(prepared);
        }
        let TaskCommand::ConsumeReply {
            task_id,
            event_id,
            expected_revision,
            ..
        } = &prepared.command
        else {
            anyhow::bail!("Only a rejected consume CAS can be reconciled");
        };
        anyhow::ensure!(
            prepared.lane == Lane::Controller
                && task_id == &self.binding.task_id
                && event_id == event,
            "Consume reconciliation changed binding"
        );
        if *expected_revision == revision {
            return Ok(prepared);
        }
        let request_id = logical_id();
        let next_key = format!("{}-reconciled-{request_id}", consume_key(event));
        let command = TaskCommand::ConsumeReply {
            request_id,
            task_id: task_id.clone(),
            event_id: event_id.clone(),
            expected_revision: revision,
        };
        let tx = self.connection.transaction()?;
        insert(&tx, &next_key, Lane::Controller, &command)?;
        tx.execute("INSERT INTO runner_metadata VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [&pointer(event), &next_key])?;
        tx.commit()?;
        self.command(&next_key)?
            .context("Reconciled intent disappeared")
    }
}

fn pointer(event: &str) -> String {
    format!("consume-intent-{event}")
}

pub(super) fn consume_conflict(error: &anyhow::Error) -> bool {
    matches!(error.downcast_ref::<ClientError>(),
        Some(ClientError::Application(error)) if error.code == "task-state-conflict")
}

pub(super) fn consumed(outcome: &TaskOutcome) -> Result<()> {
    anyhow::ensure!(
        matches!(outcome, TaskOutcome::Record(_)),
        "Expected consume receipt"
    );
    Ok(())
}
