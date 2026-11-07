//! Transaction claims retain one slot, never its SQL mutex or a media grant.
use super::*;
#[test]
fn four_compound_reservations_keep_original_bound_release_sql_and_drop_exact_slot() {
    let f = Fixture::new();
    let physical = f.pool.scope().unwrap();
    let observations = (0..OBSERVERS)
        .map(|_| f.pool.observe().unwrap())
        .collect::<Vec<_>>();
    let mut reservations = observations
        .iter()
        .map(|o| Some(f.pool.reserve(o).unwrap()))
        .collect::<Vec<_>>();
    let ids = reservations
        .iter()
        .map(|r| r.as_ref().unwrap().id)
        .collect::<Vec<_>>();
    assert_eq!(
        ids.iter()
            .map(|id| id.slot)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        4
    );
    for id in &ids {
        drop(
            f.pool.0.slots[id.slot]
                .try_lock()
                .expect("reservation is not SQL custody"),
        );
    }
    let fifth = f.pool.observe().unwrap();
    assert!(
        matches!(f.pool.reserve(&fifth),Err(e) if e.code=="authority-busy" && e.message=="All current packet observers are busy")
    );
    assert!(matches!(f.pool.connection(&fifth,None),Err(e) if e.code=="authority-busy"));
    let released = ids[1];
    drop(reservations[1].take());
    let replacement = f.pool.reserve(&fifth).unwrap();
    assert_eq!(replacement.id, released);
    assert_eq!(
        value(&f.pool.connection_reserved(&fifth, &replacement).unwrap()),
        7
    );
    drop(replacement);
    drop(reservations);
    drop(observations);
    drop(fifth);
    drop(physical);
    assert!(f.pool.reap().unwrap());
}
#[test]
fn reserved_connection_refuses_other_observation_pool_or_generation_and_unwind_releases() {
    let f = Fixture::new();
    let physical = f.pool.scope().unwrap();
    let a = f.pool.observe().unwrap();
    let b = f.pool.observe().unwrap();
    let reserved = f.pool.reserve(&a).unwrap();
    assert!(f.pool.connection_reserved(&b, &reserved).is_err());
    let foreign = Fixture::new();
    let foreign_scope = foreign.pool.scope().unwrap();
    assert!(foreign.pool.connection_reserved(&a, &reserved).is_err());
    let id = reserved.id;
    drop(reserved);
    drop(a);
    drop(b);
    let escaped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let original = f.pool.observe().unwrap();
        let reserved = f.pool.reserve(&original).unwrap();
        assert_eq!(reserved.id, id);
        panic!("controlled cancellation between original SQL intervals");
    }));
    assert!(escaped.is_err());
    let read = f.pool.observe().unwrap();
    let guards = (0..OBSERVERS)
        .map(|_| f.pool.connection(&read, None).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(guards.len(), 4);
    drop(guards);
    drop(read);
    drop(physical);
    assert!(f.pool.reap().unwrap());
    let next = f.pool.scope().unwrap();
    let current = f.pool.observe().unwrap();
    let newer = f.pool.reserve(&current).unwrap();
    assert_ne!(newer.id.generation, id.generation);
    assert!(f.pool.connection(&current, Some(id)).is_err());
    drop(newer);
    drop(current);
    drop(next);
    assert!(f.pool.reap().unwrap());
    drop(foreign_scope);
}
#[test]
fn concurrent_original_transactions_reserve_distinct_slots_and_join_without_sql_retention() {
    use std::sync::mpsc;
    let f = Fixture::new();
    let physical = f.pool.scope().unwrap();
    let observation = f.pool.observe().unwrap();
    std::thread::scope(|scope| {
        let (entered_tx, entered_rx) = mpsc::channel();
        let mut releases = Vec::new();
        let mut owners = Vec::new();
        for _ in 0..2 {
            let (release, until_release) = mpsc::channel();
            releases.push(release);
            let entered = entered_tx.clone();
            let pool = &f.pool;
            let observation = &observation;
            owners.push(scope.spawn(move || {
                let reserved = pool.reserve(observation).unwrap();
                let id = reserved.id;
                {
                    let sql = pool.connection_reserved(observation, &reserved).unwrap();
                    assert_eq!(value(&sql), 7);
                }
                drop(
                    pool.0.slots[id.slot]
                        .try_lock()
                        .expect("SQL released during transaction pause"),
                );
                entered.send(id).unwrap();
                // A failed main assertion still joins each original owner: no
                // unbounded barrier can strand the reservation on a test panic.
                assert!(until_release.recv_timeout(Duration::from_secs(2)).is_ok());
            }));
        }
        let first = entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let second = entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_ne!(first.slot, second.slot);
        for release in releases {
            release.send(()).unwrap();
        }
        for owner in owners {
            owner.join().unwrap();
        }
    });
    drop(observation);
    drop(physical);
    assert!(f.pool.reap().unwrap());
}
