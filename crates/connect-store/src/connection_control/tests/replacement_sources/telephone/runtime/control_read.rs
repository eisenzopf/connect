//! Actual original telephone allocation Read control and SQL drain. No SIP IO.
use super::*;
use std::{sync::mpsc, time::Duration};

#[test]
fn allocation_control_read_waits_for_actual_sql_after_file_admission_and_preserves_cutoff() {
    let (f, origin, sources, _, key) = setup_runtime(true);
    let p = prepared(&f, &origin, &sources);
    let until = crate::now() + 3;
    let owner = f.store.revalidate(&key).unwrap().identity_id;
    let writer = f
        .store
        .acquire_authority(
            &[crate::AuthorityScope::identity(&owner, crate::AuthorityMode::Write).unwrap()],
            &crate::AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    let before = (
        f.count("operation_effects"),
        f.count("native_replacement_steps"),
    );
    let wait = crate::AuthorityWait::new(Duration::from_secs(2));
    let (entered, waiting) = mpsc::channel();
    let (done, completion) = mpsc::channel();
    let (observed, early, result) = std::thread::scope(|scope| {
        let store = &f.store;
        let p = &p;
        let key = &key;
        let worker = scope.spawn(move || {
            let mut cap = until;
            let read = crate::authority_fences::with_waited_lock_probe(
                move || {
                    let _ = entered.send(());
                },
                || {
                    store.telephone_replacement_allocation_read_with_wait(
                        p,
                        key,
                        &policy(),
                        &mut cap,
                        &wait,
                    )
                },
            );
            let _ = done.send(());
            (read, cap)
        });
        let observed = waiting.recv_timeout(Duration::from_secs(1));
        // First SQL resolved and dropped; the worker is blocked on the real
        // owner fence. The next SQL owner must be allowed to finish in place.
        let sql = f.store.connection().unwrap();
        drop(writer);
        let early = completion.recv_timeout(Duration::from_millis(30));
        drop(sql);
        (observed, early, worker.join())
    });
    let (read, cap) = result.unwrap();
    assert!(observed.is_ok());
    assert!(
        early.is_err(),
        "post-file control resolution must wait for actual SQL custody"
    );
    let read = read.unwrap();
    assert!(read.current() && read.expires_at() <= cap && cap <= until);
    assert_eq!(read.route_actor().credential_id, key);
    drop(read);
    assert_eq!(
        (
            f.count("operation_effects"),
            f.count("native_replacement_steps")
        ),
        before
    );
}

#[test]
fn allocation_control_read_rechecks_original_route_credential_after_actual_writer() {
    let (f, origin, sources, _, key) = setup_runtime(true);
    let p = prepared(&f, &origin, &sources);
    let owner = f.store.revalidate(&key).unwrap().identity_id;
    let writer = f
        .store
        .acquire_authority(
            &[crate::AuthorityScope::identity(&owner, crate::AuthorityMode::Write).unwrap()],
            &crate::AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    let before = f.count("operation_effects");
    let until = crate::now() + 3;
    let wait = crate::AuthorityWait::new(Duration::from_secs(2));
    let (entered, waiting) = mpsc::channel();
    let (observed, result) = std::thread::scope(|scope| {
        let store = &f.store;
        let p = &p;
        let key = &key;
        let worker = scope.spawn(move || {
            let mut cap = until;
            let read = crate::authority_fences::with_waited_lock_probe(
                move || {
                    let _ = entered.send(());
                },
                || {
                    store.telephone_replacement_allocation_read_with_wait(
                        p,
                        key,
                        &policy(),
                        &mut cap,
                        &wait,
                    )
                },
            );
            (read, cap)
        });
        let observed = waiting.recv_timeout(Duration::from_secs(1));
        {
            let sql = f.store.connection_with_authority(&writer).unwrap();
            assert_eq!(
                sql.execute("UPDATE credentials SET revoked=1 WHERE id=?1", [&key])
                    .unwrap(),
                1
            );
        }
        drop(writer);
        (observed, worker.join())
    });
    let (read, cap) = result.unwrap();
    assert!(observed.is_ok());
    assert_eq!(
        read.err()
            .expect("revoked original carrier cannot admit allocation")
            .code,
        "unauthorized"
    );
    assert!(cap <= until);
    assert_eq!(f.count("operation_effects"), before);
}

#[test]
fn allocation_control_read_cancel_and_original_expiry_never_begin_effects() {
    let (f, origin, sources, _, key) = setup_runtime(true);
    let p = prepared(&f, &origin, &sources);
    let before = (
        f.count("operation_effects"),
        f.count("native_replacement_steps"),
    );
    let wait = crate::AuthorityWait::new(Duration::from_secs(2));
    let sql = f.store.connection().unwrap();
    let (entered, started) = mpsc::channel();
    let (done, completion) = mpsc::channel();
    let (observed, early, result) = std::thread::scope(|scope| {
        let store = &f.store;
        let p = &p;
        let key = &key;
        let child_wait = wait.clone();
        let worker = scope.spawn(move || {
            let mut cap = crate::now() + 3;
            let _ = entered.send(());
            let read = store.telephone_replacement_allocation_read_with_wait(
                p,
                key,
                &policy(),
                &mut cap,
                &child_wait,
            );
            let _ = done.send(());
            read
        });
        let observed = started.recv_timeout(Duration::from_secs(1));
        let early = completion.recv_timeout(Duration::from_millis(30));
        wait.cancel();
        let result = worker.join();
        drop(sql);
        (observed, early, result)
    });
    assert!(observed.is_ok());
    assert!(early.is_err());
    assert_eq!(
        result
            .unwrap()
            .err()
            .expect("canceled original wait refuses")
            .code,
        "cancelled"
    );
    let wait = crate::AuthorityWait::new(Duration::from_secs(1));
    let mut expired = crate::now();
    assert!(f
        .store
        .telephone_replacement_allocation_read_with_wait(&p, &key, &policy(), &mut expired, &wait,)
        .is_err());
    assert_eq!(
        (
            f.count("operation_effects"),
            f.count("native_replacement_steps")
        ),
        before
    );
}

#[test]
fn allocation_control_first_fact_expiry_survives_failed_file_admission_without_renewal() {
    let (f, origin, sources, _, key) = setup_runtime(true);
    let p = prepared(&f, &origin, &sources);
    let owner = f.store.revalidate(&key).unwrap().identity_id;
    let writer = f
        .store
        .acquire_authority(
            &[crate::AuthorityScope::identity(&owner, crate::AuthorityMode::Write).unwrap()],
            &crate::AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    let before = (
        f.count("operation_effects"),
        f.count("native_replacement_steps"),
    );
    let observed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let probe_observed = observed.clone();
    let mut cap = i64::MAX;
    let result = crate::authority_fences::with_waited_lock_probe(
        move || probe_observed.store(true, std::sync::atomic::Ordering::Release),
        || {
            f.store.telephone_replacement_allocation_read_with_wait(
                &p,
                &key,
                &policy(),
                &mut cap,
                &crate::AuthorityWait::new(Duration::from_millis(60)),
            )
        },
    );
    let upper_after = crate::now() + 5;
    drop(writer);
    assert!(observed.load(std::sync::atomic::Ordering::Acquire));
    assert_eq!(
        result
            .err()
            .expect("actual file Writer blocks original Read")
            .code,
        "authority-busy"
    );
    assert!(cap < i64::MAX && cap <= upper_after);
    let first = cap;
    let read = f
        .store
        .telephone_replacement_allocation_read_with_wait(
            &p,
            &key,
            &policy(),
            &mut cap,
            &crate::AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    assert!(read.current() && cap <= first && read.expires_at() <= first);
    drop(read);
    assert_eq!(
        (
            f.count("operation_effects"),
            f.count("native_replacement_steps")
        ),
        before
    );
}
