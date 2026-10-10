//! One transaction owns task intent, ordinary operation receipt and effect journal.
mod dispatch;
mod links;
mod rows;
mod state;
use super::{authority, lifecycle, storage};
use crate::{Actor, DiscoveryRuntime, Store};
use connect_protocol::{
    tasks::{TaskCommand, TaskOutcome, TaskRecord, TaskRequest, TaskState, TaskToolResult},
    Error, Result,
};
use rusqlite::Connection;

/// Outer errors roll back admission; inner errors are durable business rejections.
#[derive(Clone, Copy)]
pub(super) struct Execution<'a> {
    pub c: &'a Connection,
    pub actor: &'a Actor,
    pub task: &'a TaskRecord,
    pub store: &'a Store,
    pub runtime: &'a DiscoveryRuntime,
    pub timestamp: i64,
}

pub(super) fn invoke(
    execution: Execution<'_>,
    request: &TaskRequest,
    grant_id: Option<&str>,
) -> Result<Result<TaskOutcome>> {
    let Execution {
        c,
        actor,
        task,
        store,
        runtime,
        timestamp,
    } = execution;
    let TaskCommand::Invoke {
        operation,
        reply_to,
        wait_for_reply,
        ..
    } = &request.command
    else {
        return Err(Error::forbidden());
    };
    authority::operation(c, actor, task, operation)?;
    dispatch::authorize_reply(c, actor, task, reply_to.as_deref())?;
    if operation.request_id().is_none() {
        let outcome = crate::operations::execute(c, actor, operation, runtime, store)?.outcome?;
        return Ok(Ok(TaskOutcome::Tool(TaskToolResult::Completed {
            task_id: task.id.clone(),
            outcome: Box::new(outcome),
        })));
    }
    let intent = storage::digest(request)?;
    let TaskCommand::Invoke {
        request_id,
        step_id,
        ..
    } = &request.command
    else {
        return Err(Error::forbidden());
    };
    if let Some(mut saved) = rows::matching(c, &task.id, step_id, request_id)? {
        if saved.intent != intent {
            return Err(Error::new(
                "request-conflict",
                "Task effect already has different inputs",
            ));
        }
        rows::verify(&saved)?;
        state::reconcile(c, actor, task, &mut saved, timestamp, store)?;
        return Ok(state::tool(c, &saved));
    }
    // A completed task may expose a matching receipt, but cannot create work.
    authority::continuation(c, &storage::load(c, &task.id)?, timestamp, false)?;
    let mut record = task.clone();
    if record.state == TaskState::Ready {
        record = lifecycle::change(c, actor, record, TaskState::Running, None, timestamp, store)?;
    }
    let mut effect = rows::prepare(c, actor, request, grant_id, timestamp, store)?;
    state::event(c, actor, &record, &effect.effect, timestamp, store)?;
    let operation = rows::namespaced(
        operation,
        effect
            .wire_request_id
            .as_deref()
            .ok_or_else(Error::storage)?,
    )?;
    let result = dispatch::execute(
        Execution {
            task: &record,
            ..execution
        },
        request,
        &operation,
    )?;
    effect.result = Some(result.clone());
    if let Ok(outcome) = &result {
        effect.effect.outcome = Some(Box::new(outcome.clone()));
        links::register(c, &effect.effect.id, &operation, outcome)?;
    } else if let Err(error) = &result {
        effect.effect.error = Some(error.clone());
    }
    rows::save_result(c, &effect)?;
    state::change(
        c,
        actor,
        &record,
        &mut effect.effect,
        connect_protocol::tasks::EffectState::Dispatched,
        timestamp,
        store,
    )?;
    state::reconcile(c, actor, &record, &mut effect, timestamp, store)?;
    if *wait_for_reply && result.is_ok() && record.state == TaskState::Running {
        lifecycle::change(c, actor, record, TaskState::Waiting, None, timestamp, store)?;
    }
    Ok(state::tool(c, &effect))
}

pub(super) fn get(
    c: &Connection,
    actor: &Actor,
    task: &TaskRecord,
    id: &str,
    timestamp: i64,
    store: &Store,
) -> Result<TaskOutcome> {
    let mut effect = rows::load(c, id, &task.id)?;
    // Current operation scope still applies when reading a retained receipt.
    authority::operation(c, actor, task, &effect.operation)?;
    state::reconcile(c, actor, task, &mut effect, timestamp, store)?;
    Ok(TaskOutcome::Effect(effect.effect))
}

pub(crate) use dispatch::authorize_effect;
