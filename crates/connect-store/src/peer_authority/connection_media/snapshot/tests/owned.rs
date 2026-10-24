use super::*;

#[test]
fn packet_snapshot_owned_reader_rechecks_facts_without_reentry_and_refuses_partial_or_foreign() {
    let f = Fixture::new();
    let foreign = Fixture::new();
    let memo = Memo::default();
    let held = f.store.try_acquire_authority(&[f.read.clone()]).unwrap();
    let partial = f.store.try_acquire_authority(&[]).unwrap();
    let other = foreign
        .store
        .try_acquire_authority(&[foreign.read.clone()])
        .unwrap();
    let store = f.store.clone();
    let scope = f.write.clone();
    let writer = std::thread::spawn(move || {
        store.acquire_authority(&[scope], &crate::AuthorityWait::new(Duration::from_secs(3)))
    });
    crate::authority_fences::test_wait_for_writer(&f.store, &f.write);
    assert_eq!(
        cached(&f.store, &memo, None, |c| f.derive(c))
            .unwrap_err()
            .code,
        "authority-busy"
    );
    for proof in [&partial, &other] {
        assert_eq!(
            cached_under(&f.store, &memo, None, proof, |c| f.derive(c), |_| Ok(()))
                .unwrap_err()
                .code,
            "forbidden"
        );
    }
    let (value, continued) =
        cached_under(&f.store, &memo, None, &held, |c| f.derive(c), |_| Ok(())).unwrap();
    assert_eq!(value, 1);
    assert!(
        Arc::ptr_eq(&held, &continued),
        "reuse only the actual owned file interval"
    );
    // Controlled uncooperative SQL tests capture freshness; supported writers
    // remain blocked by the owned reader throughout this bounded composition.
    f.store
        .connection()
        .unwrap()
        .execute("UPDATE packet_snapshot_test SET value=2", [])
        .unwrap();
    let (value, latest) =
        cached_under(&f.store, &memo, None, &held, |c| f.derive(c), |_| Ok(())).unwrap();
    assert_eq!(value, 2, "owned custody cannot lend stale facts");
    let sql = f.store.connection().unwrap();
    assert_eq!(
        cached_under(&f.store, &memo, None, &held, |c| f.derive(c), |_| Ok(()))
            .unwrap_err()
            .code,
        "authority-busy"
    );
    drop(sql);
    drop(held);
    drop(continued);
    assert!(!writer.is_finished());
    drop(latest);
    drop(writer.join().unwrap().unwrap());
    assert!(f.store.try_acquire_authority(&[f.write]).is_ok());
}
