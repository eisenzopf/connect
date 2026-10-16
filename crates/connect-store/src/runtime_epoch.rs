//! Durable lineage is not evidence of a recovered physical connection.
use crate::Store;
use connect_protocol::{Error, Result};
use rusqlite::{params, Connection};
use std::sync::OnceLock;

pub(crate) fn process_epoch() -> &'static str {
    static EPOCH: OnceLock<String> = OnceLock::new();
    EPOCH.get_or_init(|| format!("runtime_{}", uuid::Uuid::new_v4().simple()))
}

pub(crate) fn fence_unobserved_connections(c: &Connection, epoch: &str) -> Result<()> {
    c.execute(
        "UPDATE connections SET state='interrupted',
         generation=MIN(generation+1,9007199254740991), observed_runtime_id=NULL,
         observed_at=NULL, readiness_json=NULL
         WHERE state IN ('preparing','ready','active','held','retiring')
         AND (observed_runtime_id IS NULL OR observed_runtime_id!=?1)",
        params![epoch],
    )
    .map_err(|_| Error::storage())?;
    Ok(())
}

impl Store {
    /// Attach to current adapter observations; never copy this value from a
    /// client request or a restored database row. Stable across handles in
    /// this process and fresh after process restart.
    pub fn runtime_epoch(&self) -> &'static str {
        process_epoch()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_a_new_runtime_requires_fresh_connection_observations() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE connections(id TEXT,state TEXT,generation INTEGER,observed_runtime_id TEXT,observed_at INTEGER,readiness_json TEXT);
             INSERT INTO connections VALUES('retained','active',4,'old_runtime',100,'{}');
             INSERT INTO connections VALUES('current','held',2,'new_runtime',100,'{}');
             INSERT INTO connections VALUES('retired','retired',5,'old_runtime',100,'{}');
             INSERT INTO connections VALUES('uncertain','unknown',6,'old_runtime',100,'{}');",
        ).unwrap();
        fence_unobserved_connections(&c, "new_runtime").unwrap();
        let value: (String,u64,Option<String>,Option<String>) = c.query_row(
            "SELECT state,generation,observed_runtime_id,readiness_json FROM connections WHERE id='retained'",[],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
        ).unwrap();
        assert_eq!(value, ("interrupted".into(), 5, None, None));
        for (id, state) in [
            ("current", "held"),
            ("retired", "retired"),
            ("uncertain", "unknown"),
        ] {
            assert_eq!(
                c.query_row("SELECT state FROM connections WHERE id=?1", [id], |r| {
                    r.get::<_, String>(0)
                })
                .unwrap(),
                state
            );
        }
        fence_unobserved_connections(&c, "new_runtime").unwrap();
        assert_eq!(
            c.query_row(
                "SELECT generation FROM connections WHERE id='retained'",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            5
        );
    }
}
