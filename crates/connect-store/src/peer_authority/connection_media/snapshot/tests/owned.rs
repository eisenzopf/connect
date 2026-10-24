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

#[test]
fn packet_owned_sql_interval_refuses_external_and_own_commits_before_finish_without_leaking_custody(
) {
    for external in [false, true] {
        let f = Fixture::new();
        let memo = Memo::default();
        let held = f.store.try_acquire_authority(&[f.read.clone()]).unwrap();
        let owners = Arc::strong_count(&held);
        let mut finished = false;
        let result = cached_under(
            &f.store,
            &memo,
            None,
            &held,
            |c| {
                let facts = f.derive(c)?;
                if external {
                    // A process-local SQL mutex cannot prevent an uncooperative
                    // external connection from committing. Keep the version seal.
                    Connection::open(f.store.path())
                        .unwrap()
                        .execute("UPDATE packet_snapshot_test SET value=2", [])
                        .unwrap();
                } else {
                    c.execute("UPDATE packet_snapshot_test SET value=2", [])
                        .unwrap();
                }
                Ok(facts)
            },
            |_| {
                finished = true;
                Ok(())
            },
        );
        assert_eq!(result.unwrap_err().code, "authority-busy");
        assert!(!finished, "changed facts reached the completion callback");
        assert_eq!(
            Arc::strong_count(&held),
            owners,
            "refusal kept a child reader"
        );
        assert!(
            f.store.try_connection().is_ok(),
            "SQL was retained after refusal"
        );
        let (value, current) =
            cached_under(&f.store, &memo, None, &held, |c| f.derive(c), |_| Ok(())).unwrap();
        assert_eq!(
            value, 2,
            "failed capture was cached or fresh facts were skipped"
        );
        assert!(Arc::ptr_eq(&held, &current));
        assert!(
            f.store.try_connection().is_ok(),
            "SQL must end before media IO"
        );
        drop(current);
        drop(held);
        drop(f.store.try_acquire_authority(&[f.write]).unwrap());
    }
}

#[test]
fn packet_owned_sql_clock_edge_rederives_under_the_same_interval_behind_a_waiting_writer() {
    let f = Fixture::new();
    let memo = Memo::default();
    let held = f.store.try_acquire_authority(&[f.read.clone()]).unwrap();
    let store = f.store.clone();
    let write = f.write.clone();
    let writer = std::thread::spawn(move || {
        store.acquire_authority(&[write], &crate::AuthorityWait::new(Duration::from_secs(3)))
    });
    crate::authority_fences::test_wait_for_writer(&f.store, &f.write);
    let mut calls = 0;
    let (value, current) = cached_under(
        &f.store,
        &memo,
        None,
        &held,
        |c| {
            calls += 1;
            if calls == 1 {
                let second = crate::now();
                while crate::now() == second {
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
            f.derive(c)
        },
        |_| Ok(()),
    )
    .unwrap();
    assert_eq!((value, calls), (1, 2));
    assert!(Arc::ptr_eq(&held, &current));
    assert!(f.store.try_connection().is_ok(), "SQL must end before IO");
    assert!(!writer.is_finished());
    drop(current);
    drop(held);
    drop(writer.join().unwrap().unwrap());
}
