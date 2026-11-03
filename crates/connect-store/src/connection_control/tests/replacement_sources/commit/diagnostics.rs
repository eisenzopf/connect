//! Attribute real metadata contention without treating admission as SQL commit.
use super::*;

#[test]
fn projection_diagnostics_distinguish_sql_mutex_authority_and_transaction_refusals() {
    let m = fixture(true);
    let capture = super::activation_barrier::captured(&m);
    let initial = capture.projection_diagnostics();
    assert_eq!(initial["capture"]["attempts"], 1);
    assert_eq!(initial["capture"]["admitted"], 1);
    assert_eq!(initial["activation"]["attempts"], 0);
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    let effects = m.f.count("operation_effects");

    let sql = m.f.store.connection().unwrap();
    assert_eq!(
        m.f.store
            .project_native_replacement_committed(&capture, &MetadataOnlyInstalled)
            .unwrap_err()
            .code,
        "authority-busy"
    );
    drop(sql);
    let metrics = capture.projection_diagnostics();
    assert_eq!(metrics["activation"]["failed_derive_before"], 1);
    assert_eq!(metrics["activation"]["failed_authority_wait"], 0);
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);

    // A real retained packet reader blocks the complete Conversation writer.
    // It remains current and is never retained by the diagnostic counters.
    let held =
        m.f.store
            .try_native_media_read(&m.sources[1], None)
            .unwrap();
    let began = std::time::Instant::now();
    assert_eq!(
        m.f.store
            .project_native_replacement_committed(&capture, &MetadataOnlyInstalled)
            .unwrap_err()
            .code,
        "authority-busy"
    );
    assert!(began.elapsed() < std::time::Duration::from_millis(500));
    assert!(held.current());
    drop(held);
    let metrics = capture.projection_diagnostics();
    assert_eq!(metrics["activation"]["failed_authority_wait"], 1);
    assert!(metrics["activation"]["authority_wait_us"].as_u64().unwrap() > 0);
    assert_eq!(metrics["activation"]["authority_busy"], 2);
    assert_eq!(metrics["activation"]["admitted"], 0);
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
    assert_eq!(metrics["activation"]["attempts"], 3);
    assert_eq!(metrics["activation"]["admitted"], 1);
    assert_eq!(
        metrics["activation"]["storage_unavailable"], 0,
        "transaction refusal occurs after the measured admission boundary"
    );
    assert_eq!(
        Store::inspect_snapshot(m.f.store.path()).unwrap(),
        before,
        "an admitted diagnostic sample cannot imply committed metadata"
    );

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
    assert_eq!(metrics["capture"], initial["capture"]);
    assert_eq!(metrics["activation"]["attempts"], 5);
    assert_eq!(metrics["activation"]["admitted"], 3);
    assert_eq!(metrics["activation"]["incomplete"], 0);
    assert_eq!(metrics["activation"]["permanent_refusal"], 0);
    assert_eq!(metrics["activation"]["failed_derive_before"], 1);
    assert_eq!(metrics["activation"]["failed_authority_wait"], 1);
    assert_eq!(metrics["activation"]["failed_sql_reacquire"], 0);
    assert_eq!(metrics["activation"]["failed_derive_after"], 0);
    assert_eq!(metrics["activation"]["failed_scope_revalidation"], 0);
}
