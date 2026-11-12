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

    // A real affected-source packet reader blocks its exact Connection Writer.
    // It remains current and is never retained by the diagnostic counters.
    let held =
        m.f.store
            .try_native_media_read(&m.sources[0], None)
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

#[test]
fn captured_projection_drains_brief_primary_sql_exclusion_within_original_admission_budget() {
    let m = fixture(true);
    let capture = super::activation_barrier::captured(&m);
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    let effects = m.f.count("operation_effects");
    let began = std::time::Instant::now();
    let (first, phases) = std::thread::scope(|workers| {
        let (locked, ready) = std::sync::mpsc::sync_channel(0);
        let (release, released) = std::sync::mpsc::sync_channel(0);
        let store = &m.f.store;
        let holder = workers.spawn(move || {
            let sql = store.connection().unwrap();
            locked.send(()).unwrap();
            released.recv().unwrap();
            // Short-lived real command SQL contention. No authority writer is
            // held by this owner, and no physical operation is manufactured.
            std::thread::sleep(std::time::Duration::from_millis(2));
            drop(sql);
        });
        ready.recv().unwrap();
        release.send(()).unwrap();
        let (result, phases) = crate::scoped_phase_timing::collect(|| {
            m.f.store
                .project_native_replacement_committed(&capture, &MetadataOnlyInstalled)
        });
        holder.join().unwrap();
        (result, phases)
    });
    let elapsed = began.elapsed();
    eprintln!(
        "original Ready body phase timings (microseconds, bounded static labels): {phases:?}"
    );
    let admitted = first.is_ok();
    let code = first
        .as_ref()
        .err()
        .map(|e| e.code.as_str())
        .unwrap_or("ok");
    eprintln!(
        "original projection short SQL exclusion: admitted={admitted} code={code} elapsed_us={} first_attempt_diagnostics={}",
        elapsed.as_micros(),
        capture.projection_diagnostics()
    );
    if !admitted {
        assert_eq!(
            Store::inspect_snapshot(m.f.store.path()).unwrap(),
            before,
            "refused initial admission has no partial metadata effect"
        );
    }
    // Settle/recover only the exact captured metadata after joining its SQL
    // owner, including on the expected before-fix failure. Never repeat Core IO.
    let established =
        m.f.store
            .project_native_replacement_committed(&capture, &MetadataOnlyInstalled)
            .unwrap();
    let complete = Store::inspect_snapshot(m.f.store.path()).unwrap();
    let recovered =
        m.f.store
            .project_native_replacement_committed(&capture, &MetadataOnlyInstalled)
            .unwrap();
    assert_eq!(
        established.runtime_connection_id(),
        recovered.runtime_connection_id()
    );
    assert_eq!(established.generation(), recovered.generation());
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), complete);
    assert_eq!(m.f.count("operation_effects"), effects);
    assert!(
        elapsed < std::time::Duration::from_millis(50),
        "bounded original admission"
    );
    assert!(
        admitted,
        "brief SQL exclusion discarded an otherwise live original admission budget"
    );
}

#[test]
fn initial_projection_sql_wait_keeps_refusals_effect_free_and_current_installation_required() {
    let m = fixture(true);
    let capture = super::activation_barrier::captured(&m);
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    let effects = m.f.count("operation_effects");
    let sql = m.f.store.connection().unwrap();
    let began = std::time::Instant::now();
    let error =
        m.f.store
            .project_native_replacement_committed(&capture, &MetadataOnlyInstalled)
            .unwrap_err();
    let elapsed = began.elapsed();
    assert_eq!(error.code, "authority-busy");
    assert!(
        elapsed < std::time::Duration::from_millis(20),
        "initial SQL exclusion must not borrow the ordinary two-second SQL timeout"
    );
    drop(sql);
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
    assert!(m
        .f
        .store
        .try_native_media_read(&m.sources[1], None)
        .unwrap()
        .current());

    let held =
        m.f.store
            .try_native_media_read(&m.sources[0], None)
            .unwrap();
    let began = std::time::Instant::now();
    let error =
        m.f.store
            .project_native_replacement_committed(&capture, &MetadataOnlyInstalled)
            .unwrap_err();
    assert_eq!(error.code, "authority-busy");
    assert!(began.elapsed() < std::time::Duration::from_millis(20));
    assert!(held.current());
    drop(held);
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);

    let foreign = rusqlite::Connection::open(m.f.store.path()).unwrap();
    foreign.execute_batch("BEGIN IMMEDIATE").unwrap();
    let began = std::time::Instant::now();
    let error =
        m.f.store
            .project_native_replacement_committed(&capture, &MetadataOnlyInstalled)
            .unwrap_err();
    assert_eq!(error.code, "storage-unavailable");
    assert!(began.elapsed() < std::time::Duration::from_millis(20));
    foreign.execute_batch("ROLLBACK").unwrap();
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);

    struct MissingInstallation;
    impl crate::NativeReplacementInstallationObservation for MissingInstallation {
        fn hold_installed(
            &self,
            _: &crate::NativeReplacementActivation,
        ) -> crate::Result<Option<Box<dyn crate::NativeReplacementInstallationGuard + '_>>>
        {
            Ok(None)
        }
    }
    assert_eq!(
        m.f.store
            .project_native_replacement_committed(&capture, &MissingInstallation,)
            .unwrap_err()
            .code,
        "authority-busy"
    );
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
    assert!(m
        .f
        .store
        .try_native_media_read(capture.activation().origin(), None)
        .is_err());
    let origin =
        m.f.store
            .project_native_replacement_committed(&capture, &MetadataOnlyInstalled)
            .unwrap();
    assert!(m
        .f
        .store
        .try_native_media_read(&origin, None)
        .unwrap()
        .current());
    assert_eq!(m.f.count("operation_effects"), effects);
    eprintln!("original projection SQL budget: sustained SQL/authority and foreign writer refused; no partial metadata, fresh packet authority restored, missing installation refused, exact capture activated once");
}
