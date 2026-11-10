//! Actual SQL/file contention and original submission custody; no SIP IO claim.
use super::*;
use std::{sync::mpsc, time::Duration};

#[test]
fn submission_control_waits_for_sql_and_same_attempt_write_remains_current_and_once_only() {
    let (f, origin, sources, _, key) = setup_runtime(true);
    let prepared = prepared(&f, &origin, &sources);
    let receipt = allocated(&f, &prepared, &key);
    let before = f.count("operation_effects");
    let wait = crate::AuthorityWait::new(Duration::from_secs(2));
    let sql = f.store.connection().unwrap();
    let (started, entered) = mpsc::channel();
    let (done, completed) = mpsc::channel();
    let (early, result) = std::thread::scope(|scope| {
        let store = &f.store;
        let receipt = receipt.clone();
        let wait = wait.clone();
        let worker = scope.spawn(move || {
            let mut until = crate::now() + 3;
            let _ = started.send(());
            let result =
                store.telephone_replacement_submission_read_with_wait(receipt, &mut until, &wait);
            let _ = done.send(());
            (result, until)
        });
        let entered = entered.recv_timeout(Duration::from_secs(1));
        let early = completed.recv_timeout(Duration::from_millis(30));
        drop(sql);
        (entered.is_ok() && early.is_err(), worker.join())
    });
    let (read, until) = result.unwrap();
    assert!(
        early,
        "old immediate read returns before the actual SQL owner drains"
    );
    let read = read.unwrap();
    assert!(read.current() && read.expires_at() <= until);
    assert_eq!(f.count("operation_effects"), before);
    let scene_scope =
        crate::AuthorityScope::conversation(&f.cid, crate::AuthorityMode::Read).unwrap();
    let other = f
        .store
        .try_acquire_authority(&[scene_scope.clone()])
        .unwrap();
    assert!(f
        .store
        .try_begin_telephone_replacement_submission_with_wait(read, &wait)
        .unwrap()
        .is_none());
    // None released SQL and its failed mutation admission; old reader is intact.
    assert!(f.store.try_connection().is_ok());
    assert_eq!(f.count("operation_effects"), before);
    drop(other);
    let mut cap = until;
    let read = f
        .store
        .telephone_replacement_submission_read_with_wait(receipt.clone(), &mut cap, &wait)
        .unwrap();
    let attempt = f
        .store
        .try_begin_telephone_replacement_submission_with_wait(read, &wait)
        .unwrap()
        .unwrap();
    assert!(
        f.store.try_acquire_authority(&[scene_scope]).is_ok(),
        "metadata attempt retains no writer"
    );
    let write = f
        .store
        .telephone_replacement_submission_begun_read_with_wait(attempt.clone(), &mut cap, &wait)
        .unwrap();
    assert!(write.current() && write.expires_at() <= until);
    assert_eq!(write.call_id(), receipt.call_id());
    drop(write);
    let read = f
        .store
        .telephone_replacement_submission_read_with_wait(receipt, &mut cap, &wait)
        .unwrap();
    assert_eq!(
        f.store
            .try_begin_telephone_replacement_submission_with_wait(read, &wait)
            .err()
            .unwrap()
            .code,
        "reconciliation-required"
    );
    assert_eq!(f.count("operation_effects"), before + 1);
    f.store
        .record_telephone_replacement_uncertain(&attempt.uncertain())
        .unwrap();
}

#[test]
fn submission_control_cancel_revocation_and_post_dml_error_are_terminal_not_deferral() {
    for revoke in [false, true] {
        let (mut f, origin, sources, _, key) = setup_runtime(true);
        let prepared = prepared(&f, &origin, &sources);
        let receipt = allocated(&f, &prepared, &key);
        let wait = crate::AuthorityWait::new(Duration::from_secs(2));
        let mut until = crate::now() + 3;
        let read = f
            .store
            .telephone_replacement_submission_read_with_wait(receipt.clone(), &mut until, &wait)
            .unwrap();
        let before = Store::inspect_snapshot(f.store.path()).unwrap();
        if revoke {
            drop(read);
            let owner = f.store.revalidate(&key).unwrap().identity_id;
            let writer = f
                .store
                .try_acquire_authority(&[crate::AuthorityScope::identity(
                    &owner,
                    crate::AuthorityMode::Write,
                )
                .unwrap()])
                .unwrap();
            {
                let c = f.store.connection_with_authority(&writer).unwrap();
                c.execute("UPDATE credentials SET revoked=1 WHERE id=?1", [&key])
                    .unwrap();
            }
            drop(writer);
            assert_eq!(
                f.store
                    .telephone_replacement_submission_read_with_wait(receipt, &mut until, &wait)
                    .err()
                    .unwrap()
                    .code,
                "unauthorized"
            );
            assert_eq!(f.count("native_replacement_steps"), 2);
        } else {
            let cancelled = crate::AuthorityWait::new(Duration::from_secs(2));
            cancelled.cancel();
            assert!(f
                .store
                .try_begin_telephone_replacement_submission_with_wait(read, &cancelled)
                .is_err());
            assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
            let read = f
                .store
                .telephone_replacement_submission_read_with_wait(receipt, &mut until, &wait)
                .unwrap();
            let capacity = f.store.limits.journal_events;
            f.store.limits.journal_events = 0;
            let result = f
                .store
                .try_begin_telephone_replacement_submission_with_wait(read, &wait);
            f.store.limits.journal_events = capacity;
            assert_eq!(
                result
                    .err()
                    .expect("touch failure after DML must not become None")
                    .code,
                "capacity-exceeded"
            );
            assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
        }
    }
}

#[test]
fn expired_initial_submission_cap_never_renews_but_current_physical_write_is_independent() {
    let (f, origin, sources, _, key) = setup_runtime(true);
    let prepared = prepared(&f, &origin, &sources);
    let receipt = allocated(&f, &prepared, &key);
    let wait = crate::AuthorityWait::new(Duration::from_secs(2));
    let mut until = crate::now() + 2;
    let read = f
        .store
        .telephone_replacement_submission_read_with_wait(receipt, &mut until, &wait)
        .unwrap();
    let attempt = f
        .store
        .try_begin_telephone_replacement_submission_with_wait(read, &wait)
        .unwrap()
        .unwrap();
    while crate::now() < until {
        std::thread::sleep(Duration::from_millis(10));
    }
    let wait = crate::AuthorityWait::new(Duration::from_secs(2));
    let mut forbidden_renewal = i64::MAX;
    assert!(f
        .store
        .telephone_replacement_submission_begun_read_with_wait(
            attempt.clone(),
            &mut forbidden_renewal,
            &wait
        )
        .is_err());
    assert!(forbidden_renewal <= until);
    let mut physical_until = i64::MAX;
    let write = f
        .store
        .telephone_replacement_submission_write_read_with_wait(
            attempt.clone(),
            &mut physical_until,
            &wait,
        )
        .unwrap();
    assert!(write.current() && write.expires_at() > until);
    drop(write);
    let control = f
        .store
        .telephone_replacement_submission_control_read_with_wait(
            attempt.clone(),
            &mut physical_until,
            &wait,
        )
        .unwrap();
    assert!(control.current());
    drop(control);
    f.store
        .record_telephone_replacement_uncertain(&attempt.uncertain())
        .unwrap();
    assert!(
        f.store
            .telephone_replacement_submission_write_read_with_wait(
                attempt,
                &mut physical_until,
                &wait
            )
            .is_err(),
        "unknown original attempt may never authorize another write"
    );
}

#[test]
fn allocation_control_begin_returns_original_attempt_only_after_actual_reader_drain() {
    let (f, origin, sources, _, key) = setup_runtime(true);
    let prepared = prepared(&f, &origin, &sources);
    let wait = crate::AuthorityWait::new(Duration::from_secs(2));
    let mut until = crate::now() + 3;
    let read = f
        .store
        .telephone_replacement_allocation_read_with_wait(
            &prepared,
            &key,
            &policy(),
            &mut until,
            &wait,
        )
        .unwrap();
    let scope = crate::AuthorityScope::conversation(&f.cid, crate::AuthorityMode::Read).unwrap();
    let other = f.store.try_acquire_authority(&[scope.clone()]).unwrap();
    let before = f.count("operation_effects");
    assert!(f
        .store
        .try_begin_telephone_replacement_allocation_with_wait(read, &wait)
        .unwrap()
        .is_none());
    assert!(f.store.try_connection().is_ok());
    assert_eq!(f.count("operation_effects"), before);
    drop(other);
    let read = f
        .store
        .telephone_replacement_allocation_read_with_wait(
            &prepared,
            &key,
            &policy(),
            &mut until,
            &wait,
        )
        .unwrap();
    let attempt = f
        .store
        .try_begin_telephone_replacement_allocation_with_wait(read, &wait)
        .unwrap()
        .unwrap();
    assert!(f.store.try_acquire_authority(&[scope]).is_ok());
    let read = f
        .store
        .telephone_replacement_allocation_begun_read_with_wait(attempt.clone(), &mut until, &wait)
        .unwrap();
    assert!(read.current() && read.expires_at() <= until);
    drop(read);
    f.store
        .record_telephone_replacement_uncertain(&attempt.uncertain())
        .unwrap();
    assert_eq!(f.count("operation_effects"), before + 1);
}
