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
        let exists:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM visible_journal WHERE id=?1 AND conversation_id=?2 AND recipient=?3)",rusqlite::params![event,definition.conversation_id,actor.identity_id],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !exists {
            return Err(Error::forbidden());
        }
    }
    Ok(())
}
pub(super) fn task(c: &Connection, actor: &Actor, id: &str) -> Result<StoredTask> {
    let task = load(c, id)?;
    if task.record.owner_id != actor.identity_id
        || task
            .record
            .scope
            .recipients
            .iter()
            .any(|id| !actor.permits_recipient(id))
    {
        return Err(Error::forbidden());
    }
    authorized_conversation(c, actor, &task.record.conversation_id)?;
    let epoch: u64 = c
        .query_row(
            "SELECT owner_membership_epoch FROM tasks WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if crate::audience::member(c, &actor.identity_id, &task.record.conversation_id, true)?
        .membership_epoch
        != epoch
    {
        return Err(Error::forbidden());
    }
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
            if let Operation::Send {
                conversation_id, ..
            }
            | Operation::ArtifactBegin {
                conversation_id, ..
            }
            | Operation::ArtifactAppend {
                conversation_id, ..
            }
            | Operation::ArtifactLinkAppend {
                conversation_id, ..
            }
            | Operation::ArtifactFinish {
                conversation_id, ..
            }
            | Operation::ArtifactDelete {
                conversation_id, ..
            } = operation
            {
                authorized_conversation(c, actor, conversation_id)?;
                return Ok(());
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

/// New fanout resolves current members. Retained fanout checks its original
/// accepted source instead of expanding an old `all` decision after late joins.
pub(super) fn fanout_audience(
    c: &Connection,
    actor: &Actor,
    task: &TaskRecord,
    operation: &Operation,
    accepted: Option<&Outcome>,
) -> Result<()> {
    use connect_protocol::membership::ContentKind;
    let (cid, to, kind, existing) = match operation {
        Operation::Send {
            conversation_id,
            message,
            ..
        } => (
            conversation_id,
            Some(&message.to),
            ContentKind::Message,
            None,
        ),
        Operation::ArtifactBegin {
            conversation_id,
            to,
            ..
        } => (conversation_id, Some(to), ContentKind::Artifact, None),
        Operation::ArtifactAppend {
            conversation_id,
            artifact_id,
            ..
        }
        | Operation::ArtifactLinkAppend {
            conversation_id,
            artifact_id,
            ..
        }
        | Operation::ArtifactFinish {
            conversation_id,
            artifact_id,
            ..
        }
        | Operation::ArtifactDelete {
            conversation_id,
            artifact_id,
            ..
        } => (
            conversation_id,
            None,
            ContentKind::Artifact,
            Some(artifact_id),
        ),
        _ => return Ok(()),
    };
    let source = match accepted {
        Some(Outcome::Accepted { message_id, .. }) if kind == ContentKind::Message => {
            Some(message_id.as_str())
        }
        Some(Outcome::Artifact(v)) if kind == ContentKind::Artifact => Some(v.id.as_str()),
        _ => None,
    };
    let members = if let Some(id) = source {
        crate::audience::require_original(c, actor, cid, kind, id)?;
        let mut st=c.prepare("SELECT DISTINCT p.identity_id FROM original_content_audiences a JOIN participants p ON p.participant_id=a.participant_id AND p.conversation_id=a.conversation_id WHERE a.kind=?1 AND a.id=?2 AND a.conversation_id=?3 ORDER BY p.identity_id").map_err(|_|Error::storage())?;
        let values = st
            .query_map(rusqlite::params![kind.as_str(), id, cid], |r| r.get(0))
            .map_err(|_| Error::storage())?
            .collect::<rusqlite::Result<Vec<String>>>()
            .map_err(|_| Error::storage())?;
        values
    } else if let Some(id) = existing {
        crate::audience::require_original(c, actor, cid, kind, id)?;
        crate::audience::source_members(c, cid, kind, id)?
    } else {
        // Validate message references too before preparing a new task effect.
        match operation {
            Operation::Send { message, .. } => crate::messages::recipients(c, actor, cid, message)?,
            _ => crate::audience::select_members(c, actor, cid, to.ok_or_else(Error::forbidden)?)?,
        }
        .into_iter()
        .map(|p| p.identity_id)
        .collect()
    };
    if members.iter().any(|id| {
        !actor.permits_recipient(id)
            || (id != &task.owner_id && !task.scope.recipients.contains(id))
    }) {
        return Err(Error::forbidden());
    }
    Ok(())
}
