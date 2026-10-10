//! Protected composition registers a verified origin; headers cannot do so.
use super::rows;
use crate::{identity::actor_from_connection_at, Actor};
use connect_protocol::{
    tasks::{InvocationBinding, TaskRecord},
    Error, Result,
};
use rusqlite::{params, Connection, OptionalExtension};

pub(super) fn check(
    c: &Connection,
    task: &TaskRecord,
    binding: &InvocationBinding,
    timestamp: i64,
) -> Result<()> {
    let row = lookup(c, binding)?;
    let Some((task_id, registered_by)) = row else {
        return Err(Error::forbidden());
    };
    let registrar = actor_from_connection_at(c, &registered_by, timestamp)?;
    registrar.require("task_grant")?;
    if task_id != task.id || registrar.identity_id != task.owner_id {
        return Err(Error::forbidden());
    }
    Ok(())
}
pub(super) fn register(
    c: &Connection,
    actor: &Actor,
    task: &TaskRecord,
    binding: &InvocationBinding,
    timestamp: i64,
    maximum: u64,
) -> Result<()> {
    binding.validate()?;
    let row = lookup(c, binding)?;
    if let Some((task_id, registrar)) = row {
        if task_id != task.id || registrar != actor.credential_id {
            return Err(Error::new(
                "binding-conflict",
                "External invocation already has another binding",
            ));
        }
        return check(c, task, binding, timestamp);
    }
    let count: u64 = c
        .query_row("SELECT COUNT(*) FROM task_bindings", [], |r| r.get(0))
        .map_err(|_| Error::storage())?;
    if count >= maximum {
        return Err(Error::new(
            "capacity-exceeded",
            "Invocation binding capacity exceeded",
        ));
    }
    c.execute(
        "INSERT INTO task_bindings(adapter,kind,external_id,task_id,registered_by,created_at)
         VALUES(?1,?2,?3,?4,?5,?6)",
        params![
            binding.adapter,
            rows::kind(binding)?,
            binding.external_id,
            task.id,
            actor.credential_id,
            timestamp
        ],
    )
    .map_err(|_| Error::storage())?;
    Ok(())
}
fn lookup(c: &Connection, binding: &InvocationBinding) -> Result<Option<(String, String)>> {
    c.query_row(
        "SELECT task_id,registered_by FROM task_bindings
         WHERE adapter=?1 AND kind=?2 AND external_id=?3",
        params![binding.adapter, rows::kind(binding)?, binding.external_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
    .map_err(|_| Error::storage())
}
