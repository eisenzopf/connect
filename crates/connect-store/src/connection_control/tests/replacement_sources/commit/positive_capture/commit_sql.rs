//! Actual Store/Registry custody only; no Core or decoded-media execution claim.
use super::*;
use std::time::{Duration, Instant};

fn counts(store: &Store, operation: &str) -> rusqlite::Result<(u64, u64)> {
    let c = rusqlite::Connection::open_with_flags(
        store.path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    c.query_row(
        "SELECT (SELECT COUNT(*) FROM native_replacement_steps s JOIN operations o ON o.record_id=s.operation_record_id WHERE o.operation_id=?1 AND s.phase='commit'),(SELECT COUNT(*) FROM operation_effects e JOIN operations o ON o.record_id=e.operation_record_id WHERE o.operation_id=?1 AND e.provider='native-replacement-commit')",
        [operation],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
}
fn metadata<T>(mut call: impl FnMut() -> crate::Result<T>) -> crate::Result<T> {
    let until = Instant::now() + Duration::from_secs(1);
    loop {
        match call() {
            Err(e) if e.code == "authority-busy" && Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(1));
            }
            result => return result,
        }
    }
}
fn scene(
    store: &Store,
    origin: &Arc<NativeReplacementMediaOrigin>,
    sources: &[Arc<NativeMediaOrigin>],
    until: Instant,
) -> crate::Result<crate::NativeReplacementSceneRead> {
    loop {
        match store.try_native_replacement_commit_scene_write(origin, sources) {
            Err(e) if e.code == "authority-busy" && Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(1));
            }
            result => return result,
        }
    }
}
fn settle_not_submitted(
    store: &Store,
    started: &crate::NativeReplacementStarted,
    attempt: crate::NativeReplacementCommitAttempt,
) -> crate::Result<()> {
    // This controlled Store fixture never invokes Core. Attribute that exact
    // once-only attempt and settle only its declared absent fixture candidate.
    let gone = super::super::super::candidate::Absent(
        attempt.activation().origin().runtime_connection_id().into(),
        attempt.activation().origin().generation(),
    );
    let capture = attempt.not_submitted();
    metadata(|| store.record_native_replacement_commit_not_submitted(&capture))?;
    metadata(|| store.record_native_replacement_started_candidate_stopped(started, &gone))?;
    Ok(())
}

#[test]
fn commit_sql_busy_after_scene_derive_preserves_unaffected_audio_io_and_original_epoch() {
    let m = fixture(true);
    let operation = m.command.operation_id.clone();
    let origin = m.origin.clone();
    let sources = m.sources.clone();
    let source = m.sources[1].clone();
    let store = Arc::new(m.f.store);
    let (snapshot, mut watcher) = publication(&store, &source);
    let io = admit(&snapshot);
    let stamp = io.snapshot_stamp().unwrap();
    let epoch = io.packet_epoch().unwrap();
    let before = counts(&store, &operation).unwrap();
    let until = Instant::now() + Duration::from_secs(2);
    let (entered, ready) = std::sync::mpsc::sync_channel(1);
    let (release, resume) = std::sync::mpsc::sync_channel(1);
    let (arrived, primary_busy, observer_current, result, survived, original_survived, after) =
        std::thread::scope(|workers| {
            let worker = workers.spawn(|| {
                Store::with_native_replacement_commit_entry_probe_for_test(
                    "scene-derived",
                    move || {
                        let _ = entered.send(());
                        let _ =
                            resume.recv_timeout(until.saturating_duration_since(Instant::now()));
                    },
                    || store.try_native_replacement_commit_scene_write(&origin, &sources),
                )
            });
            // Pause AFTER the real provisional read and all aliases dropped,
            // BEFORE any real WriterNotice. This excludes the primary-vs-packet
            // observer distinction from the intended old-condition oracle.
            let arrived = ready.recv_timeout(Duration::from_secs(1)).is_ok();
            let sql = if arrived {
                store.connection().ok()
            } else {
                None
            };
            let primary_busy = sql.is_some()
                && store
                    .try_connection()
                    .err()
                    .is_some_and(|e| e.code == "authority-busy");
            let packet = store.try_native_media_read(&source, None);
            let observer_current = packet.as_ref().is_ok_and(|read| read.current());
            drop(packet);
            let _ = release.send(());
            let joined = worker.join();
            let survived = io.current() && epoch.current();
            let original = snapshot.admit_original(stamp);
            let original_survived = original.as_ref().is_ok_and(|read| read.current());
            drop(original);
            let after = counts(&store, &operation);
            drop(sql);
            // Drop an unexpected successful SceneWriter before any cleanup.
            let result = joined.map(|result| result.map(drop));
            (
                arrived,
                primary_busy,
                observer_current,
                result,
                survived,
                original_survived,
                after,
            )
        });
    drop(io);
    let queued_survived = epoch.current();
    let shutdown = watcher.shutdown();
    let result = match result {
        Ok(result) => result,
        Err(panic) => std::panic::resume_unwind(panic),
    };
    shutdown.unwrap();
    assert!(arrived && primary_busy && observer_current,
        "actual provisional-drop/primary-exclusion/independent packet-observer preconditions failed");
    assert_eq!(before, (0, 0));
    assert_eq!(
        after.unwrap(),
        before,
        "no Commit step/effect may start during admission refusal"
    );
    assert_eq!(
        result
            .err()
            .expect("primary SQL must defer Commit entry")
            .code,
        "authority-busy"
    );
    assert!(survived && original_survived && queued_survived,
        "pre-effect Commit SQL exclusion revoked the original unaffected audio epoch before any Commit metadata or Core mutation");
}

#[test]
fn commit_begin_sql_reacquire_defers_without_parking_full_scene_writer() {
    let m = fixture(true);
    let operation = m.command.operation_id.clone();
    let origin = m.origin.clone();
    let sources = m.sources.clone();
    let started = m.started.clone();
    let source = m.sources[0].clone();
    let healthy_source = m.sources[1].clone();
    let store = Arc::new(m.f.store);
    let register = |input: &Arc<NativeMediaOrigin>| {
        let weak = Arc::downgrade(&store);
        let origin = input.clone();
        store
            .register_native_media_snapshot_for(input, None, move || {
                weak.upgrade()
                    .ok_or_else(Error::forbidden)?
                    .try_native_media_read(&origin, None)
            })
            .unwrap()
    };
    let snapshot = register(&source);
    let healthy_snapshot = register(&healthy_source);
    let mut watcher = store.start_media_authorization_watcher().unwrap();
    let io = admit(&snapshot);
    let epoch = io.packet_epoch().unwrap();
    let stamp = io.snapshot_stamp().unwrap();
    drop(io); // Only the affected original must drain for its exact Writer.
    let healthy_io = admit(&healthy_snapshot);
    let healthy_epoch = healthy_io.packet_epoch().unwrap();
    let healthy_stamp = healthy_io.snapshot_stamp().unwrap();
    assert!(healthy_io.current() && healthy_epoch.current());
    let read_scope =
        crate::AuthorityScope::connection(source.connection_id(), crate::AuthorityMode::Read)
            .unwrap();
    let until = Instant::now() + Duration::from_secs(2);
    let (entered, ready) = std::sync::mpsc::sync_channel(1);
    let (release, resume) = std::sync::mpsc::sync_channel(1);
    let (done, finished) = std::sync::mpsc::sync_channel(1);
    let (
        arrived,
        returned_while_sql_held,
        writer_released,
        old_epoch_revoked,
        healthy_during,
        before,
        joined,
    ) = std::thread::scope(|workers| {
        let worker = workers.spawn(|| {
            let result = Store::with_native_replacement_commit_entry_probe_for_test(
                "begin-sql",
                move || {
                    let _ = entered.send(());
                    let _ = resume.recv_timeout(until.saturating_duration_since(Instant::now()));
                },
                || {
                    let read = scene(&store, &origin, &sources, until)?;
                    let result = store.begin_native_replacement_commit(origin.clone(), &read);
                    drop(read); // The SAME failed original must release its complete Writer.
                    result
                },
            );
            let _ = done.send(());
            result
        });
        let arrived = ready.recv_timeout(Duration::from_secs(1)).is_ok();
        let sql = if arrived {
            store.connection().ok()
        } else {
            None
        };
        let _ = release.send(());
        let returned = finished.recv_timeout(Duration::from_millis(100)).is_ok();
        let shared = store.try_acquire_authority(std::slice::from_ref(&read_scope));
        let writer_released = shared.is_ok();
        drop(shared);
        let old_epoch_revoked = !epoch.current() && snapshot.admit_original(stamp).is_err();
        let healthy_original = healthy_snapshot.admit_original(healthy_stamp);
        let healthy_during = healthy_io.current()
            && healthy_epoch.current()
            && healthy_original.as_ref().is_ok_and(|read| read.current());
        drop(healthy_original);
        let before = counts(&store, &operation);
        drop(sql); // Old blocking owner ALWAYS gets released before join/assertion.
        let joined = worker.join();
        (
            arrived,
            returned,
            writer_released,
            old_epoch_revoked,
            healthy_during,
            before,
            joined,
        )
    });
    let result = match joined {
        Ok(result) => result,
        Err(panic) => {
            drop(healthy_io);
            watcher.shutdown().unwrap();
            std::panic::resume_unwind(panic);
        }
    };
    let (first_code, attempt) = match result {
        Ok(attempt) => (None, Ok(attempt)),
        Err(e) => {
            let code = e.code.clone();
            // Only the definite pre-effect Busy case retries the SAME root;
            // no successful/uncertain attempt is ever repeated by this fixture.
            let attempt = if code == "authority-busy" {
                scene(&store, &origin, &sources, until).and_then(|read| {
                    let result = store.begin_native_replacement_commit(origin.clone(), &read);
                    drop(read);
                    result
                })
            } else {
                Err(e)
            };
            (Some(code), attempt)
        }
    };
    let once = counts(&store, &operation);
    let healthy_original = healthy_snapshot.admit_original(healthy_stamp);
    let healthy_before_negative_cleanup = healthy_io.current()
        && healthy_epoch.current()
        && healthy_original.as_ref().is_ok_and(|read| read.current());
    drop(healthy_original);
    drop(healthy_io);
    // Negative NotSubmitted settlement deliberately retains its broad metadata
    // Writer. The unaffected survival claim is measured BEFORE that separate
    // broad cleanup, but after the actual SQL refusal + same-root Begin retry.
    let settled = attempt.and_then(|attempt| settle_not_submitted(&store, &started, attempt));
    let old_epoch_still_revoked = !epoch.current() && snapshot.admit_original(stamp).is_err();
    let shutdown = watcher.shutdown();
    shutdown.unwrap();
    settled.unwrap();
    assert!(
        arrived,
        "the actual affected Connection Writer must precede the primary-SQL race"
    );
    assert_eq!(before.unwrap(), (0, 0));
    assert!(healthy_during&&healthy_before_negative_cleanup,"same unaffected source1 epoch/stamp/current read changed during failed SQL Begin or same-root retry");
    assert!(
        old_epoch_revoked && old_epoch_still_revoked,
        "a real Writer announcement must never revive the original queued stamp"
    );
    assert_eq!(
        once.unwrap(),
        (1, 1),
        "same original root begins exactly one Commit attempt"
    );
    assert!(returned_while_sql_held && writer_released,
        "pre-effect Commit begin parked its affected Connection Writer behind primary SQL before any Commit step/Core IO");
    assert_eq!(first_code.as_deref(), Some("authority-busy"));
    // A shared file probe proves Writer release only. It is deliberately NOT
    // presented as proof of fresh media publication/IO while SQL stayed held.
}

#[test]
fn read_only_commit_refusal_cannot_masquerade_as_definite_sql_deferral() {
    let m = fixture(true);
    let read = metadata(|| {
        m.f.store
            .try_native_replacement_scene_read(&m.origin, &m.sources)
    })
    .unwrap();
    let before = counts(&m.f.store, &m.command.operation_id).unwrap();
    let sql = m.f.store.connection().unwrap();
    let result =
        m.f.store
            .try_begin_native_replacement_commit(m.origin.clone(), &read);
    let code = result.as_ref().err().map(|e| e.code.clone());
    drop(result);
    drop(sql);
    drop(read);
    assert_eq!(
        code.as_deref(),
        Some("forbidden"),
        "a Read cannot gain Commit authority through the SQL-busy branch"
    );
    assert_eq!(counts(&m.f.store, &m.command.operation_id).unwrap(), before);
    assert_eq!(before, (0, 0));
}

#[test]
fn commit_begin_sql_body_error_is_terminal_and_rolls_back_original_attempt() {
    let m = fixture(true);
    m.f.store
        .connection()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER commit_begin_body_fault BEFORE INSERT ON native_replacement_steps
         WHEN NEW.phase='commit' BEGIN SELECT RAISE(ABORT,'actual Commit begin body fault'); END;",
        )
        .unwrap();
    let until = Instant::now() + Duration::from_secs(2);
    let read = scene(&m.f.store, &m.origin, &m.sources, until).unwrap();
    let result =
        m.f.store
            .try_begin_native_replacement_commit(m.origin.clone(), &read);
    let code = result.as_ref().err().map(|e| e.code.clone());
    drop(result);
    drop(read);
    // This error happened AFTER the Unit, transaction and effect INSERT. Its
    // exact original transaction rolls back; it must NEVER be typed None.
    let rolled_back = counts(&m.f.store, &m.command.operation_id).unwrap();
    m.f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER commit_begin_body_fault")
        .unwrap();
    assert_eq!(code.as_deref(), Some("storage-unavailable"));
    assert_eq!(rolled_back, (0, 0));
    // A separate explicit caller action can begin the same root after the
    // injected fault is removed; the admission loop itself never retries Err.
    let read = scene(&m.f.store, &m.origin, &m.sources, until).unwrap();
    let attempt =
        m.f.store
            .try_begin_native_replacement_commit(m.origin.clone(), &read)
            .unwrap()
            .expect("SQL is free");
    drop(read);
    let once = counts(&m.f.store, &m.command.operation_id).unwrap();
    let settled = settle_not_submitted(&m.f.store, &m.started, attempt);
    settled.unwrap();
    assert_eq!(once, (1, 1));
}
