//! Command arguments cannot select authority beyond the authenticated invocation.
use super::{revalidate, TaskInvocation};
use crate::Actor;
use connect_protocol::{
    tasks::{TaskCommand, TaskPermission, TaskRecord, TaskRequest},
    Error, Operation, Result,
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
            super::super::authority::operation(c, &actor, &task, operation)?;
        }
        _ => return Err(Error::forbidden()),
    }
    Ok((actor, task))
}
