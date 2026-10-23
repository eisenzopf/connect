//! Durable lineage is not evidence of a recovered physical connection.
use crate::Store;
use connect_protocol::{Error, Result};
use rusqlite::{params, TransactionBehavior};
use std::sync::OnceLock;

pub(crate) fn unowned_process_epoch() -> &'static str {
    static EPOCH: OnceLock<String> = OnceLock::new();
    EPOCH.get_or_init(|| format!("runtime_{}", uuid::Uuid::new_v4().simple()))
}

impl Store {
    pub(crate) fn fence_unobserved_connections(&self, limit: u32) -> Result<u32> {
        let _runtime = self.require_runtime_owner()?;
        let kind = crate::startup_authority::Kind::ConnectionEpoch;
        let (ids, guard) = self.startup_batch(kind, limit)?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let selected = self.startup_rows(&tx, kind, &ids, &guard)?;
        for row in &selected {
            tx.execute("UPDATE connections SET state='interrupted',generation=MIN(generation+1,9007199254740991),observed_runtime_id=NULL,observed_at=NULL,readiness_json=NULL WHERE id=?1", params![row.id]).map_err(|_| Error::storage())?;
        }
        for row in &selected {
            let (cid,sid):(String,String)=tx.query_row("SELECT s.conversation_id,n.session_id FROM connections n JOIN sessions s ON s.id=n.session_id WHERE n.id=?1",[&row.id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|Error::storage())?;
            crate::session_participant_lifecycle::reconcile(&tx, self, &cid, &sid, None)?;
        }
        self.require_runtime_owner()?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(selected.len() as u32)
    }
    /// Attach to current adapter observations; never copy this value from a
    /// client request or a restored database row. Stable across handles in
    /// the owned runtime and fresh for each new exclusive owner, even in one process.
    pub fn runtime_epoch(&self) -> &str {
        &self.runtime_epoch
    }
}

#[cfg(test)]
mod tests;
