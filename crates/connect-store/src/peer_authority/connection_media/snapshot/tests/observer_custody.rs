use super::*;

#[test]
fn an_earlier_observer_becoming_free_cannot_replace_the_original_snapshot_observer() {
    for selected in 1..crate::packet_observation::OBSERVERS {
        let f = Fixture::new();
        let mut earlier: Vec<_> = f.store.packet_connections[..selected]
            .iter()
            .map(|observer| observer.lock().unwrap())
            .collect();
        let captured = std::cell::Cell::new(None);
        let calls = std::cell::Cell::new(0);
        let result = acquire_with(
            &f.store,
            |c, stamp| {
                captured.set(Some(stamp.connection));
                calls.set(calls.get() + 1);
                f.derive(c)
            },
            |c| {
                assert_eq!(Some(c as *const Connection as usize), captured.get());
                Ok(())
            },
            |scopes| {
                // The original SQL mutex has ended before actual file admission.
                let original = f.store.packet_connections[selected].try_lock().unwrap();
                assert_eq!(
                    Some(&*original as *const Connection as usize),
                    captured.get()
                );
                drop(original);
                // These now-free observers are not interchangeable with the
                // selected connection's data_version and total_changes counters.
                earlier.clear();
                f.store.try_acquire_authority(scopes)
            },
        );
        let (value, held) = result
            .expect("a free earlier observer must not invalidate an unchanged original snapshot");
        assert_eq!(value, 1);
        assert!(
            (1..=2).contains(&calls.get()),
            "only an actual clock edge may rederive facts"
        );
        assert_eq!(
            f.store
                .try_acquire_authority(&[f.write.clone()])
                .unwrap_err()
                .code,
            "authority-busy"
        );
        drop(held);
        drop(f.store.try_acquire_authority(&[f.write.clone()]).unwrap());
    }
}

#[test]
fn original_observer_contention_refuses_without_substitution_and_releases_the_reader() {
    let f = Fixture::new();
    let mut occupied = None;
    let result = acquire_with(
        &f.store,
        |c, _| f.derive(c),
        |_| Ok(()),
        |scopes| {
            occupied = Some(f.store.packet_connections[0].lock().unwrap());
            // Every other observer remains available with its own counters.
            for observer in &f.store.packet_connections[1..] {
                drop(observer.try_lock().unwrap());
            }
            f.store.try_acquire_authority(scopes)
        },
    );
    assert_eq!(result.unwrap_err().code, "authority-busy");
    drop(f.store.try_acquire_authority(&[f.write.clone()]).unwrap());
    drop(occupied);
    assert_eq!(acquire(&f.store, |c| f.derive(c)).unwrap().0, 1);
}

#[test]
fn original_observer_reacquisition_rechecks_external_data_and_schema_versions() {
    for schema in [false, true] {
        let f = Fixture::new();
        let mut earlier = Some(f.store.packet_connections[0].lock().unwrap());
        let external = Connection::open(f.store.path()).unwrap();
        let result = acquire_with(
            &f.store,
            |c, _| f.derive(c),
            |_| Ok(()),
            |scopes| {
                drop(earlier.take());
                if schema {
                    external
                        .execute_batch("ALTER TABLE packet_snapshot_test ADD COLUMN marker INTEGER")
                        .unwrap();
                } else {
                    external
                        .execute("UPDATE packet_snapshot_test SET value=7", [])
                        .unwrap();
                }
                f.store.try_acquire_authority(scopes)
            },
        );
        assert_eq!(result.unwrap_err().code, "authority-busy");
        drop(f.store.try_acquire_authority(&[f.write.clone()]).unwrap());
        assert_eq!(
            acquire(&f.store, |c| f.derive(c)).unwrap().0,
            if schema { 1 } else { 7 }
        );
    }
}
