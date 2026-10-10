//! Current owner, credential and immutable conversation checks precede receipts.
use super::storage::{load, StoredTask};
use crate::{conversations::authorized_conversation, identity::actor_from_connection_at, Actor};
use connect_protocol::{
    tasks::{TaskDefinition, TaskState},
    Error, Result,
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
