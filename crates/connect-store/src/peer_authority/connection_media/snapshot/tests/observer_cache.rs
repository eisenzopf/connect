use super::*;

fn occupy_except(f: &Fixture, selected: usize) -> Vec<std::sync::MutexGuard<'_, Connection>> {
    f.store
        .packet_connections
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != selected)
        .map(|(_, observer)| observer.lock().unwrap())
        .collect()
}

#[test]
fn cached_current_observer_needs_only_one_fresh_file_admission() {
    for paired in [false, true] {
        let mut checked = false;
        for _ in 0..8 {
            let f = Fixture::new();
            let memo = Memo::default();
            let source: MetadataToken = Arc::new(7u8);
            let source = paired.then_some(&source);
            let second = crate::now();
            let derived = std::cell::Cell::new(0);
            let finished = std::cell::Cell::new(0);
            let read = || {
                cached_with(
                    &f.store,
                    &memo,
                    source,
                    None,
                    |c| {
                        derived.set(derived.get() + 1);
                        f.derive(c)
                    },
                    |_, value| {
                        assert_eq!(*value, 1);
                        finished.set(finished.get() + 1);
                        Ok(())
                    },
                )
            };
            for selected in 0..crate::packet_observation::OBSERVERS {
                let occupied = occupy_except(&f, selected);
                drop(read().unwrap());
                drop(occupied);
            }
            let mut admissions = Vec::new();
            for selected in 0..crate::packet_observation::OBSERVERS {
                let occupied = occupy_except(&f, selected);
                let before = f.store.authority.test_admission_attempts();
                let (value, held) = read().unwrap();
                assert_eq!(value, 1);
                admissions.push(f.store.authority.test_admission_attempts() - before);
                assert_eq!(
                    f.store
                        .try_acquire_authority(&[f.write.clone()])
                        .unwrap_err()
                        .code,
                    "authority-busy"
                );
                drop(held);
                drop(f.store.try_acquire_authority(&[f.write.clone()]).unwrap());
                drop(occupied);
            }
            if crate::now() != second {
                continue;
            }
            assert_eq!(derived.get(), crate::packet_observation::OBSERVERS);
            assert_eq!(finished.get(), 2 * crate::packet_observation::OBSERVERS);
            assert_eq!(
                admissions,
                vec![1; crate::packet_observation::OBSERVERS],
                "current metadata for the actual SQL observer must not repeat fresh file admission"
            );
            checked = true;
            break;
        }
        assert!(
            checked,
            "No unchanged clock interval for the observer admission boundary"
        );
    }
}

#[test]
fn another_observers_old_scope_names_cannot_cover_new_current_metadata() {
    let f = Fixture::new();
    let memo = Memo::default();
    let new_read = AuthorityScope::identity("new-packet-scope", AuthorityMode::Read).unwrap();
    let new_write = AuthorityScope::identity("new-packet-scope", AuthorityMode::Write).unwrap();
    let derive = |c: &Connection| {
        let (value, _) = f.derive(c)?;
        Ok((
            value,
            vec![if value == 1 {
                f.read.clone()
            } else {
                new_read.clone()
            }],
        ))
    };
    let occupied = occupy_except(&f, 0);
    drop(cached(&f.store, &memo, None, derive).unwrap());
    drop(occupied);
    let external = Connection::open(f.store.path()).unwrap();
    external
        .execute("UPDATE packet_snapshot_test SET value=2", [])
        .unwrap();
    let occupied = occupy_except(&f, 1);
    assert_eq!(cached(&f.store, &memo, None, derive).unwrap().0, 2);
    let writer = f.store.try_acquire_authority(&[new_write.clone()]).unwrap();
    assert_eq!(
        cached(&f.store, &memo, None, derive).unwrap_err().code,
        "authority-busy",
        "old fresh scope names cannot authorize a current entry with different scopes"
    );
    drop(writer);
    let (value, held) = cached(&f.store, &memo, None, derive).unwrap();
    assert_eq!(value, 2);
    assert!(f.store.authority_covers(&held, &[new_read]));
    assert!(!f.store.authority_covers(&held, &[f.read.clone()]));
    drop(held);
    drop(occupied);
    drop(f.store.try_acquire_authority(&[new_write]).unwrap());
}
