//! Durable application task state. No media socket or host session owns its lifetime.
mod authority;
pub(crate) mod budget;
mod claims;
mod effects;
mod grants;
pub use grants::{IssuedTaskGrant, TaskInvocation};
mod inbox;
mod lifecycle;
mod native_reply;
mod owner;
mod recall;
mod replies;
mod storage;
use connect_protocol::{tasks::TaskRequest, Error, Result};

/// Qualification hooks are compiled only into explicit test-support binaries.
pub(super) fn commit(tx: rusqlite::Transaction<'_>, request: &TaskRequest) -> Result<()> {
    #[cfg(feature = "test-support")]
    if let Some(id) = request.command.request_id() {
        crate::fault_point("task-before-commit", id);
    }
    tx.commit().map_err(|_| Error::storage())?;
    #[cfg(feature = "test-support")]
    if let Some(id) = request.command.request_id() {
        crate::fault_point("task-after-commit", id);
    }
    #[cfg(not(feature = "test-support"))]
    let _ = request;
    Ok(())
}

/// Ordinary jobs have no effect link. Task jobs must retain their original
/// delegation at dispatch, even when the account-level credential still works.
pub(crate) fn authorize_effect_link(
    c: &rusqlite::Connection,
    kind: &str,
    resource: &str,
    timestamp: i64,
) -> Result<()> {
    use rusqlite::OptionalExtension;
    let id: Option<String> = c
        .query_row(
            "SELECT effect_id FROM task_effect_links WHERE kind=?1 AND resource_id=?2",
            rusqlite::params![kind, resource],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| Error::storage())?;
    if let Some(id) = id {
        effects::authorize_effect(c, &id, timestamp)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
