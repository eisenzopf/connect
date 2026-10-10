//! Project authorized journal references; never include file bytes or server paths.
use super::super::storage;
use crate::Store;
use connect_protocol::{
    tasks::{TaskRecord, TaskSource, TASK_CONTEXT_BYTES},
    Error, Result,
};
use rusqlite::{params, Connection, OptionalExtension};

pub(super) struct SourceRow {
    pub sequence: i64,
    pub event_id: String,
    pub kind: String,
    pub message_id: Option<String>,
    pub session_id: Option<String>,
    pub artifact_id: Option<String>,
}
pub(super) fn read(
    c: &Connection,
    task: &TaskRecord,
    row: &SourceRow,
    timestamp: i64,
    store: &Store,
) -> Result<TaskSource> {
    let source_id = row
        .message_id
        .as_ref()
        .or(row.session_id.as_ref())
        .or(row.artifact_id.as_ref())
        .ok_or_else(Error::storage)?;
    let mut source = TaskSource {
        source_id: source_id.clone(),
        kind: row.kind.clone(),
        actor_id: String::new(),
        created_at: None,
        provenance: format!("conversation-journal:{}", row.event_id),
        state: "unavailable".into(),
        body: None,
    };
    if let Some(id) = &row.message_id {
        let message: Option<(String, Option<String>)> = c
            .query_row(
                "SELECT sender,CASE WHEN length(CAST(body AS BLOB))<=?3 THEN body END
             FROM messages WHERE id=?1 AND conversation_id=?2",
                params![id, task.conversation_id, TASK_CONTEXT_BYTES],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|_| Error::storage())?;
        if let Some((actor, body)) = message {
            source.actor_id = actor;
            source.state = if body.is_some() {
                "available"
            } else {
                "unavailable"
            }
            .into();
            if body.is_none() {
                source.provenance.push_str(";body-exceeds-task-budget");
            }
            source.body = body;
        }
        // The message schema has no timestamp. None is evidence of that absence,
        // not an estimate from query time or the task's creation date.
        source.provenance.push_str(";timestamp-not-recorded");
    } else if let Some(id) = &row.session_id {
        match crate::sessions::get(c, &task.conversation_id, id) {
            Ok(session) => {
                source.actor_id = session.initiator_id.clone();
                source.created_at = Some(session.created_at);
                source.body = Some(storage::json(&session)?);
                source.state = "available".into();
                source.provenance.push_str(";current-session-metadata");
            }
            Err(e) if e.code == "forbidden" => (),
            Err(e) => return Err(e),
        }
    } else if let Some(id) = &row.artifact_id {
        match crate::artifacts::metadata(c, &task.conversation_id, id) {
            Ok(artifact) => {
                source.actor_id = artifact.owner_id.clone();
                source.created_at = Some(artifact.created_at);
                source
                    .provenance
                    .push_str(";current-artifact-metadata;bytes-require-artifact-get");
                source.state = if artifact.state == "deleted" {
                    "deleted"
                } else if artifact.expires_at.is_some_and(|e| e <= timestamp) {
                    "expired"
                } else if artifact.state == "ready" && blob_available(c, &artifact, store)? {
                    "available"
                } else {
                    "unavailable"
                }
                .into();
                if source.state == "available" {
                    source.body = Some(storage::json(&artifact)?);
                }
            }
            Err(e) if e.code == "forbidden" => (),
            Err(e) => return Err(e),
        }
    }
    Ok(source)
}

fn blob_available(
    c: &Connection,
    artifact: &connect_protocol::Artifact,
    store: &Store,
) -> Result<bool> {
    let blob: Option<String> = c
        .query_row(
            "SELECT blob_name FROM artifacts WHERE id=?1 AND conversation_id=?2",
            params![artifact.id, artifact.conversation_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let Some(blob) = blob else { return Ok(false) };
    if !crate::artifacts::valid_blob_name(&blob) {
        return Ok(false);
    }
    // Symlinks and missing/wrong-size files are explicit unavailability. Recall
    // never reads the bytes; authorized chunk retrieval owns its own checks.
    Ok(std::fs::symlink_metadata(store.blob_directory.join(blob))
        .is_ok_and(|m| m.file_type().is_file() && m.len() == artifact.size))
}
