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
