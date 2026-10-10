//! Provider turns derive context and replies from committed task effect metadata.
//! Route-wide provider history never supplies task invocation authority.
use super::rows::{self, StoredEffect};
use crate::{tasks, Actor, Store};
use connect_protocol::{
    tasks::{TaskOutcome, TaskPermission},
    Error, Operation, Outcome, Result,
};
use rusqlite::Connection;

/// `Restricted` is a task send without permission to export history, not an
/// invitation to fall back to the broader backing account's context.
pub(crate) enum Context {
    Ordinary,
    Restricted,
    Selected { json: String, watermark: u64 },
}

fn effect(c: &Connection, job: &str) -> Result<Option<StoredEffect>> {
    let mut query = c
        .prepare(
            "SELECT e.id,e.task_id,j.message_id FROM task_effect_links l
         JOIN task_effects e ON e.id=l.effect_id JOIN provider_jobs j ON j.id=l.resource_id
         WHERE l.kind='provider_job' AND l.resource_id=?1 LIMIT 2",
        )
        .map_err(|_| Error::storage())?;
    let found = query
        .query_map([job], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    if found.is_empty() {
        return Ok(None);
    }
    let [(id, task, message)] = found.as_slice() else {
        return Err(Error::forbidden());
    };
    let saved = rows::load(c, id, task)?;
    rows::verify(&saved)?;
    let (
        Operation::Send {
            conversation_id, ..
        },
        Some(outcome),
    ) = (&saved.operation, &saved.effect.outcome)
    else {
        return Err(Error::forbidden());
    };
    if !matches!(outcome.as_ref(), Outcome::Accepted { message_id, conversation_id: cid, .. }
        if message_id == message && cid == conversation_id)
    {
        return Err(Error::forbidden());
    }
    Ok(Some(saved))
}

pub(crate) fn context(c: &Connection, job: &str, store: &Store) -> Result<Context> {
    let Some(saved) = effect(c, job)? else {
        return Ok(Context::Ordinary);
    };
    let timestamp = crate::now();
    super::dispatch::authorize_effect(c, &saved.effect.id, timestamp)?;
    let (actor, task) = if let Some(grant) = &saved.grant_id {
        tasks::grants::effect_context_authority(c, grant, timestamp)?
    } else {
        let credential = saved
            .credential_id
            .as_deref()
            .ok_or_else(Error::forbidden)?;
        let actor = crate::identity::actor_from_connection_at(c, credential, timestamp)?;
        let task = tasks::authority::task(c, &actor, &saved.effect.task_id)?.record;
        (actor, task)
    };
    if ![TaskPermission::History, TaskPermission::ContextView]
        .iter()
        .any(|p| task.scope.operations.contains(p) && actor.permits(p.required_scope()))
    {
        return Ok(Context::Restricted);
    }
    let selection = tasks::recall::read(
        c,
        &actor,
        &task,
        None,
        store.limits.page_size.min(100),
        timestamp,
        store,
    )?;
    let TaskOutcome::Recall { next, .. } = &selection else {
        return Err(Error::storage());
    };
    let watermark = crate::cursor::decode(
        c,
        &actor.identity_id,
        &format!("task-recall:{}", task.id),
        next.as_deref(),
    )?;
    Ok(Context::Selected {
        json: serde_json::to_string(&selection).map_err(|_| Error::storage())?,
        watermark: watermark as u64,
    })
}

/// The provider response answers the exact delivered question. Its sender uses
/// current route authority; a revoked invocation does not erase incoming speech.
/// Terminal task handling and inbox limits remain in the shared reply transaction.
pub(crate) fn reply(
    c: &Connection,
    actor: &Actor,
    job: &str,
    body: &str,
    store: &Store,
) -> Result<Option<String>> {
    let Some(saved) = effect(c, job)? else {
        return Ok(None);
    };
    actor.require("send")?;
    let Some(outcome) = &saved.effect.outcome else {
        return Err(Error::forbidden());
    };
    let Outcome::Accepted {
        message_id,
        conversation_id,
        ..
    } = outcome.as_ref()
    else {
        return Err(Error::forbidden());
    };
    let request_id = format!("provider-reply:{job}");
    let intent = tasks::storage::digest(&(
        "provider-task-reply/1",
        job,
        message_id,
        conversation_id,
        body,
    ))?;
    let outcome = tasks::native_reply::execute(
        c,
        actor,
        tasks::native_reply::Input {
            request_id: &request_id,
            conversation_id,
            reply_to: message_id,
            body,
        },
        &intent,
        crate::now(),
        store,
    )?;
    let TaskOutcome::ReplyAccepted { message_id, .. } = outcome else {
        return Err(Error::storage());
    };
    Ok(Some(message_id))
}
