//! Actual Store scope ordering and controlled committed observations. No
//! physical media or decoded-readiness qualification is inferred here.
use super::*;

#[test]
fn recorded_commit_replay_preserves_actual_media_read_and_original_queued_epoch() {
    let m = fixture(true);
    let captured = activation_barrier::captured(&m);
    m.f.store
        .project_native_replacement_committed(&captured, &MetadataOnlyInstalled)
        .unwrap();
    let source = m.sources[1].clone();
    let store = Arc::new(m.f.store);
    let weak_store = Arc::downgrade(&store);
    let original_source = source.clone();
    let snapshot = store
        .register_native_media_snapshot_for(&source, None, move || {
            weak_store
                .upgrade()
                .ok_or_else(Error::forbidden)?
                .try_native_media_read(&original_source, None)
        })
        .unwrap();
    let registry = store.media_authorization_registry();
    let mut worker = registry
        .start_watcher(std::time::Duration::from_millis(1))
        .unwrap();
    registry.enable_after_writer_boundary().unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(1);
    let held = loop {
        if let Ok(read) = snapshot.admit() {
            break read;
        }
        assert!(std::time::Instant::now() < until);
        std::thread::sleep(std::time::Duration::from_millis(1));
    };
    let epoch = held.packet_epoch().unwrap();
    let before = Store::inspect_snapshot(store.path()).unwrap();
    // Same immutable captured result as the watcher re-observes after public
    // activation. This must not request a Writer or revoke queued source work.
    let repeated = store.record_native_replacement_committed(&captured);
    let original_stayed_current = held.current() && epoch.current();
    drop(held);
    let queued_stayed_current = epoch.current();
    let after = Store::inspect_snapshot(store.path()).unwrap();
    worker.shutdown().unwrap();
    assert!(
        original_stayed_current && queued_stayed_current,
        "read-only captured outcome replay revoked original media epoch"
    );
    assert_eq!(repeated.unwrap().state, OperationState::Completed);
    assert_eq!(before, after, "captured replay changed durable state");
}

#[test]
fn first_known_commit_capture_still_requires_writer_and_records_once_after_io_drains() {
    let m = fixture(true);
    let write =
        m.f.store
            .try_native_replacement_scene_write(&m.origin, &m.sources)
            .unwrap();
    let attempt =
        m.f.store
            .begin_native_replacement_commit(m.origin.clone(), &write)
            .unwrap();
    let expected = attempt.expected().unwrap();
    let captured = attempt
        .committed(expected.clone(), &ready(&write, &expected))
        .unwrap();
    drop(write);
    let held =
        m.f.store
            .try_native_media_read(&m.sources[1], None)
            .unwrap();
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    let refused = m.f.store.record_native_replacement_committed(&captured);
    assert_eq!(refused.unwrap_err().code, "authority-busy");
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
    drop(held);
    let recorded =
        m.f.store
            .record_native_replacement_committed(&captured)
            .unwrap();
    let once = Store::inspect_snapshot(m.f.store.path()).unwrap();
    assert_eq!(
        m.f.store
            .record_native_replacement_committed(&captured)
            .unwrap(),
        recorded
    );
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), once);
}

#[test]
fn public_replacement_commit_refuses_read_only_scene_without_starting_an_effect() {
    let m = fixture(true);
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    let read =
        m.f.store
            .try_native_replacement_scene_read(&m.origin, &m.sources)
            .unwrap();
    let result =
        m.f.store
            .begin_native_replacement_commit(m.origin.clone(), &read);
    let code = result.as_ref().err().map(|error| error.code.clone());
    drop(result);
    drop(read);
    assert_eq!(
        code.as_deref(),
        Some("forbidden"),
        "public replacement accepted only a preparation Read interval"
    );
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
}

#[test]
fn public_replacement_write_drains_old_readers_then_rederives_and_commits_once() {
    let m = fixture(true);
    let old =
        m.f.store
            .try_native_media_read(&m.sources[1], None)
            .unwrap();
    let blocked =
        m.f.store
            .try_native_replacement_scene_write(&m.origin, &m.sources);
    let code = blocked.as_ref().err().map(|error| error.code.clone());
    drop(blocked);
    drop(old);
    assert_eq!(
        code.as_deref(),
        Some("authority-busy"),
        "live source reader did not exclude the actual movement writer"
    );

    // If any provisional Read alias survived inside scene_write, this new
    // original Write would wait on itself rather than succeeding now.
    let write =
        m.f.store
            .try_native_replacement_scene_write(&m.origin, &m.sources)
            .unwrap();
    let blocked = m.f.store.try_native_media_read(&m.sources[1], None);
    let packet_code = blocked.as_ref().err().map(|error| error.code.clone());
    drop(blocked);
    let attempt =
        m.f.store
            .begin_native_replacement_commit(m.origin.clone(), &write)
            .unwrap();
    let expected = attempt.expected().unwrap();
    let observed = ready(&write, &expected);
    let captured = attempt.committed(expected, &observed).unwrap();
    drop(write);
    assert_eq!(packet_code.as_deref(), Some("authority-busy"));
    let connections_before_capture = m.f.count("connections");
    let effects_before_capture = m.f.count("operation_effects");
    let ready_events = || -> u64 {
        m.f.store.connection().unwrap().query_row(
            "SELECT count(*) FROM journal WHERE kind='connection_ready' AND operation_record_id=?1",
            [captured.activation().origin().offer_record_id()],
            |row| row.get(0),
        ).unwrap()
    };
    let recorded =
        m.f.store
            .record_native_replacement_committed(&captured)
            .unwrap();
    // An actual captured engine result is durable, but only the independent
    // exact-installation projection may publish Ready and complete the operation.
    // Preserve the public completion assertion below, after that required step.
    assert_eq!(recorded.state, OperationState::Preparing);
    assert!(recorded.result.is_none());
    let commit_effects = recorded
        .effects
        .iter()
        .filter(|effect| effect.provider == "native-replacement-commit")
        .collect::<Vec<_>>();
    assert_eq!(commit_effects.len(), 1);
    assert_eq!(commit_effects[0].state, EffectState::Confirmed);
    assert_eq!(commit_effects[0].attempt, 1);
    assert_eq!(m.f.count("connections"), connections_before_capture);
    assert_eq!(m.f.count("operation_effects"), effects_before_capture);
    assert_eq!(ready_events(), 0);
    assert!(m
        .f
        .store
        .try_native_media_read(captured.activation().origin(), None)
        .is_err());
    let capture_snapshot = Store::inspect_snapshot(m.f.store.path()).unwrap();
    assert_eq!(
        m.f.store
            .record_native_replacement_committed(&captured)
            .unwrap(),
        recorded
    );
    assert_eq!(
        Store::inspect_snapshot(m.f.store.path()).unwrap(),
        capture_snapshot
    );

    // Existing Store-only fixture; it does not assert actual Service installation
    // or physical media. Real installation and recheck controls remain separate.
    let origin =
        m.f.store
            .project_native_replacement_committed(&captured, &MetadataOnlyInstalled)
            .unwrap();
    assert_eq!(
        origin.connection_id(),
        captured.activation().origin().connection_id()
    );
    let completed =
        m.f.store
            .record_native_replacement_committed(&captured)
            .unwrap();
    assert_eq!(completed.state, OperationState::Completed);
    assert_eq!(
        completed.result.as_ref().unwrap()["activation_state"],
        "ready"
    );
    assert_eq!(m.f.count("operation_effects"), effects_before_capture);
    assert!(completed.effects.iter().all(|effect| effect.attempt == 1));
    assert!(ready_events() > 0);
    let completed_snapshot = Store::inspect_snapshot(m.f.store.path()).unwrap();
    m.f.store
        .project_native_replacement_committed(&captured, &MetadataOnlyInstalled)
        .unwrap();
    assert_eq!(
        Store::inspect_snapshot(m.f.store.path()).unwrap(),
        completed_snapshot
    );
    assert!(m
        .f
        .store
        .try_native_media_read(&m.sources[1], None)
        .unwrap()
        .current());
    assert_eq!(
        m.f.store
            .record_native_replacement_committed(&captured)
            .unwrap(),
        completed
    );
}

#[test]
fn public_replacement_writer_keeps_original_expiry_and_rejects_foreign_scene() {
    let m = fixture(true);
    let other = fixture(true);
    let provisional =
        m.f.store
            .try_native_replacement_scene_read(&m.origin, &m.sources)
            .unwrap();
    let cutoff = provisional.media().source().expires_at();
    drop(provisional);
    let write =
        m.f.store
            .try_native_replacement_scene_write(&m.origin, &m.sources)
            .unwrap();
    assert!(write.media().source().expires_at() <= cutoff);
    let foreign = other
        .f
        .store
        .begin_native_replacement_commit(other.origin.clone(), &write);
    let code = foreign.err().map(|error| error.code);
    drop(write);
    assert_eq!(code.as_deref(), Some("forbidden"));
}
