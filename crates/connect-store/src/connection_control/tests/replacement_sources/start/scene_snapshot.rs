//! Real precommit scene/source facts and original IO custody, no adapter IO.
use super::*;
use crate::{AuthorityMode, AuthorityScope, AuthorityWait, NativeReplacementSceneSnapshot};
use std::time::{Duration, Instant};
fn admitted(
    snapshot: &NativeReplacementSceneSnapshot,
) -> crate::NativeReplacementSceneSnapshotLease {
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        match snapshot.admit() {
            Ok(io) => return io,
            Err(error) if error.code == "authority-busy" && Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(1))
            }
            Err(error) => panic!("original scene admission refused: {}", error.code),
        }
    }
}
// Enrollment is effect-free but deliberately nonblocking with the one owned
// watcher. Keep the exact original context and one fixed setup deadline; never
// apply this retry to actual steady admission or held-SQL delivery assertions.
fn scene_snapshot(
    store: &Arc<Store>,
    origin: &Arc<crate::NativeReplacementMediaOrigin>,
    sources: &[Arc<crate::NativeMediaOrigin>],
) -> NativeReplacementSceneSnapshot {
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        match store.try_register_native_replacement_scene_snapshot(origin.clone(), sources.to_vec())
        {
            Ok(snapshot) => return snapshot,
            Err(error) if error.code == "authority-busy" && Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(1))
            }
            Err(error) => panic!("original scene enrollment refused: {}", error.code),
        }
    }
}
fn source_snapshot(
    store: &Arc<Store>,
    source: &Arc<crate::NativeMediaOrigin>,
) -> crate::NativeMediaSnapshot {
    let weak = Arc::downgrade(store);
    let original = source.clone();
    store
        .register_native_media_snapshot_for(source, None, move || {
            weak.upgrade()
                .ok_or_else(crate::Error::forbidden)?
                .try_native_media_read(&original, None)
        })
        .unwrap()
}
fn source_admitted(snapshot: &crate::NativeMediaSnapshot) -> crate::NativeMediaRead {
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        match snapshot.admit() {
            Ok(read) => return read,
            Err(error) if error.code == "authority-busy" && Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(1))
            }
            Err(error) => panic!("original source admission refused: {}", error.code),
        }
    }
}
#[test]
fn private_scene_delivery_uses_real_original_source_without_primary_or_observer_sql() {
    let (f, _, _, _, origin, sources) = resource_fixture_scene();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let store = Arc::new(f.store);
    let mut watcher = store.start_media_authorization_watcher().unwrap();
    let source = source_snapshot(&store, &sources[0]);
    let original = source_admitted(&source);
    let legacy = store.try_native_media_read(&sources[0], None).unwrap();
    let scene = scene_snapshot(&store, &origin, &sources);
    drop(admitted(&scene));
    assert!(original.packet_epoch().is_some());
    assert_eq!(
        scene.admit_under(&sources[0], &legacy).err().unwrap().code,
        "forbidden"
    );
    assert_eq!(
        scene
            .admit_under(&sources[1], &original)
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    drop(legacy);
    let sql = store.connection().unwrap();
    let observation = store.packet_connections.observe().unwrap();
    let observers: Vec<_> = (0..crate::packet_observation::OBSERVERS)
        .map(|_| store.try_packet_connection(&observation).unwrap())
        .collect();
    // Existing resolver is really excluded, while the exact same delivery
    // facts and original ingress custody remain immediately usable.
    assert_eq!(
        store
            .try_native_replacement_scene_read(&origin, &sources)
            .err()
            .unwrap()
            .code,
        "authority-busy"
    );
    for _ in 0..1000 {
        let io = scene.admit_under(&sources[0], &original).unwrap();
        assert!(io.current() && io.packet_epoch().current());
        assert!(io.expires_at() <= original.expires_at());
        assert!(io
            .scene()
            .existing(sources[0].runtime_connection_id(), None)
            .is_some());
        drop(io);
    }
    let io = scene.admit_under(&sources[0], &original).unwrap();
    let scene_cutoff = io.expires_at();
    let epoch = io.packet_epoch();
    let source_epoch = original.packet_epoch().unwrap();
    assert!(
        scene_cutoff < original.expires_at(),
        "this fixture must exercise scene expiry before original source expiry"
    );
    drop(io);
    let expiry = Instant::now() + Duration::from_secs(6);
    while epoch.current() && Instant::now() < expiry {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        !epoch.current(),
        "queued scene metadata extended the earlier scene expiry"
    );
    // Only the short full-scene projection expired. Its rederivation is really
    // excluded by the original SQL owners; that is transient scene absence,
    // not definitive loss of the still-current original ingress permission.
    assert!(crate::now() >= scene_cutoff);
    assert!(original.current() && source_epoch.current());
    assert_eq!(scene.admit().err().unwrap().code, "authority-busy");
    assert_eq!(
        scene
            .admit_under(&sources[0], &original)
            .err()
            .unwrap()
            .code,
        "authority-busy"
    );
    drop(observers);
    drop(sql);
    drop(original);
    drop(scene);
    drop(source);
    watcher.shutdown().unwrap();
    assert_eq!(Store::inspect_snapshot(store.path()).unwrap(), before);
}
#[test]
fn private_scene_actual_io_drains_writer_and_queued_epochs_never_borrow_republication() {
    let (f, _, command, _, origin, sources) = resource_fixture_scene();
    let store = Arc::new(f.store);
    let mut watcher = store.start_media_authorization_watcher().unwrap();
    let source = source_snapshot(&store, &sources[0]);
    let original = source_admitted(&source);
    let original_epoch = original.packet_epoch().unwrap();
    let scene = scene_snapshot(&store, &origin, &sources);
    drop(admitted(&scene));
    let io = scene.admit_under(&sources[0], &original).unwrap();
    let queued = io.packet_epoch();
    let scope =
        AuthorityScope::conversation(&command.conversation_id, AuthorityMode::Write).unwrap();
    assert_eq!(
        store
            .try_acquire_authority(&[scope.clone()])
            .unwrap_err()
            .code,
        "authority-busy"
    );
    assert!(!io.current() && !queued.current() && !original_epoch.current());
    // This separate path DID lose its original ingress epoch, so refusal is
    // definitive before consulting any newly refreshing scene publication.
    assert_eq!(
        scene
            .admit_under(&sources[0], &original)
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    drop(original);
    // The only surviving original ingress Read is now the actual IO lease.
    // Invalidation cannot let the writer commit until that lease drains.
    assert_eq!(
        store
            .try_acquire_authority(&[scope.clone()])
            .unwrap_err()
            .code,
        "authority-busy"
    );
    drop(io);
    let writer = store
        .acquire_authority(&[scope], &AuthorityWait::new(Duration::from_secs(1)))
        .unwrap();
    assert!(scene.admit().is_err());
    drop(writer);
    let next_source = source_admitted(&source);
    drop(admitted(&scene));
    let next_io = scene.admit_under(&sources[0], &next_source).unwrap();
    assert!(next_io.current() && next_io.packet_epoch().current());
    assert!(!queued.current() && !original_epoch.current());
    drop(next_io);
    drop(next_source);
    drop(scene);
    drop(source);
    watcher.shutdown().unwrap();
}
#[test]
fn private_scene_revoked_original_source_remains_refused_and_epoch_expires_at_earliest_fact() {
    let (f, _, _, _, origin, sources) = resource_fixture_scene();
    let admin = f.store.revalidate(&f.admin).unwrap();
    let key = f.keys[0].clone();
    let store = Arc::new(f.store);
    let mut watcher = store.start_media_authorization_watcher().unwrap();
    let source = source_snapshot(&store, &sources[0]);
    let original = source_admitted(&source);
    let scene = scene_snapshot(&store, &origin, &sources);
    drop(admitted(&scene));
    let io = scene.admit_under(&sources[0], &original).unwrap();
    let queued = io.packet_epoch();
    assert!(io.expires_at() <= original.expires_at());
    assert!(io.expires_at() <= io.scene().media().source().expires_at());
    for origin in &sources {
        assert!(
            io.expires_at()
                <= io
                    .scene()
                    .existing(origin.runtime_connection_id(), None)
                    .unwrap()
                    .expires_at()
        );
    }
    drop(io);
    drop(original);
    store.revoke(&admin, &key).unwrap();
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        match scene.admit() {
            Err(error) if matches!(error.code.as_str(), "unauthorized" | "forbidden") => break,
            Err(error) if error.code == "authority-busy" && Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(1))
            }
            _ => panic!("revoked original scene remained authorized"),
        }
    }
    assert!(!queued.current());
    drop(scene);
    drop(source);
    watcher.shutdown().unwrap();
}

#[test]
fn original_attach_journal_writer_is_released_before_fresh_complete_read_and_old_source_io() {
    let (f, _, _, _, origin, sources) = resource_fixture_scene();
    let store = Arc::new(f.store);
    let mut watcher = store.start_media_authorization_watcher().unwrap();
    let source = source_snapshot(&store, &sources[0]);
    let original = source_admitted(&source);
    let old_epoch = original.packet_epoch().unwrap();
    drop(original);
    let writer = store
        .try_native_replacement_scene_write(&origin, &sources)
        .unwrap();
    let cutoff = writer.media().source().expires_at();
    let attempt = store
        .begin_native_replacement_media_phase(
            origin.clone(),
            crate::NativeReplacementMediaPhase::Attach,
            writer.media(),
        )
        .unwrap();
    assert!(
        !old_epoch.current(),
        "the journal writer really withdrew the old publication"
    );
    drop(writer);
    let read = store
        .try_native_replacement_scene_read(&origin, &sources)
        .unwrap();
    let fresh = source_admitted(&source);
    for _ in 0..100 {
        let next = source.admit().unwrap();
        assert!(read.current() && next.current() && fresh.current());
        assert!(
            !old_epoch.current(),
            "fresh admission never revives queued old metadata"
        );
    }
    // Effect recording is NOT downgraded to Read authority. The exact once-only
    // original SQL attempt still exists; another begin cannot issue an effect.
    assert_eq!(
        store
            .begin_native_replacement_media_phase(
                origin.clone(),
                crate::NativeReplacementMediaPhase::Attach,
                read.media()
            )
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    let sql = store.connection().unwrap();
    let before: (i64, String, String) = sql.query_row(
        "SELECT count(*),min(attempt_id),min(effect_id) FROM native_replacement_media_steps WHERE phase='attach'",
        [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(before.0, 1);
    assert!(
        crate::now() < cutoff,
        "same original short capture still bounds this control"
    );
    drop(sql);
    drop(fresh);
    drop(read);
    // No Core adapter effect is executed in this Store control. Its original
    // unresolved attempt is recorded conservatively, never fabricated applied.
    store
        .record_native_replacement_media_phase(&attempt.uncertain())
        .unwrap();
    let sql = store.connection().unwrap();
    let after: (i64, String, String) = sql.query_row(
        "SELECT count(*),min(attempt_id),min(effect_id) FROM native_replacement_media_steps WHERE phase='attach'",
        [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(before, after);
    drop(sql);
    drop(source_admitted(&source));
    drop(source);
    watcher.shutdown().unwrap();
}

#[test]
fn cancellation_between_original_attach_reservation_and_fresh_read_refuses_without_new_effect() {
    let (f, key, command, _, origin, sources) = resource_fixture_scene();
    let writer = f
        .store
        .try_native_replacement_scene_write(&origin, &sources)
        .unwrap();
    let attempt = f
        .store
        .begin_native_replacement_media_phase(
            origin.clone(),
            crate::NativeReplacementMediaPhase::Attach,
            writer.media(),
        )
        .unwrap();
    drop(writer);
    cancel(&f, &key, &command);
    assert!(f
        .store
        .try_native_replacement_scene_read(&origin, &sources)
        .is_err());
    let before = f.count("native_replacement_media_steps");
    f.store
        .record_native_replacement_media_phase(&attempt.uncertain())
        .unwrap();
    assert_eq!(before, 1);
    assert_eq!(f.count("native_replacement_media_steps"), before);
    assert!(f
        .store
        .try_native_replacement_scene_read(&origin, &sources)
        .is_err());
    assert!(
        f.store.try_native_media_read(&sources[0], None).is_ok(),
        "cancelling the unpublished replacement does not end its original source"
    );
}

#[test]
fn credential_revocation_after_attach_begin_is_checked_by_new_complete_read() {
    let (f, _, _, _, origin, sources) = resource_fixture_scene();
    let writer = f
        .store
        .try_native_replacement_scene_write(&origin, &sources)
        .unwrap();
    let attempt = f
        .store
        .begin_native_replacement_media_phase(
            origin.clone(),
            crate::NativeReplacementMediaPhase::Attach,
            writer.media(),
        )
        .unwrap();
    drop(writer);
    let admin = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&admin, &f.keys[0]).unwrap();
    let error = f
        .store
        .try_native_replacement_scene_read(&origin, &sources)
        .err()
        .unwrap();
    assert!(matches!(error.code.as_str(), "unauthorized" | "forbidden"));
    let effects = f.count("operation_effects");
    f.store
        .record_native_replacement_media_phase(&attempt.uncertain())
        .unwrap();
    assert_eq!(f.count("operation_effects"), effects);
    assert!(f
        .store
        .try_native_replacement_scene_read(&origin, &sources)
        .is_err());
}

mod positive_ledger;
