//! Real SQLite failure points, conserved durable facts, and original replay.
use super::*;

#[test]
fn transaction_diagnostics_separate_begin_body_commit_and_unchanged_replay() {
    let m = fixture(true);
    let capture = super::activation_barrier::captured(&m);
    let initial = capture.projection_diagnostics();
    assert_eq!(initial["capture"]["transaction"]["committed"], 1);
    assert_eq!(initial["activation"]["transaction"]["committed"], 0);
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    let effects = m.f.count("operation_effects");

    // Duplicate capture is a current read with no SQL transaction, not
    // another movement or an invented unchanged transaction.
    m.f.store
        .record_native_replacement_committed(&capture)
        .unwrap();
    let recorded = capture.projection_diagnostics()["capture"].clone();
    assert_eq!(recorded, initial["capture"], "read-only historical replay must leave every capture transaction/admission diagnostic unchanged");
    assert_eq!(recorded["transaction"]["committed"], 1);
    assert_eq!(recorded["transaction"]["unchanged"], 0);
    assert_eq!(m.f.count("operation_effects"), effects);
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);

    let foreign = rusqlite::Connection::open(m.f.store.path()).unwrap();
    foreign.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert_eq!(
        m.f.store
            .project_native_replacement_committed(&capture, &MetadataOnlyInstalled)
            .unwrap_err()
            .code,
        "storage-unavailable"
    );
    foreign.execute_batch("ROLLBACK").unwrap();
    let metrics = capture.projection_diagnostics();
    assert_eq!(metrics["activation"]["admitted"], 1);
    assert_eq!(metrics["activation"]["transaction"]["begin_failed"], 1);
    assert_eq!(metrics["activation"]["transaction"]["body_aborted"], 0);
    assert_eq!(metrics["activation"]["transaction"]["commit_failed"], 0);
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);

    // Abort a real write after admission, while no transaction can commit.
    m.f.store
        .connection()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER diagnostics_body_fault BEFORE INSERT ON native_replacement_lifetimes
         BEGIN SELECT RAISE(ABORT,'controlled body fault'); END;",
        )
        .unwrap();
    assert_eq!(
        m.f.store
            .project_native_replacement_committed(&capture, &MetadataOnlyInstalled)
            .unwrap_err()
            .code,
        "storage-unavailable"
    );
    m.f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER diagnostics_body_fault")
        .unwrap();
    let metrics = capture.projection_diagnostics();
    assert_eq!(metrics["activation"]["transaction"]["body_aborted"], 1);
    assert_eq!(metrics["activation"]["transaction"]["commit_failed"], 0);
    assert_eq!(metrics["activation"]["transaction"]["committed"], 0);
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);

    // A deferred foreign key accepts the body and fails at COMMIT itself.
    // Fixture-only tables/trigger never alter original operation or effect rows.
    m.f.store.connection().unwrap().execute_batch(
        "CREATE TABLE diagnostics_parent(id INTEGER PRIMARY KEY);
         CREATE TABLE diagnostics_child(parent INTEGER REFERENCES diagnostics_parent(id) DEFERRABLE INITIALLY DEFERRED);
         CREATE TRIGGER diagnostics_commit_fault AFTER INSERT ON native_replacement_lifetimes
         BEGIN INSERT INTO diagnostics_child(parent) VALUES(1); END;"
    ).unwrap();
    assert_eq!(
        m.f.store
            .connection()
            .unwrap()
            .query_row("PRAGMA foreign_keys", [], |r| r.get::<_, u64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        m.f.store
            .project_native_replacement_committed(&capture, &MetadataOnlyInstalled)
            .unwrap_err()
            .code,
        "storage-unavailable"
    );
    let metrics = capture.projection_diagnostics();
    assert_eq!(metrics["activation"]["transaction"]["begin_failed"], 1);
    assert_eq!(metrics["activation"]["transaction"]["body_aborted"], 1);
    assert_eq!(metrics["activation"]["transaction"]["commit_failed"], 1);
    assert_eq!(metrics["activation"]["transaction"]["committed"], 0);
    assert_eq!(
        m.f.store
            .connection()
            .unwrap()
            .query_row("SELECT count(*) FROM diagnostics_child", [], |r| r
                .get::<_, u64>(0))
            .unwrap(),
        0
    );
    m.f.store.connection().unwrap().execute_batch(
        "DROP TRIGGER diagnostics_commit_fault; DROP TABLE diagnostics_child; DROP TABLE diagnostics_parent;"
    ).unwrap();
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);

    m.f.store
        .project_native_replacement_committed(&capture, &MetadataOnlyInstalled)
        .unwrap();
    let complete = Store::inspect_snapshot(m.f.store.path()).unwrap();
    m.f.store
        .project_native_replacement_committed(&capture, &MetadataOnlyInstalled)
        .unwrap();
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), complete);
    assert_eq!(m.f.count("operation_effects"), effects);
    let metrics = capture.projection_diagnostics();
    assert_eq!(metrics["capture"], recorded);
    assert_eq!(metrics["activation"]["attempts"], 5);
    assert_eq!(metrics["activation"]["admitted"], 5);
    let sql = &metrics["activation"]["transaction"];
    for outcome in [
        "begin_failed",
        "body_aborted",
        "commit_failed",
        "committed",
        "unchanged",
    ] {
        assert_eq!(sql[outcome], 1, "incorrect transaction outcome: {outcome}");
    }
    assert!(sql["body_us"].as_u64().unwrap() > 0);
    assert!(sql["commit_us"].as_u64().unwrap() > 0);
}
