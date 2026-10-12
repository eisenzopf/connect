//! Lifecycle writes and attributed events share the admission transaction.
use super::{authority, storage};
use crate::{Actor, Store};
use connect_protocol::{
    tasks::{
        TaskCommand, TaskDefinition, TaskOutcome, TaskRecord, TaskRequest, TaskState,
        MAX_ACTIVE_TASKS, MAX_RETAINED_TASKS,
    },
    Error, Result,
};
use rusqlite::{params, Connection};

pub(super) fn create(
    c: &Connection,
    actor: &Actor,
    definition: &TaskDefinition,
    timestamp: i64,
    store: &Store,
) -> Result<TaskRecord> {
    authority::definition(c, actor, definition)?;
    super::sms::admit(c, definition)?;
    let (retained,active):(u32,u32)=c.query_row("SELECT COUNT(*),COALESCE(SUM(state IN ('ready','running','waiting') AND expires_at>?1),0) FROM tasks",[timestamp],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|Error::storage())?;
    if retained >= MAX_RETAINED_TASKS || active >= MAX_ACTIVE_TASKS {
        return Err(Error::new(
            "capacity-exceeded",
            "Task admission capacity exceeded",
        ));
    }
    let record = TaskRecord {
        id: uuid::Uuid::new_v4().to_string(),
        owner_id: actor.identity_id.clone(),
        conversation_id: definition.conversation_id.clone(),
        scope: definition.scope.clone(),
        state: TaskState::Ready,
        revision: 0,
        expires_at: definition.expires_at,
        initiating_event_id: definition.initiating_event_id.clone(),
        created_at: timestamp,
        updated_at: timestamp,
        reason: None,
    };
    c.execute("INSERT INTO tasks(id,owner_id,credential_id,conversation_id,scope_json,state,revision,expires_at,initiating_event_id,created_at,updated_at,reason) VALUES(?1,?2,?3,?4,?5,'ready',0,?6,?7,?8,?8,NULL)",params![record.id,record.owner_id,actor.credential_id,record.conversation_id,storage::json(&record.scope)?,record.expires_at,record.initiating_event_id,timestamp]).map_err(|_|Error::storage())?;
    storage::event(c, actor, &record, store)?;
    Ok(record)
}
pub(super) fn change(
    c: &Connection,
    actor: &Actor,
    mut record: TaskRecord,
    next: TaskState,
    reason: Option<String>,
    timestamp: i64,
    store: &Store,
) -> Result<TaskRecord> {
    record.state.transition(next)?;
    record.revision = record.revision.checked_add(1).ok_or_else(Error::storage)?;
    record.state = next;
    record.reason = reason;
    record.updated_at = timestamp;
    c.execute(
        "UPDATE tasks SET state=?1,revision=?2,reason=?3,updated_at=?4 WHERE id=?5",
        params![
            storage::state_name(next)?,
            record.revision,
            record.reason,
            timestamp,
            record.id
        ],
    )
    .map_err(|_| Error::storage())?;
    if next.is_terminal() {
        c.execute(
            "UPDATE task_grants SET revoked=1 WHERE task_id=?1",
            [&record.id],
        )
        .map_err(|_| Error::storage())?;
        c.execute("UPDATE task_effects SET state='cancelled',updated_at=?1 WHERE task_id=?2 AND state='prepared'",params![timestamp,record.id]).map_err(|_|Error::storage())?;
    }
    storage::event(c, actor, &record, store)?;
    Ok(record)
}

/// The caller already revalidated an explicit task-cancel grant inside this
/// transaction. Current authorization precedes receipt lookup; after successful
/// cancellation the revoked grant cannot replay even its accepted receipt.
pub(super) fn cancel_delegated(
    c: &Connection,
    actor: &Actor,
    task: TaskRecord,
    request: &TaskRequest,
    timestamp: i64,
    store: &Store,
) -> Result<TaskOutcome> {
    let TaskCommand::Cancel {
        request_id,
        reason,
        task_id,
    } = &request.command
    else {
        return Err(Error::forbidden());
    };
    if task_id != &task.id {
        return Err(Error::forbidden());
    }
    let intent = storage::digest(request)?;
    if let Some((saved_task, outcome)) = storage::saved(c, actor, request_id, &intent)? {
        return if saved_task == task.id {
            Ok(outcome)
        } else {
            Err(Error::forbidden())
        };
    }
    let record = change(
        c,
        actor,
        task,
        TaskState::Cancelled,
        reason.clone(),
        timestamp,
        store,
    )?;
    storage::save(c, actor, request_id, &intent, &record, store)
}
