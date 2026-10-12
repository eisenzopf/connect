//! Shared operation engine. The outer transaction belongs to either the ordinary
//! control request or its task ledger; no nested handler commits underneath it.
mod mutations;
mod reads;
mod receipts;
mod resources;
use crate::{Actor, Store};
use connect_protocol::{Error, Operation, Outcome, Result};
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};

pub(crate) struct OperationCommit {
    /// Authorized business rejection is retained; infrastructure errors are outer errors.
    pub outcome: Result<Outcome>,
    /// Existing fault injection points apply only to newly committed receipts.
    #[cfg_attr(not(feature = "test-support"), allow(dead_code))]
    pub new_receipt: bool,
}
pub(crate) fn execute(
    c: &Connection,
    actor: &Actor,
    operation: &Operation,
    runtime: &crate::DiscoveryRuntime,
    store: &Store,
) -> Result<OperationCommit> {
    actor.require(operation.required_scope())?;
    if let Some(outcome) = reads::read(c, actor, operation, runtime, store)? {
        return Ok(OperationCommit {
            outcome: Ok(outcome),
            new_receipt: false,
        });
    }
    let id = operation
        .request_id()
        .ok_or_else(|| invalid("Logical request ID is required"))?;
    receipts::valid_id(id)?;
    let resources = resources::authorize(c, actor, operation, store)?;
    let canonical = serde_json::to_value(operation).map_err(|_| invalid("Invalid operation"))?;
    let digest = format!(
        "v1:{}",
        hex::encode(Sha256::digest(
            serde_json::to_vec(&canonical).map_err(|_| Error::storage())?
        ))
    );
    if let Some(saved) = receipts::receipt(c, &actor.identity_id, id)? {
        receipts::check_receipt_authority(actor, &saved)?;
        if saved.digest != digest {
            return Err(Error::new(
                "conflict",
                "Logical request ID already has different inputs",
            ));
        }
        return Ok(OperationCommit {
            outcome: saved.result,
            new_receipt: false,
        });
    }
    let receipts: u64 = c
        .query_row(
            "SELECT (SELECT COUNT(*) FROM requests)+(SELECT COUNT(*) FROM task_requests)",
            [],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if !crate::tasks::budget::fits(c, store.limits.operation_receipts, receipts, 1)? {
        return Err(Error::new(
            "capacity-exceeded",
            "Durable request receipt capacity exceeded; no acceptance recorded",
        ));
    }
    // A rejected operation must not commit partial SQL writes. The enclosing
    // transaction retains its canonical rejection, except on storage failure.
    c.execute_batch("SAVEPOINT connect_operation")
        .map_err(|_| Error::storage())?;
    let result = mutations::apply(c, actor, operation, &resources, store);
    if result.is_err() {
        c.execute_batch("ROLLBACK TO connect_operation")
            .map_err(|_| Error::storage())?;
    }
    c.execute_batch("RELEASE connect_operation")
        .map_err(|_| Error::storage())?;
    if result
        .as_ref()
        .is_err_and(|e| e.code == "storage-unavailable")
    {
        return Err(Error::storage());
    }
    c.execute(
        "INSERT INTO requests VALUES(?1,?2,?3,?4,?5,?6)",
        params![
            actor.identity_id,
            id,
            digest,
            operation.name(),
            serde_json::to_string(&resources).map_err(|_| Error::storage())?,
            serde_json::to_string(&result).map_err(|_| Error::storage())?
        ],
    )
    .map_err(|_| Error::storage())?;
    Ok(OperationCommit {
        outcome: result,
        new_receipt: true,
    })
}
fn invalid(message: &str) -> Error {
    Error::new("invalid-request", message)
}
