//! Protect one terminal journal entry and mutation receipt per nonterminal task.
//! An elapsed clock alone cannot release this room: its expiry event must commit.
use connect_protocol::{Error, Result};
use rusqlite::Connection;

/// Shared operation/task capacity admits ordinary work only around the reserve.
/// Terminal transitions update the task state in the same transaction before
/// writing their event/receipt, so each consumes its own released reservation.
pub(crate) fn fits(c: &Connection, limit: u64, consumed: u64, additional: u64) -> Result<bool> {
    let reserved: u64 = c
        .query_row(
            "SELECT COUNT(*) FROM tasks WHERE state IN ('ready','running','waiting')",
            [],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    Ok(consumed
        .checked_add(additional)
        .and_then(|n| n.checked_add(reserved))
        .is_some_and(|n| n <= limit))
}
