//! Fixed SQL projection and durable receipts; never bearer plaintext.
use crate::{Actor, Store};
use connect_protocol::{
    tasks::{TaskEventData, TaskOutcome, TaskRecord, TaskState},
    Error, Result,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Serialize};
use sha2::{Digest, Sha256};

pub(super) struct StoredTask {
    pub record: TaskRecord,
    pub credential_id: String,
}
pub(super) fn json<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(|_| Error::storage())
}
fn decode<T: DeserializeOwned>(value: String) -> rusqlite::Result<T> {
    serde_json::from_str(&value).map_err(|_| rusqlite::Error::InvalidQuery)
}
pub(super) fn state_name(state: TaskState) -> Result<String> {
    serde_json::from_value(serde_json::to_value(state).map_err(|_| Error::storage())?)
        .map_err(|_| Error::storage())
}
pub(super) fn load(c: &Connection, id: &str) -> Result<StoredTask> {
    c.query_row(
        "SELECT id, owner_id, credential_id, conversation_id, scope_json, state,
                revision, expires_at, initiating_event_id, created_at, updated_at, reason
         FROM tasks WHERE id=?1",
        [id],
        |row| {
            let state = serde_json::from_value(serde_json::Value::String(row.get(5)?))
                .map_err(|_| rusqlite::Error::InvalidQuery)?;
            Ok(StoredTask {
                credential_id: row.get(2)?,
                record: TaskRecord {
                    id: row.get(0)?,
                    owner_id: row.get(1)?,
                    conversation_id: row.get(3)?,
                    scope: decode(row.get(4)?)?,
                    state,
                    revision: row.get(6)?,
                    expires_at: row.get(7)?,
                    initiating_event_id: row.get(8)?,
                    created_at: row.get(9)?,
                    updated_at: row.get(10)?,
                    reason: row.get(11)?,
                },
            })
        },
    )
    .optional()
    .map_err(|_| Error::storage())?
    .ok_or_else(Error::forbidden)
}
pub(super) fn event(
    c: &Connection,
    actor: &Actor,
    record: &TaskRecord,
    store: &Store,
) -> Result<()> {
    let data = TaskEventData::State {
        state: record.state,
        revision: record.revision,
        reason: record.reason.clone(),
    };
    event_data(c, actor, record, &data, store)
}
pub(super) fn event_data(
    c: &Connection,
    actor: &Actor,
    record: &TaskRecord,
    data: &TaskEventData,
    store: &Store,
) -> Result<()> {
    let count: u64 = c
        .query_row(
            "SELECT (SELECT COUNT(*) FROM journal)+(SELECT COUNT(*) FROM task_events)",
            [],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if !super::budget::journal(c, store.limits.journal_events, count, 1)? {
        return Err(Error::new(
            "capacity-exceeded",
            "Task journal capacity exceeded; no acceptance recorded",
        ));
    }
    c.execute(
        "INSERT INTO task_events(id,task_id,actor_id,created_at,data_json) VALUES(?1,?2,?3,?4,?5)",
        params![
            uuid::Uuid::new_v4().to_string(),
            record.id,
            actor.identity_id,
            record.updated_at,
            json(data)?
        ],
    )
    .map_err(|_| Error::storage())?;
    Ok(())
}
pub(super) fn digest<T: Serialize>(value: &T) -> Result<String> {
    let mut canonical = serde_json::to_value(value).map_err(|_| Error::storage())?;
    if canonical.get("task_profile").is_some()
        && canonical
            .pointer("/command/action")
            .and_then(|v| v.as_str())
            == Some("invoke")
    {
        if let Some(operation) = canonical.pointer_mut("/command/operation") {
            *operation = crate::artifact_links::canonical_intent(operation.take());
        }
    }
    Ok(format!(
        "task-v1:{}",
        hex::encode(Sha256::digest(json(&canonical)?.as_bytes()))
    ))
}
pub(super) fn saved(
    c: &Connection,
    actor: &Actor,
    id: &str,
    intent: &str,
) -> Result<Option<(String, TaskOutcome)>> {
    let row: Option<(String, String, String)> = c
        .query_row(
            "SELECT task_id,digest,result_json FROM task_requests WHERE actor_id=?1 AND id=?2",
            params![actor.identity_id, id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|_| Error::storage())?;
    match row {
        Some((task_id, previous, result)) => {
            if previous != intent {
                return Err(Error::new(
                    "request-conflict",
                    "Logical task request already has different inputs",
                ));
            }
            Ok(Some((
                task_id,
                serde_json::from_str(&result).map_err(|_| Error::storage())?,
            )))
        }
        None => Ok(None),
    }
}
pub(super) fn save(
    c: &Connection,
    actor: &Actor,
    id: &str,
    intent: &str,
    record: &TaskRecord,
    store: &Store,
) -> Result<TaskOutcome> {
    let outcome = TaskOutcome::Record(Box::new(record.clone()));
    save_outcome(c, actor, id, intent, &record.id, &outcome, store)?;
    Ok(outcome)
}
pub(super) fn save_outcome(
    c: &Connection,
    actor: &Actor,
    id: &str,
    intent: &str,
    task_id: &str,
    outcome: &TaskOutcome,
    store: &Store,
) -> Result<()> {
    let count: u64 = c
        .query_row(
            "SELECT (SELECT COUNT(*) FROM operations)+(SELECT COUNT(*) FROM task_requests)",
            [],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if !super::budget::receipts(c, store.limits.operation_receipts, count, 1)? {
        return Err(Error::new(
            "capacity-exceeded",
            "Task receipt capacity exceeded; no acceptance recorded",
        ));
    }
    c.execute(
        "INSERT INTO task_requests(actor_id,id,task_id,digest,result_json) VALUES(?1,?2,?3,?4,?5)",
        params![actor.identity_id, id, task_id, intent, json(outcome)?],
    )
    .map_err(|_| Error::storage())?;
    Ok(())
}
