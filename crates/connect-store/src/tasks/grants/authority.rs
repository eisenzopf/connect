//! Narrow ceilings are rechecked against the immutable task and current issuers.
use super::super::{authority as tasks, storage};
use crate::{identity::actor_from_connection_at, Actor};
use connect_protocol::{
    tasks::{GrantDefinition, TaskPermission, TaskRecord},
    Error, Result,
};
use rusqlite::Connection;

pub(super) fn issuing_task(
    c: &Connection,
    actor: &Actor,
    definition: &GrantDefinition,
    timestamp: i64,
) -> Result<TaskRecord> {
    actor.require("task_grant")?;
    definition.validate(timestamp)?;
    let task = tasks::task(c, actor, &definition.task_id)?;
    tasks::continuation(c, &task, timestamp, false)?;
    if definition.expires_at > task.record.expires_at
        || definition.expires_at > actor.expires_at
        || definition.operations.iter().any(|op| {
            !task.record.scope.operations.contains(op) || !actor.permits(op.required_scope())
        })
        || task
            .record
            .scope
            .recipients
            .iter()
            .any(|id| !actor.permits_recipient(id))
    {
        return Err(Error::forbidden());
    }
    Ok(task.record)
}
pub(super) fn intent(
    definition: &GrantDefinition,
    credential_id: &str,
    task: &TaskRecord,
) -> Result<String> {
    // Bind immutable task owner/cid/ceiling as well as this invocation subset.
    storage::digest(&(
        definition,
        credential_id,
        &task.owner_id,
        &task.conversation_id,
        &task.scope,
    ))
}
pub(super) fn subset(
    c: &Connection,
    credential_id: &str,
    task: &TaskRecord,
    operations: &[TaskPermission],
    timestamp: i64,
) -> Result<Actor> {
    let actor = actor_from_connection_at(c, credential_id, timestamp)?;
    actor.require("task_grant")?;
    if actor.identity_id != task.owner_id
        || operations
            .iter()
            .any(|op| !task.scope.operations.contains(op) || !actor.permits(op.required_scope()))
        || task
            .scope
            .recipients
            .iter()
            .any(|id| !actor.permits_recipient(id))
    {
        return Err(Error::forbidden());
    }
    Ok(actor)
}
