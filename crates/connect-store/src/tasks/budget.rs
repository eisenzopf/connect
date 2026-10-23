//! Protect terminal work in each independent durable capacity budget.
use connect_protocol::{Error, Result};
use rusqlite::Connection;

fn fits(c: &Connection, limit: u64, consumed: u64, additional: u64, journal: bool) -> Result<bool> {
    let common: u64 = c.query_row(
        "SELECT (SELECT COUNT(*) FROM tasks WHERE state IN ('ready','running','waiting'))+(SELECT COUNT(*) FROM operations WHERE ledger_version=1 AND state NOT IN ('completed','cancelled','failed'))", [], |r|r.get(0),
    ).map_err(|_|Error::storage())?;
    let native: u64 = c.query_row(if journal {
        // Cleanup always retains its audience fanout and terminal operation fact.
        // The first uncertain observation and first cancellation each consume
        // one extra reservation; neither releases the cleanup reservation.
        "SELECT (SELECT COALESCE(SUM(CASE WHEN o.state='unknown' THEN n.terminal_reserve ELSE 2*n.terminal_reserve+1 END + CASE WHEN o.cancel_requested=1 THEN 0 ELSE 1 END),0) FROM native_admission_sources n JOIN operations o ON o.record_id=n.operation_record_id WHERE o.state NOT IN ('completed','cancelled','failed'))+(SELECT COALESCE(SUM(n.terminal_reserve),0) FROM native_connection_lifetimes life JOIN native_admission_sources n ON n.operation_record_id=life.operation_record_id WHERE life.retired=0)"
    } else {
        // The first explicit cancel decision has a separate ledger record.
        "SELECT COUNT(*) FROM native_admission_sources n JOIN operations o ON o.record_id=n.operation_record_id WHERE o.state NOT IN ('completed','cancelled','failed') AND o.cancel_requested=0"
    }, [], |r|r.get(0)).map_err(|_|Error::storage())?;
    let streams: u64 = c.query_row(if journal {
        "SELECT COALESCE(SUM(CASE o.state WHEN 'accepted' THEN 3 WHEN 'preparing' THEN 2 ELSE 1 END + CASE WHEN o.cancel_requested=1 THEN 0 ELSE 2 END),0) FROM native_stream_operations n JOIN operations o ON o.record_id=n.operation_record_id WHERE o.state NOT IN ('completed','cancelled','failed')"
    } else {
        "SELECT COUNT(*) FROM native_stream_operations n JOIN operations o ON o.record_id=n.operation_record_id WHERE o.state NOT IN ('completed','cancelled','failed') AND o.cancel_requested=0"
    }, [], |r|r.get(0)).map_err(|_|Error::storage())?;
    let writers: u64 = if journal {
        c.query_row(
            "SELECT (SELECT COUNT(*) FROM native_data_writes WHERE disposition IS NULL)+(SELECT COUNT(*) FROM native_dtmf_recipients WHERE disposition IS NULL)",
            [],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?
    } else {
        0
    };
    Ok(consumed
        .checked_add(additional)
        .and_then(|n| n.checked_add(common))
        .and_then(|n| n.checked_add(native))
        .and_then(|n| n.checked_add(streams))
        .and_then(|n| n.checked_add(writers))
        .is_some_and(|n| n <= limit))
}
pub(crate) fn journal(c: &Connection, limit: u64, consumed: u64, additional: u64) -> Result<bool> {
    fits(c, limit, consumed, additional, true)
}
pub(crate) fn receipts(c: &Connection, limit: u64, consumed: u64, additional: u64) -> Result<bool> {
    fits(c, limit, consumed, additional, false)
}
