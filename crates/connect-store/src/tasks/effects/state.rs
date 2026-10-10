//! Source-backed effect observations never return dispatched work to prepared.
use super::{
    links,
    rows::{self, StoredEffect},
};
use crate::{Actor, Store};
use connect_protocol::{
    tasks::{EffectState, TaskEffect, TaskEventData, TaskOutcome, TaskRecord, TaskToolResult},
    Error, Outcome, Result,
};
use rusqlite::{params, Connection};

pub(super) fn event(
    c: &Connection,
    actor: &Actor,
    task: &TaskRecord,
    effect: &TaskEffect,
    timestamp: i64,
    store: &Store,
) -> Result<()> {
    let mut task = task.clone();
    task.updated_at = timestamp;
    super::super::storage::event_data(
        c,
        actor,
        &task,
        &TaskEventData::Effect(effect.clone()),
        store,
    )
}
pub(super) fn change(
    c: &Connection,
    actor: &Actor,
    task: &TaskRecord,
    effect: &mut TaskEffect,
    next: EffectState,
    timestamp: i64,
    store: &Store,
) -> Result<()> {
    if effect.state == next {
        return Ok(());
    }
    effect.state.transition(next)?;
    effect.state = next;
    let name = serde_json::to_value(next).map_err(|_| Error::storage())?;
    c.execute(
        "UPDATE task_effects SET state=?1,updated_at=?2 WHERE id=?3",
        params![
            name.as_str().ok_or_else(Error::storage)?,
            timestamp,
            effect.id
        ],
    )
    .map_err(|_| Error::storage())?;
    event(c, actor, task, effect, timestamp, store)
}
pub(super) fn reconcile(
    c: &Connection,
    actor: &Actor,
    task: &TaskRecord,
    effect: &mut StoredEffect,
    timestamp: i64,
    store: &Store,
) -> Result<()> {
    if !matches!(
        effect.effect.state,
        EffectState::Dispatched | EffectState::Uncertain
    ) {
        return Ok(());
    }
    // Schema 11 did not record protected dispatch authority. Preserve such
    // observations as read-only; missing links cannot manufacture success.
    if effect.credential_id.is_none() {
        return Ok(());
    }
    rows::verify(effect)?;
    if effect.result.is_none() {
        return Err(Error::storage());
    }
    let next = if effect.result.as_ref().is_some_and(Result::is_err) {
        EffectState::Failed
    } else {
        links::state(c, &effect.effect.id, store)?
    };
    // An unresolved observation cannot establish a safe re-dispatch boundary.
    if effect.effect.state == EffectState::Uncertain && next == EffectState::Dispatched {
        return Ok(());
    }
    if next == EffectState::Failed && effect.effect.error.is_none() {
        effect.effect.error = Some(Error::new(
            "provider-failed",
            "The recorded provider operation failed or was blocked",
        ));
        rows::save_result(c, effect)?;
    }
    change(c, actor, task, &mut effect.effect, next, timestamp, store)
}
pub(super) fn tool(c: &Connection, effect: &StoredEffect) -> Result<TaskOutcome> {
    let task_id = effect.effect.task_id.clone();
    let result = match effect.effect.state {
        EffectState::Prepared | EffectState::Dispatched => TaskToolResult::Pending {
            task_id,
            effect_id: Some(effect.effect.id.clone()),
        },
        EffectState::Uncertain => TaskToolResult::Uncertain {
            task_id,
            effect_id: effect.effect.id.clone(),
        },
        EffectState::Failed | EffectState::Cancelled => {
            return Err(effect.effect.error.clone().unwrap_or_else(|| {
                Error::new("task-effect-cancelled", "Task effect did not complete")
            }))
        }
        EffectState::Succeeded => {
            let outcome = effect.effect.outcome.clone().ok_or_else(Error::storage)?;
            let answered = if effect.wait_for_reply {
                let Outcome::Accepted { message_id, .. } = outcome.as_ref() else {
                    return Err(Error::storage());
                };
                c.query_row(
                    "SELECT EXISTS(SELECT 1 FROM task_replies WHERE task_id=?1 AND reply_to=?2)",
                    params![task_id, message_id],
                    |r| r.get::<_, bool>(0),
                )
                .map_err(|_| Error::storage())?
            } else {
                true
            };
            if answered {
                TaskToolResult::Completed { task_id, outcome }
            } else {
                TaskToolResult::Pending {
                    task_id,
                    effect_id: Some(effect.effect.id.clone()),
                }
            }
        }
    };
    Ok(TaskOutcome::Tool(result))
}
