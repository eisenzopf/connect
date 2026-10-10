//! Current owner, credential and immutable conversation checks precede receipts.
use super::storage::{load, StoredTask};
use crate::{conversations::authorized_conversation, identity::actor_from_connection_at, Actor};
use connect_protocol::{
    tasks::{TaskDefinition, TaskPermission, TaskRecord, TaskState},
    Error, Operation, Outcome, Result,
};
use rusqlite::Connection;

pub(super) fn definition(c: &Connection, actor: &Actor, definition: &TaskDefinition) -> Result<()> {
    let conversation = authorized_conversation(c, actor, &definition.conversation_id)?;
    if conversation.closed {
        return Err(Error::new("conflict", "Conversation is closed"));
    }
    if definition.expires_at > actor.expires_at
        || definition
            .scope
            .operations
            .iter()
            .any(|op| !actor.permits(op.required_scope()))
        || definition
            .scope
            .recipients
            .iter()
            .any(|id| !conversation.participants.contains(id) || !actor.permits_recipient(id))
    {
        return Err(Error::forbidden());
    }
    if let Some(event) = &definition.initiating_event_id {
        let exists:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM journal WHERE id=?1 AND conversation_id=?2 AND recipient=?3)",rusqlite::params![event,definition.conversation_id,actor.identity_id],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !exists {
            return Err(Error::forbidden());
        }
    }
    Ok(())
}
pub(super) fn task(c: &Connection, actor: &Actor, id: &str) -> Result<StoredTask> {
    let task = load(c, id)?;
    if task.record.owner_id != actor.identity_id {
        return Err(Error::forbidden());
    }
    authorized_conversation(c, actor, &task.record.conversation_id)?;
    Ok(task)
}
pub(super) fn continuation(
    c: &Connection,
    task: &StoredTask,
    timestamp: i64,
    replay: bool,
) -> Result<()> {
    if task.record.expires_at <= timestamp
        || matches!(
            task.record.state,
            TaskState::Cancelled | TaskState::Failed | TaskState::Expired
        )
        || (!replay && task.record.state == TaskState::Completed)
    {
        return Err(Error::forbidden());
    }
    let backing = actor_from_connection_at(c, &task.credential_id, timestamp)?;
    // Inspection/cancellation may use a different current owner credential,
    // but continuation cannot outlive the original delegation authority.
    definition(
        c,
        &backing,
        &TaskDefinition {
            conversation_id: task.record.conversation_id.clone(),
            scope: task.record.scope.clone(),
            expires_at: task.record.expires_at,
            initiating_event_id: task.record.initiating_event_id.clone(),
        },
    )?;
    Ok(())
}

/// Delegate only communication in the task ceiling. Reads share the approved
/// conversation context; mutations cannot publish outside its recipient ceiling.
pub(super) fn operation(
    c: &Connection,
    actor: &Actor,
    task: &TaskRecord,
    operation: &Operation,
) -> Result<()> {
    let permission = TaskPermission::for_operation(operation)?;
    if !task.scope.operations.contains(&permission) {
        return Err(Error::forbidden());
    }
    actor.require(operation.required_scope())?;
    match operation {
        Operation::Inbox { .. } | Operation::RequestStatus { .. } => return Err(Error::forbidden()),
        Operation::Resolve { address } | Operation::Capabilities { address, .. } => {
            if let Operation::Capabilities {
                conversation_id, ..
            } = operation
            {
                if conversation_id.as_deref() != Some(task.conversation_id.as_str()) {
                    return Err(Error::forbidden());
                }
            }
            let Outcome::Identity(identity) =
                crate::conversation_queries::resolve(c, actor, address)?
            else {
                return Err(Error::storage());
            };
            if identity.id != task.owner_id && !task.scope.recipients.contains(&identity.id) {
                return Err(Error::forbidden());
            }
        }
        _ => {
            if operation.conversation_id() != Some(task.conversation_id.as_str()) {
                return Err(Error::forbidden());
            }
            let conversation = authorized_conversation(c, actor, &task.conversation_id)?;
            if operation.request_id().is_some()
                && conversation
                    .participants
                    .iter()
                    .any(|id| id != &task.owner_id && !task.scope.recipients.contains(id))
            {
                return Err(Error::forbidden());
            }
        }
    }
    Ok(())
}
