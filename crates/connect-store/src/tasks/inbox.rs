//! Paged journal catch-up. Cursors are authenticated, task-specific checkpoints.
use super::storage;
use crate::{conversations::page_limit, cursor, Actor, Store};
use connect_protocol::{
    tasks::{TaskEvent, TaskEventData, TaskOutcome, TaskRecord, TASK_CONTEXT_BYTES},
    Error, Result,
};
use rusqlite::{params, Connection};

pub(super) fn read(
    c: &Connection,
    actor: &Actor,
    task: &TaskRecord,
    after: Option<&str>,
    limit: u32,
    store: &Store,
) -> Result<TaskOutcome> {
    page_limit(limit, store.limits.page_size)?;
    let resource = format!("task-inbox:{}", task.id);
    let position = cursor::decode(c, &actor.identity_id, &resource, after)?;
    let maximum: i64 = c
        .query_row(
            "SELECT COALESCE(MAX(sequence),0) FROM task_events",
            [],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if position > maximum {
        return Err(Error::new(
            "invalid-request",
            "Cursor is newer than this task snapshot",
        ));
    }
    let mut statement = c.prepare("SELECT sequence,id,actor_id,created_at,data_json FROM task_events WHERE task_id=?1 AND sequence>?2 ORDER BY sequence LIMIT ?3").map_err(|_| Error::storage())?;
    let rows = statement
        .query_map(params![task.id, position, limit], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
            ))
        })
        .map_err(|_| Error::storage())?;
    let mut events = Vec::new();
    let mut next = after.map(str::to_owned);
    let mut bytes = 0;
    for row in rows {
        let (sequence, id, actor_id, created_at, data) = row.map_err(|_| Error::storage())?;
        let mut data: TaskEventData = serde_json::from_str(&data).map_err(|_| Error::storage())?;
        // Inbox projects effect metadata. The explicitly authorized effect read
        // retrieves outcomes; large artifact chunks cannot stall catch-up here.
        if let TaskEventData::Effect(effect) = &mut data {
            effect.outcome = None;
        }
        let event = TaskEvent {
            id,
            task_id: task.id.clone(),
            actor_id,
            sequence: sequence.try_into().map_err(|_| Error::storage())?,
            created_at,
            data,
        };
        let length = storage::json(&event)?.len();
        if bytes + length > TASK_CONTEXT_BYTES {
            break;
        }
        bytes += length;
        next = Some(cursor::encode(c, &actor.identity_id, &resource, sequence)?);
        events.push(event);
    }
    Ok(TaskOutcome::Inbox { events, next })
}
