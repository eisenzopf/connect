//! Statement reuse is not value reuse: every production stamp still steps SQL.
use super::*;
use rusqlite::StatementStatus;

fn runs(c: &Connection, sql: &str) -> i32 {
    c.prepare_cached(sql)
        .unwrap()
        .get_status(StatementStatus::Run)
}

#[test]
fn version_reuses_both_statements_without_replacing_fresh_sql_steps() {
    let f = Fixture::new();
    let observation = f.store.packet_connections.observe().unwrap();
    let c = f.store.try_packet_connection(&observation).unwrap();
    for sql in [DATA_VERSION_SQL, SCHEMA_VERSION_SQL] {
        c.prepare_cached(sql)
            .unwrap()
            .reset_status(StatementStatus::Run);
    }
    const READS: i32 = 256;
    let started = Instant::now();
    let mut first = None;
    for _ in 0..READS {
        let current = version(&c, c.id()).unwrap();
        if let Some(first) = first {
            assert!(current.same_database(first));
        } else {
            first = Some(current);
        }
    }
    // Diagnostic only. No elapsed-time threshold or WT/media conclusion.
    eprintln!(
        "native-version-statement-reuse reads={} elapsed_ns={}",
        READS,
        started.elapsed().as_nanos()
    );
    assert_eq!(runs(&c, DATA_VERSION_SQL), READS);
    assert_eq!(runs(&c, SCHEMA_VERSION_SQL), READS);
}

#[test]
fn prepared_version_steps_report_actual_external_data_and_schema_changes() {
    let f = Fixture::new();
    let observation = f.store.packet_connections.observe().unwrap();
    let c = f.store.try_packet_connection(&observation).unwrap();
    let initial = version(&c, c.id()).unwrap();
    let external = Connection::open(f.store.path()).unwrap();
    external
        .execute("UPDATE packet_snapshot_test SET value=2", [])
        .unwrap();
    let data_changed = version(&c, c.id()).unwrap();
    assert_eq!(data_changed.connection, initial.connection);
    assert_ne!(data_changed.external, initial.external);
    assert_eq!(data_changed.schema, initial.schema);
    assert!(!initial.same_database(data_changed));
    assert_eq!(f.derive(&c).unwrap().0, 2);
    external
        .execute_batch(
            "ALTER TABLE packet_snapshot_test ADD COLUMN statement_probe INTEGER;
         UPDATE packet_snapshot_test SET value=3;",
        )
        .unwrap();
    let schema_changed = version(&c, c.id()).unwrap();
    assert_eq!(schema_changed.connection, initial.connection);
    assert_ne!(schema_changed.schema, data_changed.schema);
    assert_ne!(schema_changed.external, data_changed.external);
    assert!(!data_changed.same_database(schema_changed));
    assert_eq!(f.derive(&c).unwrap().0, 3);
    // These statements still execute SQL after schema invalidation; neither
    // returned value is a cached PRAGMA result or an invented shared epoch.
    assert!(runs(&c, DATA_VERSION_SQL) >= 1);
    assert!(runs(&c, SCHEMA_VERSION_SQL) >= 1);
}
