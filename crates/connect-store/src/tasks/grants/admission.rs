//! Command arguments cannot select authority beyond the authenticated invocation.
use super::{revalidate, TaskInvocation};
use crate::Actor;
use connect_protocol::{
    tasks::{TaskCommand, TaskPermission, TaskRecord, TaskRequest},
    Error, Operation, Outcome, Result,
};
use rusqlite::Connection;

pub(super) fn authorize(
    c: &Connection,
    invocation: &TaskInvocation,
    request: &TaskRequest,
    timestamp: i64,
) -> Result<(Actor, TaskRecord)> {
    let (grant, actor, task) = revalidate(c, invocation, timestamp)?;
    request.validate(timestamp)?;
    if invocation.task_id != task.id || request.command.task_id() != Some(task.id.as_str()) {
        return Err(Error::forbidden());
    }
    let permits = |permission| grant.grant.operations.contains(&permission);
    match &request.command {
        TaskCommand::Get { .. } => (),
        TaskCommand::Inbox { .. } | TaskCommand::Wait { .. } if permits(TaskPermission::Inbox) => {}
        TaskCommand::Recall { .. }
            if permits(TaskPermission::History) || permits(TaskPermission::ContextView) => {}
        TaskCommand::EffectGet { effect_id, .. } if permits(TaskPermission::RequestStatus) => {
            let operation: String = c
                .query_row(
                    "SELECT operation_json FROM task_effects WHERE id=?1 AND task_id=?2",
                    rusqlite::params![effect_id, task.id],
                    |r| r.get(0),
                )
                .map_err(|_| Error::forbidden())?;
            let operation: Operation =
                serde_json::from_str(&operation).map_err(|_| Error::storage())?;
            if !permits(TaskPermission::for_operation(&operation)?) {
                return Err(Error::forbidden());
            }
        }
        TaskCommand::Invoke { operation, .. } => {
            if !permits(TaskPermission::for_operation(operation)?) {
                return Err(Error::forbidden());
            }
            actor.require(operation.required_scope())?;
            match operation.as_ref() {
                // Generic account inbox/receipt lookup would disclose other tasks.
                Operation::Inbox { .. } | Operation::RequestStatus { .. } => {
                    return Err(Error::forbidden())
                }
                Operation::Resolve { address } | Operation::Capabilities { address, .. } => {
                    if let Operation::Capabilities {
                        conversation_id, ..
                    } = operation.as_ref()
                    {
                        if conversation_id.as_deref() != Some(task.conversation_id.as_str()) {
                            return Err(Error::forbidden());
                        }
                    }
                    let identity = match crate::conversation_queries::resolve(c, &actor, address)? {
                        Outcome::Identity(identity) => identity,
                        _ => return Err(Error::storage()),
                    };
                    if identity.id != task.owner_id && !task.scope.recipients.contains(&identity.id)
                    {
                        return Err(Error::forbidden());
                    }
                }
                _ => {
                    if operation.conversation_id() != Some(task.conversation_id.as_str()) {
                        return Err(Error::forbidden());
                    }
                    let conversation = crate::conversations::authorized_conversation(
                        c,
                        &actor,
                        &task.conversation_id,
                    )?;
                    // Existing sends publish to the immutable room. Do not quietly
                    // contact a participant outside this task's recipient ceiling.
                    if conversation
                        .participants
                        .iter()
                        .any(|id| id != &task.owner_id && !task.scope.recipients.contains(id))
                    {
                        return Err(Error::forbidden());
                    }
                }
            }
        }
        _ => return Err(Error::forbidden()),
    }
    Ok((actor, task))
}
