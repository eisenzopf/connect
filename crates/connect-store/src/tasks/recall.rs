//! Source-backed conversation recall. A cursor is a checkpoint, never a grant.
mod projection;
use super::storage;
use crate::{conversations::page_limit, cursor, Actor, Store};
use connect_protocol::{
    tasks::{TaskOutcome, TaskPermission, TaskRecord, TaskSource, TASK_CONTEXT_BYTES},
    Error, Result,
};
use rusqlite::{params, Connection};

// Match the established initial context selection; non-source notifications are
// not fabricated into facts. Each event keeps its own journal provenance.
const SOURCE_FILTER: &str = "recipient=?1 AND conversation_id=?2 AND
    (message_id IS NOT NULL OR session_json IS NOT NULL OR artifact_json IS NOT NULL OR (?6=1 AND kind IN ('record_ready','record_history_shared') AND source_kind='record'))
    AND (source_owner IS NULL OR source_owner=?1 OR EXISTS(SELECT 1 FROM json_each(?5) WHERE value IN(source_owner,'*')))";

pub(super) fn read(
    c: &Connection,
    actor: &Actor,
    task: &TaskRecord,
    after: Option<&str>,
    limit: u32,
    timestamp: i64,
    store: &Store,
) -> Result<TaskOutcome> {
    if ![TaskPermission::History, TaskPermission::ContextView]
        .iter()
        .any(|p| task.scope.operations.contains(p) && actor.permits(p.required_scope()))
    {
        return Err(Error::forbidden());
    }
    crate::conversations::authorized_conversation(c, actor, &task.conversation_id)?;
    page_limit(limit, store.limits.page_size)?;
    let resource = format!("task-recall:{}", task.id);
    let position = cursor::decode(c, &actor.identity_id, &resource, after)?;
    let maximum: i64 = c
        .query_row("SELECT COALESCE(MAX(sequence),0) FROM journal", [], |r| {
            r.get(0)
        })
        .map_err(|_| Error::storage())?;
    if position > maximum {
        return Err(Error::new(
            "invalid-request",
            "Cursor is newer than this source snapshot",
        ));
    }
    let record_context =
        task.scope.operations.contains(&TaskPermission::RecordGet) && actor.permits("record_get");
    let filter = format!("{SOURCE_FILTER} AND {}", crate::audience::journal_scope(5));
    let total: u64 = c
        .query_row(
            &format!("SELECT COUNT(*) FROM visible_journal WHERE {filter} AND sequence>?3"),
            params![
                actor.identity_id,
                task.conversation_id,
                position,
                limit,
                serde_json::to_string(&actor.scope.recipients).map_err(|_| Error::storage())?,
                record_context
            ],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let mut statement = c
        .prepare(&format!(
            "SELECT sequence,id,kind,message_id,
                    json_extract(session_json,'$.id'),json_extract(artifact_json,'$.id'),CASE WHEN source_kind='record' THEN source_id END
             FROM visible_journal WHERE {filter} AND sequence>?3 ORDER BY sequence LIMIT ?4"
        ))
        .map_err(|_| Error::storage())?;
    let rows = statement
        .query_map(
            params![
                actor.identity_id,
                task.conversation_id,
                position,
                limit,
                serde_json::to_string(&actor.scope.recipients).map_err(|_| Error::storage())?,
                record_context
            ],
            |r| {
                Ok(projection::SourceRow {
                    sequence: r.get(0)?,
                    event_id: r.get(1)?,
                    kind: r.get(2)?,
                    message_id: r.get(3)?,
                    session_id: r.get(4)?,
                    artifact_id: r.get(5)?,
                    record_id: r.get(6)?,
                })
            },
        )
        .map_err(|_| Error::storage())?;
    let mut sources = Vec::new();
    let mut next = after.map(str::to_owned);
    let mut record_budget = 32 * 1024 * 1024;
    for row in rows {
        let row = row.map_err(|_| Error::storage())?;
        let candidate_cursor = Some(cursor::encode(
            c,
            &actor.identity_id,
            &resource,
            row.sequence,
        )?);
        let mut source =
            projection::read(c, actor, task, &row, timestamp, store, &mut record_budget)?;
        // An individually oversized source must not trap the reader forever at
        // the same checkpoint. Preserve its reference and explicit absence.
        if page_bytes(std::slice::from_ref(&source), &candidate_cursor, total)? > TASK_CONTEXT_BYTES
        {
            source.state = "unavailable".into();
            source.body = None;
            if let Some(record) = &mut source.record {
                record.state = connect_protocol::ContextRecordState::Unavailable;
                record.content_hash = None;
                record.valid_until = None;
            }
            source.provenance.push_str(";body-exceeds-task-budget");
        }
        sources.push(source);
        let omitted = total.saturating_sub(sources.len() as u64);
        if page_bytes(&sources, &candidate_cursor, omitted)? > TASK_CONTEXT_BYTES {
            sources.pop();
            if sources.is_empty() {
                return Err(Error::new(
                    "capacity-exceeded",
                    "Source metadata exceeds task page budget",
                ));
            }
            break;
        }
        next = candidate_cursor;
    }
    Ok(TaskOutcome::Recall {
        omitted: total.saturating_sub(sources.len() as u64),
        sources,
        next,
    })
}

fn page_bytes(sources: &[TaskSource], next: &Option<String>, omitted: u64) -> Result<usize> {
    storage::json(&TaskOutcome::Recall {
        sources: sources.to_vec(),
        next: next.clone(),
        omitted,
    })
    .map(|v| v.len())
}
