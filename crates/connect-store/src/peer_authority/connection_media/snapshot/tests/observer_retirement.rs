use super::*;
#[test]
fn same_slot_reopen_invalidates_old_memo_despite_equal_sql_counters() {
    let mut f = Fixture::new();
    let memo = Memo::default();
    let calls = std::cell::Cell::new(0);
    let read = || {
        cached(&f.store, &memo, None, |c| {
            calls.set(calls.get() + 1);
            f.derive(c)
        })
    };
    drop(read().unwrap());
    let first = memo.0.lock().unwrap()[0].version;
    let old_address = {
        let c = f.store.packet_connections[0].lock().unwrap();
        &**c as *const Connection as usize
    };
    // The facts remain deliberately retained; no weak callback eviction hides
    // whether the actual generation stamp rejects the old cache entry.
    drop(f.packet_scope.take());
    assert!(f.store.reap_idle_packet_observers().unwrap());
    f.packet_scope = Some(f.store.retain_packet_observers().unwrap());
    let new_address = {
        let c = f.store.packet_connections[0].lock().unwrap();
        &**c as *const Connection as usize
    };
    assert_eq!(
        old_address, new_address,
        "same fixed slot stores the reopened Connection at the same address"
    );
    let before = calls.get();
    drop(
        cached(&f.store, &memo, None, |c| {
            calls.set(calls.get() + 1);
            f.derive(c)
        })
        .unwrap(),
    );
    let second = memo.0.lock().unwrap()[0].version;
    assert_eq!(
        first.external, second.external,
        "SQLite counters restart independently of generation"
    );
    assert_eq!(first.changes, second.changes);
    assert_ne!(
        first.connection, second.connection,
        "address/counter reuse cannot restore old memo validity"
    );
    assert!(
        calls.get() > before,
        "reopened observer must derive fresh current facts"
    );
    assert_eq!(
        memo.0.lock().unwrap().len(),
        1,
        "old generations are pruned on access/publication"
    );
    let writer = f.store.try_acquire_authority(&[f.write.clone()]).unwrap();
    assert!(
        matches!(cached(&f.store,&memo,None,|c|f.derive(c)),Err(e) if e.code=="authority-busy")
    );
    drop(writer);
}
#[test]
fn last_scope_drop_during_file_admission_preserves_original_observer_until_reseal() {
    let mut f = Fixture::new();
    let scope = f.packet_scope.take().unwrap();
    let mut scope = Some(scope);
    let captured = std::cell::Cell::new(None);
    let (value, held) = acquire_with(
        &f.store,
        |c, version| {
            captured.set(Some(version.connection));
            f.derive(c)
        },
        |_| Ok(()),
        |scopes| {
            drop(scope.take());
            assert!(!f.store.reap_idle_packet_observers().unwrap());
            assert!(
                f.store.packet_connections.observe().is_err(),
                "a new operation cannot prolong an unowned generation"
            );
            f.store.try_acquire_authority(scopes)
        },
    )
    .unwrap();
    assert_eq!(value, 1);
    assert!(captured.get().is_some());
    // Actual resource close is driven only outside the returned fresh interval.
    drop(held);
    assert!(f.store.reap_idle_packet_observers().unwrap());
    assert!(f.store.try_acquire_authority(&[f.write]).is_ok());
}
