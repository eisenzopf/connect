//! One exact prepared return survives only definite pre-effect admission misses.
use super::*;
#[test]
fn native_return_prebegin_reader_drain_retains_same_reservation_and_first_read_cutoff() {
    let (f, command, key, _parent, sources) = fixture_full();
    let (lease, origin) = prepared(&f, command, &key, &sources);
    let original = Arc::new(lease);
    let original_plan = original.plan().unwrap();
    let alias = f
        .store
        .native_consultation_return_attempt_read(&original)
        .unwrap();
    let first = f
        .store
        .native_consultation_return_attempt_read(&original)
        .unwrap();
    let first_until = first.expires_at();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    assert!(f
        .store
        .try_begin_native_consultation_return_before(original.clone(), first, first_until)
        .unwrap()
        .is_none());
    let next = f
        .store
        .native_consultation_return_attempt_read_before(&original, first_until)
        .unwrap();
    assert!(next.expires_at() <= first_until);
    assert!(f
        .store
        .try_begin_native_consultation_return_before(original.clone(), next, first_until)
        .unwrap()
        .is_none());
    assert!(alias.current());
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    drop(alias);
    // No second prepare/admission/plan/effect. Only same original fresh current
    // read and typed pre-effect admission are repeated after the real reader drains.
    let next = f
        .store
        .native_consultation_return_attempt_read_before(&original, first_until)
        .unwrap();
    let (attempt, cut) = f
        .store
        .try_begin_native_consultation_return_before(original.clone(), next, first_until)
        .unwrap()
        .expect("original begin after drain");
    assert_eq!(attempt.record_id(), original.record_id());
    assert_eq!(attempt.plan().unwrap(), original_plan);
    assert!(cut.current() && cut.expires_at() <= first_until);
    let sql = f.store.connection().unwrap();
    let actual:(bool,u64,u64,String,u64)=sql.query_row(
        "SELECT n.started,e.attempt,e.submission_generation,e.state,(SELECT COUNT(*) FROM operation_effects WHERE operation_record_id=n.operation_record_id) FROM native_consultation_returns n JOIN operation_effects e ON e.id=n.effect_id WHERE n.operation_record_id=?1",
        [origin.record_id()], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
    assert_eq!(actual, (true, 1, 1, "submitting".into(), 1));
    drop(sql);
    drop(cut);
    let repeated = f
        .store
        .native_consultation_return_attempt_read_before(&original, first_until)
        .unwrap();
    let result =
        f.store
            .try_begin_native_consultation_return_before(original, repeated, first_until);
    assert_eq!(
        result.err().expect("cannot begin same effect twice").code,
        "reconciliation-required"
    );
}
#[test]
fn native_return_prebegin_primary_sql_busy_releases_writer_without_losing_original() {
    let (f, command, key, _parent, sources) = fixture_full();
    let (lease, origin) = prepared(&f, command, &key, &sources);
    let original = Arc::new(lease);
    let selected = f
        .store
        .native_consultation_return_attempt_read(&original)
        .unwrap();
    let until = selected.expires_at();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let sql = f.store.connection().unwrap();
    assert!(f
        .store
        .try_begin_native_consultation_return_before(original.clone(), selected, until)
        .unwrap()
        .is_none());
    let fresh = f
        .store
        .try_acquire_authority(&[crate::AuthorityScope::conversation(
            &f.cid,
            crate::AuthorityMode::Read,
        )
        .unwrap()])
        .expect("immediate SQL refusal parked a mutation Writer");
    drop(fresh);
    drop(sql);
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    let current = f
        .store
        .native_consultation_return_attempt_read_before(&original, until)
        .unwrap();
    let (attempt, cut) = f
        .store
        .try_begin_native_consultation_return_before(original.clone(), current, until)
        .unwrap()
        .unwrap();
    assert_eq!(attempt.record_id(), origin.record_id());
    assert_eq!(attempt.plan().unwrap(), original.plan().unwrap());
    assert!(cut.expires_at() <= until);
    drop(cut);
}
#[test]
fn native_return_prebegin_expiry_withdrawal_and_postmutation_error_never_become_deferral() {
    for withdraw in [false, true] {
        let (mut f, command, key, _parent, sources) = fixture_full();
        let (lease, _origin) = prepared(&f, command, &key, &sources);
        let original = Arc::new(lease);
        if withdraw {
            f.store
                .revoke(&f.store.revalidate(&f.admin).unwrap(), &key)
                .unwrap();
            assert!(f
                .store
                .native_consultation_return_attempt_read(&original)
                .is_err());
            continue;
        }
        let read = f
            .store
            .native_consultation_return_attempt_read(&original)
            .unwrap();
        let before = Store::inspect_snapshot(f.store.path()).unwrap();
        let expired = f.store.try_begin_native_consultation_return_before(
            original.clone(),
            read,
            crate::now(),
        );
        assert_eq!(
            expired.err().expect("expiry must be terminal").code,
            "forbidden"
        );
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
        let read = f
            .store
            .native_consultation_return_attempt_read(&original)
            .unwrap();
        let until = read.expires_at();
        // touch() checks journal capacity AFTER started/effect mutation. Even
        // though the transaction rolls back, this is Err, never typed prebegin None.
        let capacity = f.store.limits.journal_events;
        f.store.limits.journal_events = 0;
        let refused = f
            .store
            .try_begin_native_consultation_return_before(original, read, until);
        f.store.limits.journal_events = capacity;
        assert_eq!(
            refused
                .err()
                .expect("post-DML failure must be terminal")
                .code,
            "capacity-exceeded"
        );
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    }
}

#[test]
fn return_control_read_releases_sql_for_file_wait_then_rechecks_current_original() {
    use std::{sync::mpsc, time::Duration};
    for withdraw in [false, true] {
        let (f, command, key, _, sources) = fixture_full();
        let (lease, _) = prepared(&f, command, &key, &sources);
        let owner = f.store.revalidate(&key).unwrap().identity_id;
        let writer = f
            .store
            .acquire_authority(
                &[crate::AuthorityScope::identity(&owner, crate::AuthorityMode::Write).unwrap()],
                &crate::AuthorityWait::new(Duration::from_secs(2)),
            )
            .unwrap();
        let cap = lease.expires_at().min(crate::now() + 3);
        let wait = crate::AuthorityWait::new(Duration::from_secs(2));
        let (at_file, file_wait) = mpsc::channel();
        let (done, result) = mpsc::channel();
        let (observed, early, joined) = std::thread::scope(|scope| {
            let store = &f.store;
            let lease = &lease;
            let worker = scope.spawn(move || {
                let mut until = cap;
                let read = crate::authority_fences::with_waited_lock_probe(
                    move || {
                        let _ = at_file.send(());
                    },
                    || {
                        store.native_consultation_return_attempt_read_with_wait(
                            lease, &mut until, &wait,
                        )
                    },
                );
                let _ = done.send(());
                (read, until)
            });
            let observed = file_wait.recv_timeout(Duration::from_secs(1));
            if withdraw {
                let sql = f.store.connection_with_authority(&writer).unwrap();
                sql.execute("UPDATE credentials SET revoked=1 WHERE id=?1", [&key])
                    .unwrap();
                drop(sql);
                drop(writer);
                (observed, None, worker.join().unwrap())
            } else {
                // First SQL is gone while file admission waits. After file
                // release, final SQL must wait rather than discard this Read.
                let sql = f.store.connection().unwrap();
                drop(writer);
                let early = result.recv_timeout(Duration::from_millis(30));
                drop(sql);
                (observed, Some(early), worker.join().unwrap())
            }
        });
        assert!(observed.is_ok());
        assert!(joined.1 <= cap);
        if withdraw {
            assert_eq!(joined.0.err().unwrap().code, "unauthorized");
        } else {
            assert!(early.unwrap().is_err());
            let read = joined.0.unwrap();
            assert!(read.current() && read.expires_at() <= cap);
            drop(read);
        }
        let sql = f.store.connection().unwrap();
        let counts: (bool, u64) = sql.query_row(
            "SELECT started,(SELECT attempt FROM operation_effects e WHERE e.id=r.effect_id) FROM native_consultation_returns r WHERE operation_record_id=?1",
            [lease.record_id()], |r| Ok((r.get(0)?, r.get(1)?)),
        ).unwrap();
        assert_eq!(counts, (false, 0));
    }
}

#[test]
fn return_control_read_cancelled_sql_wait_returns_without_reservation_or_reader() {
    use std::{sync::mpsc, time::Duration};
    let (f, command, key, _, sources) = fixture_full();
    let (lease, _) = prepared(&f, command, &key, &sources);
    let wait = crate::AuthorityWait::new(Duration::from_secs(2));
    let sql = f.store.connection().unwrap();
    let (started, entered) = mpsc::channel();
    let joined = std::thread::scope(|scope| {
        let store = &f.store;
        let lease = &lease;
        let worker_wait = wait.clone();
        let worker = scope.spawn(move || {
            let mut cap = lease.expires_at();
            let _ = started.send(());
            store.native_consultation_return_attempt_read_with_wait(lease, &mut cap, &worker_wait)
        });
        let entered = entered.recv_timeout(Duration::from_secs(1));
        wait.cancel();
        let result = worker.join().unwrap();
        (entered, result)
    });
    drop(sql);
    assert!(joined.0.is_ok());
    assert_eq!(joined.1.err().unwrap().code, "cancelled");
    let writer = f
        .store
        .try_acquire_authority(&[crate::AuthorityScope::conversation(
            &f.cid,
            crate::AuthorityMode::Write,
        )
        .unwrap()])
        .unwrap();
    drop(writer);
}

#[test]
fn return_control_begin_waits_for_sql_without_any_authority_then_keeps_one_attempt() {
    use std::{sync::mpsc, time::Duration};
    let (f, command, key, _, sources) = fixture_full();
    let (lease, origin) = prepared(&f, command, &key, &sources);
    let lease = Arc::new(lease);
    let read = f
        .store
        .native_consultation_return_attempt_read(&lease)
        .unwrap();
    let cap = read.expires_at();
    let sql = f.store.connection().unwrap();
    let (done, outcome) = mpsc::channel();
    let (early, no_authority, joined) = std::thread::scope(|scope| {
        let store = &f.store;
        let lease = lease.clone();
        let worker = scope.spawn(move || {
            let result = store.try_begin_native_consultation_return_with_wait(
                lease,
                read,
                cap,
                &crate::AuthorityWait::new(Duration::from_secs(2)),
            );
            let _ = done.send(());
            result
        });
        let early = outcome.recv_timeout(Duration::from_millis(30));
        let proof = f
            .store
            .try_acquire_authority(&[crate::AuthorityScope::conversation(
                &f.cid,
                crate::AuthorityMode::Write,
            )
            .unwrap()]);
        let no_authority = proof.is_ok();
        drop(proof);
        drop(sql);
        (early, no_authority, worker.join().unwrap())
    });
    let (attempt, control) = joined.unwrap().unwrap();
    assert!(early.is_err() && no_authority);
    assert_eq!(attempt.record_id(), origin.record_id());
    assert_eq!(attempt.plan().unwrap(), lease.plan().unwrap());
    assert!(control.current() && control.expires_at() <= cap);
    let sql = f.store.connection().unwrap();
    let count: u64 = sql.query_row("SELECT count(*) FROM native_consultation_returns WHERE operation_record_id=?1 AND started=1", [lease.record_id()], |r|r.get(0)).unwrap();
    assert_eq!(count, 1);
    drop(sql);
    drop(control);
    f.store
        .native_consultation_return_uncertain(&attempt)
        .unwrap();
}

#[test]
fn return_control_begin_never_waits_for_writer_while_holding_sql() {
    use std::{sync::mpsc, time::Duration};
    let (f, command, key, _, sources) = fixture_full();
    let (lease, _) = prepared(&f, command, &key, &sources);
    let lease = Arc::new(lease);
    let alias = f
        .store
        .native_consultation_return_attempt_read(&lease)
        .unwrap();
    let selected = f
        .store
        .native_consultation_return_attempt_read(&lease)
        .unwrap();
    let cap = selected.expires_at();
    let (sent, received) = mpsc::channel();
    let (early, sql_released, joined) = std::thread::scope(|scope| {
        let store = &f.store;
        let original = lease.clone();
        let worker = scope.spawn(move || {
            let result = store.try_begin_native_consultation_return_with_wait(
                original,
                selected,
                cap,
                &crate::AuthorityWait::new(Duration::from_secs(2)),
            );
            let _ = sent.send(());
            result
        });
        let early = received.recv_timeout(Duration::from_millis(250));
        let sql = f.store.try_connection();
        let sql_released = sql.is_ok();
        drop(sql);
        drop(alias);
        (early, sql_released, worker.join().unwrap())
    });
    assert!(early.is_ok() && sql_released);
    assert!(joined.unwrap().is_none());
    let sql = f.store.connection().unwrap();
    let state: (bool,u64) = sql.query_row("SELECT started,(SELECT attempt FROM operation_effects e WHERE e.id=r.effect_id) FROM native_consultation_returns r WHERE operation_record_id=?1",[lease.record_id()],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(state, (false, 0));
}

mod initial_control;
