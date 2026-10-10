//! Durable application task state. No media socket or host session owns its lifetime.
mod authority;
pub(crate) mod budget;
mod claims;
mod clock;
mod effects;
mod grants;
pub use grants::{IssuedTaskGrant, NativeTaskAuthority, TaskInvocation};
mod inbox;
mod lifecycle;
mod media_context;
pub use media_context::VapiSessionContext;
mod native_reply;
mod owner;
mod recall;
mod replies;
mod sms;
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
    } else if let Some(operation) = match kind {
        "call" => Some("call_start"),
        "sip_leg" => Some("sip_dial"),
        _ => None,
    } {
        let legacy: bool = c.query_row(
            "SELECT EXISTS(SELECT 1 FROM task_effects WHERE json_extract(operation_json,'$.op')=?1 AND json_extract(outcome_json,'$.data.id')=?2)",
            rusqlite::params![operation, resource], |r| r.get(0),
        ).map_err(|_| Error::storage())?;
        if legacy {
            return Err(Error::forbidden());
        }
    }
    Ok(())
}
pub(crate) use effects::authorize_effect;
pub(crate) use effects::{provider_task_context, provider_task_reply, ProviderTaskContext};

#[cfg(test)]
mod tests;

/// Match only a task's explicitly reviewed SMS purpose. Ordinary jobs retain
/// their existing eligibility path; enrollment creation remains authority-owned.
pub(crate) fn task_sms_decision(
    c: &rusqlite::Connection,
    job: &str,
    phone: &str,
    sender: &str,
    conversation: &str,
) -> connect_protocol::Result<Option<std::result::Result<String, &'static str>>> {
    sms::dispatch(c, job, phone, sender, conversation)
}
