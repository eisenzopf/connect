//! CAS leases coordinate workers; they never authorize another external dispatch.
use super::{authority, storage};
use crate::{Actor, Store};
use connect_protocol::{
    tasks::{
        TaskCommand, TaskEventData, TaskOutcome, TaskRecord, TaskRequest, TaskStepClaim,
        MAX_RUNNING_TASKS, TASK_CLAIM_SECONDS,
    },
    Error, Result,
};
use rusqlite::{params, Connection, OptionalExtension};

fn load(c: &Connection, task: &str, step: &str) -> Result<Option<TaskStepClaim>> {
    c.query_row(
        "SELECT worker_id,revision,expires_at FROM task_claims WHERE task_id=?1 AND step_id=?2",
        params![task, step],
        |r| {
            Ok(TaskStepClaim {
                task_id: task.into(),
                step_id: step.into(),
                worker_id: r.get(0)?,
                revision: r.get(1)?,
                expires_at: r.get(2)?,
            })
        },
    )
    .optional()
    .map_err(|_| Error::storage())
}
fn conflict() -> Error {
    Error::new(
        "task-claim-conflict",
        "The step lease has changed or belongs to another worker",
    )
}
pub(super) fn execute(
    c: &Connection,
    actor: &Actor,
    task: &TaskRecord,
    request: &TaskRequest,
    timestamp: i64,
    store: &Store,
) -> Result<TaskOutcome> {
    let step = match &request.command {
        TaskCommand::Claim { step_id, .. }
        | TaskCommand::ClaimGet { step_id, .. }
        | TaskCommand::ReleaseClaim { step_id, .. } => step_id,
        _ => return Err(Error::forbidden()),
    };
    let current = load(c, &task.id, step)?;
    if matches!(request.command, TaskCommand::ClaimGet { .. }) {
        return Ok(TaskOutcome::Claim(current));
    }
    let id = request.command.request_id().ok_or_else(Error::forbidden)?;
    let intent = storage::digest(request)?;
    if let Some((_, saved)) = storage::saved(c, actor, id, &intent)? {
        return Ok(saved);
    }
    authority::continuation(c, &storage::load(c, &task.id)?, timestamp, false)?;
    let claim = match &request.command {
        TaskCommand::Claim {
            worker_id,
            expected_revision,
            ..
        } => {
            match &current {
                None if expected_revision.is_none() => (),
                Some(row)
                    if Some(row.revision) == *expected_revision
                        && (row.worker_id == *worker_id || row.expires_at <= timestamp) => {}
                _ => return Err(conflict()),
            }
            let unsafe_effect:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM task_effects WHERE task_id=?1 AND step_id=?2 AND state!='prepared' AND state!='cancelled')",params![task.id,step],|r|r.get(0)).map_err(|_|Error::storage())?;
            if unsafe_effect {
                return Err(Error::new(
                    "effect-reconciliation-required",
                    "The step has a durable effect; reconcile it without reclaiming dispatch",
                ));
            }
            let count:u32=c.query_row("SELECT COUNT(*) FROM task_claims c JOIN tasks t ON t.id=c.task_id WHERE c.expires_at>?1 AND t.expires_at>?1 AND t.state IN ('ready','running','waiting')",[timestamp],|r|r.get(0)).map_err(|_|Error::storage())?;
            if count >= MAX_RUNNING_TASKS
                && !current.as_ref().is_some_and(|r| r.expires_at > timestamp)
            {
                return Err(Error::new(
                    "capacity-exceeded",
                    "Concurrent step lease capacity exceeded",
                ));
            }
            TaskStepClaim {
                task_id: task.id.clone(),
                step_id: step.clone(),
                worker_id: worker_id.clone(),
                revision: current.as_ref().map_or(Ok(0), |r| {
                    r.revision.checked_add(1).ok_or_else(Error::storage)
                })?,
                expires_at: timestamp
                    .checked_add(TASK_CLAIM_SECONDS)
                    .ok_or_else(Error::storage)?
                    .min(task.expires_at)
                    .min(actor.expires_at),
            }
        }
        TaskCommand::ReleaseClaim {
            worker_id,
            expected_revision,
            ..
        } => {
            let row = current.ok_or_else(conflict)?;
            if row.worker_id != *worker_id || row.revision != *expected_revision {
                return Err(conflict());
            }
            TaskStepClaim {
                revision: row.revision.checked_add(1).ok_or_else(Error::storage)?,
                expires_at: timestamp.min(row.expires_at),
                ..row
            }
        }
        _ => return Err(Error::forbidden()),
    };
    c.execute("INSERT INTO task_claims(task_id,step_id,worker_id,revision,expires_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(task_id,step_id) DO UPDATE SET worker_id=excluded.worker_id,revision=excluded.revision,expires_at=excluded.expires_at",params![claim.task_id,claim.step_id,claim.worker_id,claim.revision,claim.expires_at]).map_err(|_|Error::storage())?;
    let mut record = task.clone();
    record.updated_at = timestamp;
    storage::event_data(
        c,
        actor,
        &record,
        &TaskEventData::Claim(claim.clone()),
        store,
    )?;
    let outcome = TaskOutcome::Claim(Some(claim));
    storage::save_outcome(c, actor, id, &intent, &task.id, &outcome, store)?;
    Ok(outcome)
}
