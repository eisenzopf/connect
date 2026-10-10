//! Owner-authorized task admission and retained control receipt inspection.
use super::{
    authority, claims, clock::Clock, commit, effects, inbox, lifecycle, native_reply, recall,
    replies, storage,
};
use crate::{identity::actor_from_connection_at, DiscoveryRuntime, Store};
use connect_protocol::{
    tasks::{TaskCommand, TaskOutcome, TaskRequest, TaskState},
    Error, Result,
};
use rusqlite::TransactionBehavior;

impl Store {
    /// Admit a typed task command with current authority and one serialized
    /// transaction. Service/client exposure is a separate application extension.
    pub fn execute_task(&self, credential_id: &str, request: &TaskRequest) -> Result<TaskOutcome> {
        self.execute_task_with_runtime(credential_id, request, &DiscoveryRuntime::default())
    }
    /// Use configured discovery rather than a synthetic default for nested tools.
    pub fn execute_task_with_runtime(
        &self,
        credential_id: &str,
        request: &TaskRequest,
        runtime: &DiscoveryRuntime,
    ) -> Result<TaskOutcome> {
        self.execute_task_runtime_at(credential_id, request, runtime, Clock::Current)
    }
    #[cfg(test)]
    pub(in crate::tasks) fn execute_task_at(
        &self,
        credential_id: &str,
        request: &TaskRequest,
        timestamp: i64,
    ) -> Result<TaskOutcome> {
        self.execute_task_runtime_at(
            credential_id,
            request,
            &DiscoveryRuntime::default(),
            Clock::At(timestamp),
        )
    }
    fn execute_task_runtime_at(
        &self,
        credential_id: &str,
        request: &TaskRequest,
        runtime: &DiscoveryRuntime,
        clock: Clock,
    ) -> Result<TaskOutcome> {
        let mut connection = self.connection()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let timestamp = clock.read();
        let actor = actor_from_connection_at(&tx, credential_id, timestamp)?;
        request.validate(timestamp)?;
        actor.require(request.command.required_scope())?;
        if let TaskCommand::Reply {
            request_id,
            conversation_id,
            reply_to,
            body,
        } = &request.command
        {
            let outcome = native_reply::execute(
                &tx,
                &actor,
                native_reply::Input {
                    request_id,
                    conversation_id,
                    reply_to,
                    body,
                },
                &storage::digest(request)?,
                timestamp,
                self,
            )?;
            commit(tx, request)?;
            return Ok(outcome);
        }
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
                TaskCommand::Get { .. }
                    | TaskCommand::Cancel { .. }
                    | TaskCommand::EffectGet { .. }
            ) && !terminal_transition(&request.command)
            {
                authority::continuation(&tx, task, timestamp, true)?;
            }
            match &request.command {
                TaskCommand::AssignReply { event_id, .. } => {
                    replies::authorize(&tx, &actor, &task.record, event_id, false)?;
                }
                TaskCommand::ConsumeReply { event_id, .. } => {
                    replies::authorize(&tx, &actor, &task.record, event_id, true)?;
                }
                _ => (),
            }
        }
        match &request.command {
            TaskCommand::Claim { .. }
            | TaskCommand::ClaimGet { .. }
            | TaskCommand::ReleaseClaim { .. } => {
                let task = current.ok_or_else(Error::forbidden)?;
                let outcome = claims::execute(&tx, &actor, &task.record, request, timestamp, self)?;
                commit(tx, request)?;
                return Ok(outcome);
            }
            TaskCommand::Invoke { .. } => {
                let task = current.ok_or_else(Error::forbidden)?;
                let result = effects::invoke(
                    effects::Execution {
                        c: &tx,
                        actor: &actor,
                        task: &task.record,
                        store: self,
                        runtime,
                        timestamp,
                    },
                    request,
                    None,
                )?;
                commit(tx, request)?;
                return result;
            }
            TaskCommand::EffectGet { effect_id, .. } => {
                let task = current.ok_or_else(Error::forbidden)?;
                let result = effects::get(&tx, &actor, &task.record, effect_id, timestamp, self)?;
                commit(tx, request)?;
                return Ok(result);
            }
            TaskCommand::Replies {
                conversation_id,
                after,
                limit,
            } => {
                return replies::list(&tx, &actor, conversation_id, after.as_deref(), *limit, self);
            }
            TaskCommand::Inbox { after, limit, .. } | TaskCommand::Wait { after, limit, .. } => {
                let task = current.ok_or_else(Error::forbidden)?;
                return inbox::read(&tx, &actor, &task.record, after.as_deref(), *limit, self);
            }
            TaskCommand::Recall { after, limit, .. } => {
                let task = current.ok_or_else(Error::forbidden)?;
                return recall::read(
                    &tx,
                    &actor,
                    &task.record,
                    after.as_deref(),
                    *limit,
                    timestamp,
                    self,
                );
            }
            _ => (),
        }
        let intent = storage::digest(request)?;
        if let Some(id) = request.command.request_id() {
            if let Some((task_id, outcome)) = storage::saved(&tx, &actor, id, &intent)? {
                let saved_task = authority::task(&tx, &actor, &task_id)?;
                if !matches!(request.command, TaskCommand::Cancel { .. })
                    && !terminal_transition(&request.command)
                {
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
            TaskCommand::AssignReply {
                event_id, reply_to, ..
            } => replies::assign(
                &tx,
                &actor,
                current.ok_or_else(Error::forbidden)?.record,
                event_id,
                reply_to.as_deref(),
                timestamp,
                self,
            )?,
            TaskCommand::ConsumeReply {
                event_id,
                expected_revision,
                ..
            } => replies::consume(
                &tx,
                &actor,
                current.ok_or_else(Error::forbidden)?.record,
                event_id,
                *expected_revision,
                timestamp,
                self,
            )?,
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
        commit(tx, request)?;
        Ok(outcome)
    }
}

// A matching terminal-control receipt is inspectable under current owner/ACL
// and task_transition scope. A new transition still goes through continuation
// below, so receipt inspection cannot reopen work or authorize another effect.
fn terminal_transition(command: &TaskCommand) -> bool {
    matches!(command, TaskCommand::Transition { state, .. } if state.is_terminal())
}
