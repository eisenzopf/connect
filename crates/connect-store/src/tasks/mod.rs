//! Durable application task state. No media socket or host session owns its lifetime.
mod authority;
mod lifecycle;
mod storage;
use crate::{identity::actor_from_connection_at, Store};
use connect_protocol::{
    tasks::{TaskCommand, TaskOutcome, TaskRequest, TaskState},
    Error, Result,
};
use rusqlite::TransactionBehavior;

impl Store {
    /// Admit a typed task command with current authority and one serialized
    /// transaction. Service/client exposure is a separate application extension.
    pub fn execute_task(&self, credential_id: &str, request: &TaskRequest) -> Result<TaskOutcome> {
        self.execute_task_at(credential_id, request, crate::now())
    }
    fn execute_task_at(
        &self,
        credential_id: &str,
        request: &TaskRequest,
        timestamp: i64,
    ) -> Result<TaskOutcome> {
        let mut connection = self.connection()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let actor = actor_from_connection_at(&tx, credential_id, timestamp)?;
        request.validate(timestamp)?;
        actor.require(request.command.required_scope())?;
        // Authority precedes receipt lookup; a saved result is never a grant.
        let current = if let Some(id) = request.command.task_id() {
            Some(authority::task(&tx, &actor, id)?)
        } else {
            None
        };
        if let TaskCommand::Create { definition, .. } = &request.command {
            authority::definition(&tx, &actor, definition)?;
        }
        if let Some(task) = &current {
            if !matches!(
                request.command,
                TaskCommand::Get { .. } | TaskCommand::Cancel { .. }
            ) {
                authority::continuation(&tx, task, timestamp, true)?;
            }
        }
        let intent = storage::digest(request)?;
        if let Some(id) = request.command.request_id() {
            if let Some((task_id, outcome)) = storage::saved(&tx, &actor, id, &intent)? {
                let saved_task = authority::task(&tx, &actor, &task_id)?;
                if !matches!(request.command, TaskCommand::Cancel { .. }) {
                    authority::continuation(&tx, &saved_task, timestamp, true)?;
                }
                return Ok(outcome);
            }
        }
        let record = match &request.command {
            TaskCommand::Create { definition, .. } => {
                lifecycle::create(&tx, &actor, definition, timestamp, self)?
            }
            TaskCommand::Get { .. } => {
                let record = current.ok_or_else(Error::forbidden)?.record;
                if !record.state.is_terminal() && record.expires_at <= timestamp {
                    lifecycle::change(
                        &tx,
                        &actor,
                        record,
                        TaskState::Expired,
                        Some("Task lifetime elapsed".into()),
                        timestamp,
                        self,
                    )?
                } else {
                    record
                }
            }
            TaskCommand::Cancel { reason, .. } => lifecycle::change(
                &tx,
                &actor,
                current.ok_or_else(Error::forbidden)?.record,
                TaskState::Cancelled,
                reason.clone(),
                timestamp,
                self,
            )?,
            TaskCommand::Transition {
                expected_revision,
                state,
                reason,
                ..
            } => {
                let task = current.ok_or_else(Error::forbidden)?;
                authority::continuation(&tx, &task, timestamp, false)?;
                let record = task.record;
                if record.revision != *expected_revision {
                    return Err(Error::new("task-state-conflict", "Task revision changed"));
                }
                lifecycle::change(&tx, &actor, record, *state, reason.clone(), timestamp, self)?
            }
            _ => {
                return Err(Error::new(
                    "unsupported-operation",
                    "Task operation is not implemented yet",
                ))
            }
        };
        let outcome = if let Some(id) = request.command.request_id() {
            storage::save(&tx, &actor, id, &intent, &record, self)?
        } else {
            TaskOutcome::Record(Box::new(record))
        };
        tx.commit().map_err(|_| Error::storage())?;
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests;
