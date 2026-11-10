//! Pure original-source binding against real authenticated native origins.
use super::*;
use crate::{NativeMediaRead, NativeMediaSnapshot};
use std::time::{Duration, Instant};

fn current(snapshot: &NativeMediaSnapshot) -> NativeMediaRead {
    let until = Instant::now() + Duration::from_secs(1);
    loop {
        if let Ok(read) = snapshot.admit() {
            return read;
        }
        assert!(
            Instant::now() < until,
            "original live snapshot did not publish"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn with_original_source(
    run: impl FnOnce(&Store, &NativeMediaSnapshot, &NativeMediaOrigin, &NativeMediaOrigin),
) {
    let m = fixture_scoped(true, &["stream.subscribe"]);
    let source = m.sources[1].clone();
    let other = m.sources[0].clone();
    // All original setup and signed resource selection happened before this
    // worker is enabled. Remaining fixture fields retain its original directory
    // and packet observation scope throughout shutdown and source release.
    let store = Arc::new(m.f.store);
    let weak_store = Arc::downgrade(&store);
    let captured_source = source.clone();
    let snapshot = store
        .register_native_media_snapshot_for(&source, None, move || {
            weak_store
                .upgrade()
                .ok_or_else(Error::forbidden)?
                .try_native_media_read(&captured_source, None)
        })
        .unwrap();
    let registry = store.media_authorization_registry();
    let mut worker = registry.start_watcher(Duration::from_millis(1)).unwrap();
    registry.enable_after_writer_boundary().unwrap();
    run(&store, &snapshot, &source, &other);
    worker.shutdown().unwrap();
}

#[test]
fn unary_snapshot_preserves_exact_original_source_epoch_and_earliest_expiry() {
    with_original_source(|store, snapshot, source, _| {
        let original = current(snapshot);
        let original_epoch = original.packet_epoch().unwrap();
        // Real Store SQL and every packet observer are unavailable. An already
        // published unary source admission and pure binding must still work.
        let _primary = store.connection().unwrap();
        let _observers = store
            .packet_connections
            .iter()
            .map(|connection| connection.lock().unwrap())
            .collect::<Vec<_>>();
        let fresh = snapshot.admit().unwrap();
        let bound = fresh.bound_to_original_source(&original, source).unwrap();
        assert!(bound.current());
        assert!(bound.source().is_none());
        assert_eq!(
            bound.target().runtime_connection_id,
            source.runtime_connection_id()
        );
        assert!(bound.expires_at() <= original.expires_at());
        assert!(bound.packet_epoch().unwrap().current() && original_epoch.current());
    });
}

#[test]
fn unary_snapshot_cannot_bind_a_different_real_source_origin() {
    with_original_source(|_, snapshot, _, other| {
        let original = current(snapshot);
        let fresh = current(snapshot);
        assert_eq!(
            fresh
                .bound_to_original_source(&original, other)
                .err()
                .unwrap()
                .code,
            "forbidden"
        );
        assert!(
            original.current(),
            "foreign binding failure must not alter the original owner"
        );
    });
}

#[test]
fn unary_snapshot_refuses_expired_original_without_renewing_live_publication() {
    with_original_source(|_, snapshot, source, _| {
        let original = current(snapshot);
        let fresh = current(snapshot);
        let result = {
            let _clock = crate::test_clock::Clock::at(original.expires_at());
            fresh.bound_to_original_source(&original, source)
        };
        assert_eq!(result.err().unwrap().code, "forbidden");
        assert!(
            original.current(),
            "the scoped clock must restore its original owner interval"
        );
    });
}

#[test]
fn original_presence_lifetime_snapshot_observes_without_sql_and_rejects_foreign_origin() {
    let m = fixture_scoped(true, &["stream.subscribe"]);
    let source = m.sources[1].clone();
    let other = m.sources[0].clone();
    let store = Arc::new(m.f.store);
    let snapshot = store
        .try_register_native_connection_lifetime_snapshot(source.clone())
        .unwrap();
    let registry = store.media_authorization_registry();
    let mut worker = registry.start_watcher(Duration::from_millis(1)).unwrap();
    registry.enable_after_writer_boundary().unwrap();
    let until = Instant::now() + Duration::from_secs(1);
    loop {
        match snapshot.observe_original(&source) {
            Ok(true) => break,
            Err(error) if error.code == "authority-busy" && Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(1))
            }
            other => panic!(
                "actual Presence lifetime did not publish: {}",
                other.is_ok()
            ),
        }
    }
    {
        let _primary = store.connection().unwrap();
        let _observers = store
            .packet_connections
            .iter()
            .map(|connection| connection.lock().unwrap())
            .collect::<Vec<_>>();
        let generation = snapshot.observe_original_generation(&source).unwrap();
        for _ in 0..16 {
            assert_eq!(snapshot.observe_original(&source).unwrap(), true);
            assert!(snapshot.observe_original_generation(&source).unwrap() == generation);
        }
        assert_eq!(
            snapshot.observe_original(&other).unwrap_err().code,
            "forbidden"
        );
    }
    worker.shutdown().unwrap();
}
