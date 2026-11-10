//! Current initial planning/reservation under real SQL/file ownership; no Core IO.
use super::*;
use crate::NativeConsultationReturnPreparation as Preparation;
use std::{sync::mpsc, time::Duration};
fn initial(
    f: &Fixture,
    command: TransitionCommand,
    key: &str,
    sources: &[Arc<crate::NativeMediaOrigin>],
) -> (crate::NativeConsultationReturnOrigin, Value) {
    f.store
        .admit_transition_command(key, command.clone())
        .unwrap();
    let origin = f
        .store
        .native_consultation_return_origin(key, command)
        .unwrap();
    let plan = return_plan(f, &origin, sources);
    (origin, plan)
}
#[test]
fn initial_return_control_waits_for_final_sql_and_rechecks_actual_credential_with_first_cap() {
    for withdraw in [false, true] {
        let (f, command, key, _, sources) = fixture_full();
        let (origin, _) = initial(&f, command, &key, &sources);
        let identity = f.store.revalidate(&key).unwrap().identity_id;
        let writer = f
            .store
            .acquire_authority(
                &[
                    crate::AuthorityScope::identity(&identity, crate::AuthorityMode::Write)
                        .unwrap(),
                ],
                &crate::AuthorityWait::new(Duration::from_secs(2)),
            )
            .unwrap();
        let until = crate::now() + 3;
        let (entered, entering) = mpsc::channel();
        let (done, completed) = mpsc::channel();
        let (observed, early, joined) = std::thread::scope(|scope| {
            let store = &f.store;
            let origin = &origin;
            let sources = &sources;
            let worker = scope.spawn(move || {
                let mut cap = until;
                let read = crate::authority_fences::with_waited_lock_probe(
                    move || {
                        let _ = entered.send(());
                    },
                    || {
                        store.native_return_planning_read_with_wait(
                            origin,
                            sources,
                            &[],
                            &mut cap,
                            &crate::AuthorityWait::new(Duration::from_secs(2)),
                        )
                    },
                );
                let _ = done.send(());
                (read, cap)
            });
            let observed = entering.recv_timeout(Duration::from_secs(1));
            if withdraw {
                let sql = f.store.connection_with_authority(&writer).unwrap();
                assert_eq!(
                    sql.execute("UPDATE credentials SET revoked=1 WHERE id=?1", [&key])
                        .unwrap(),
                    1
                );
                drop(sql);
                drop(writer);
                (observed, None, worker.join().unwrap())
            } else {
                let sql = f.store.connection().unwrap();
                drop(writer);
                let early = completed.recv_timeout(Duration::from_millis(30));
                drop(sql);
                (observed, Some(early), worker.join().unwrap())
            }
        });
        assert!(observed.is_ok() && joined.1 <= until);
        if withdraw {
            assert_eq!(joined.0.err().unwrap().code, "unauthorized");
        } else {
            assert!(early.unwrap().is_err());
            let read = joined.0.unwrap();
            assert!(read.current() && read.expires_at() <= until);
            drop(read);
        }
        let sql = f.store.connection().unwrap();
        let state:(String,u64)=sql.query_row("SELECT state,(SELECT count(*) FROM operation_effects WHERE operation_record_id=o.record_id) FROM operations o WHERE record_id=?1",[origin.record_id()],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(state, ("accepted".into(), 0));
    }
}
#[test]
fn initial_return_planning_cancelled_sql_wait_joins_without_read_or_reservation() {
    let (f, command, key, _, sources) = fixture_full();
    let (origin, _) = initial(&f, command, &key, &sources);
    let sql = f.store.connection().unwrap();
    let wait = crate::AuthorityWait::new(Duration::from_secs(2));
    let (sent, received) = mpsc::channel();
    let result = std::thread::scope(|scope| {
        let store = &f.store;
        let origin = &origin;
        let sources = &sources;
        let worker_wait = wait.clone();
        let worker = scope.spawn(move || {
            let mut cap = crate::now() + 3;
            let _ = sent.send(());
            store.native_return_planning_read_with_wait(
                origin,
                sources,
                &[],
                &mut cap,
                &worker_wait,
            )
        });
        let entered = received.recv_timeout(Duration::from_secs(1));
        wait.cancel();
        (entered, worker.join().unwrap())
    });
    drop(sql);
    assert!(result.0.is_ok());
    assert_eq!(result.1.err().unwrap().code, "cancelled");
    drop(
        f.store
            .try_acquire_authority(&[crate::AuthorityScope::conversation(
                &f.cid,
                crate::AuthorityMode::Write,
            )
            .unwrap()])
            .unwrap(),
    );
    let sql = f.store.connection().unwrap();
    assert_eq!(
        sql.query_row(
            "SELECT count(*) FROM operation_effects WHERE operation_record_id=?1",
            [origin.record_id()],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
}
#[test]
fn initial_return_prepare_waits_sql_without_authority_and_reserves_same_plan_once() {
    let (f, command, key, _, sources) = fixture_full();
    let (origin, plan) = initial(&f, command, &key, &sources);
    let read = f
        .store
        .try_native_return_read(&origin, &sources, &[])
        .unwrap();
    let until = read.expires_at();
    let sql = f.store.connection().unwrap();
    let (done, completed) = mpsc::channel();
    let (early, no_authority, result) = std::thread::scope(|scope| {
        let store = &f.store;
        let plan = plan.clone();
        let worker = scope.spawn(move || {
            let result = store.prepare_native_consultation_return_with_wait(
                read,
                plan,
                until,
                &crate::AuthorityWait::new(Duration::from_secs(2)),
            );
            let _ = done.send(());
            result
        });
        let early = completed.recv_timeout(Duration::from_millis(30));
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
    assert!(early.is_err() && no_authority);
    let Preparation::Reserved(lease) = result.unwrap() else {
        panic!("original reservation did not complete")
    };
    assert_eq!(lease.record_id(), origin.record_id());
    assert_eq!(lease.plan().unwrap(), plan);
    let read = f
        .store
        .try_native_return_read(&origin, &sources, &[])
        .unwrap();
    let Preparation::Reserved(replay) = f
        .store
        .prepare_native_consultation_return_with_wait(
            read,
            plan,
            until,
            &crate::AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap()
    else {
        panic!("same original reservation replay")
    };
    assert_eq!(replay.plan().unwrap(), lease.plan().unwrap());
    let sql = f.store.connection().unwrap();
    let state:(bool,u64,u64)=sql.query_row("SELECT started,(SELECT attempt FROM operation_effects e WHERE e.id=r.effect_id),(SELECT count(*) FROM operation_effects WHERE operation_record_id=r.operation_record_id) FROM native_consultation_returns r WHERE operation_record_id=?1",[origin.record_id()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(state, (false, 0, 1));
}
#[test]
fn initial_return_prepare_reader_refusal_is_typed_pre_dml_and_does_not_occupy_sql() {
    let (f, command, key, _, sources) = fixture_full();
    let (origin, plan) = initial(&f, command, &key, &sources);
    let alias = f
        .store
        .try_native_return_read(&origin, &sources, &[])
        .unwrap();
    let read = f
        .store
        .try_native_return_read(&origin, &sources, &[])
        .unwrap();
    let until = read.expires_at();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let outcome = f
        .store
        .prepare_native_consultation_return_with_wait(
            read,
            plan.clone(),
            until,
            &crate::AuthorityWait::new(Duration::from_secs(2)),
        )
        .unwrap();
    assert!(matches!(outcome,Preparation::Deferred(ref error) if error.code=="authority-busy"));
    drop(
        f.store
            .try_connection()
            .expect("immediate Writer refusal must release primary SQL"),
    );
    assert!(alias.current());
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    drop(alias);
    let read = f
        .store
        .try_native_return_read(&origin, &sources, &[])
        .unwrap();
    assert!(matches!(
        f.store
            .prepare_native_consultation_return_with_wait(
                read,
                plan,
                until,
                &crate::AuthorityWait::new(Duration::from_secs(2))
            )
            .unwrap(),
        Preparation::Reserved(_)
    ));
}
#[test]
fn initial_return_prepare_cancel_expiry_and_post_dml_capacity_are_never_deferred() {
    let (mut f, command, key, _, sources) = fixture_full();
    let (origin, plan) = initial(&f, command, &key, &sources);
    let read = f
        .store
        .try_native_return_read(&origin, &sources, &[])
        .unwrap();
    let until = read.expires_at();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let sql = f.store.connection().unwrap();
    let wait = crate::AuthorityWait::new(Duration::from_secs(2));
    let (sent, received) = mpsc::channel();
    let result = std::thread::scope(|scope| {
        let store = &f.store;
        let plan = plan.clone();
        let worker_wait = wait.clone();
        let worker = scope.spawn(move || {
            let _ = sent.send(());
            store.prepare_native_consultation_return_with_wait(read, plan, until, &worker_wait)
        });
        let entered = received.recv_timeout(Duration::from_secs(1));
        wait.cancel();
        (entered, worker.join().unwrap())
    });
    drop(sql);
    assert!(result.0.is_ok());
    assert_eq!(
        result.1.err().expect("cancellation is terminal").code,
        "cancelled"
    );
    let read = f
        .store
        .try_native_return_read(&origin, &sources, &[])
        .unwrap();
    assert_eq!(
        f.store
            .prepare_native_consultation_return_with_wait(
                read,
                plan.clone(),
                crate::now(),
                &crate::AuthorityWait::new(Duration::from_secs(1))
            )
            .err()
            .expect("original expiry is terminal")
            .code,
        "forbidden"
    );
    let read = f
        .store
        .try_native_return_read(&origin, &sources, &[])
        .unwrap();
    let until = read.expires_at();
    let capacity = f.store.limits.journal_events;
    f.store.limits.journal_events = 0;
    let result = f.store.prepare_native_consultation_return_with_wait(
        read,
        plan,
        until,
        &crate::AuthorityWait::new(Duration::from_secs(1)),
    );
    f.store.limits.journal_events = capacity;
    assert_eq!(
        result
            .err()
            .expect("post-DML storage/capacity must not be a planning retry")
            .code,
        "capacity-exceeded"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
}
#[test]
fn captured_return_record_and_completion_do_not_park_writer_on_primary_sql() {
    let (f, command, key, _, sources) = fixture_full();
    let (lease, origin) = prepared(&f, command, &key, &sources);
    let plan = lease.plan().unwrap();
    let read = f
        .store
        .native_consultation_return_attempt_read(&lease)
        .unwrap();
    let (attempt, read) = f
        .store
        .begin_native_consultation_return(lease, read)
        .unwrap();
    drop(read);
    let receipt = Arc::new(
        attempt
            .applied(
                observation(&plan),
                crate::native_stream_projection::unchanged_audience_metadata_for_test(
                    &observation(&plan),
                ),
            )
            .unwrap(),
    );
    let sql = f.store.connection().unwrap();
    let (recorded, no_writer) = std::thread::scope(|scope| {
        let store = &f.store;
        let receipt = &receipt;
        let worker = scope.spawn(move || store.record_native_consultation_return_applied(receipt));
        let proof = f
            .store
            .try_acquire_authority(&[crate::AuthorityScope::conversation(
                &f.cid,
                crate::AuthorityMode::Write,
            )
            .unwrap()]);
        let no_writer = proof.is_ok();
        drop(proof);
        (worker.join().unwrap(), no_writer)
    });
    assert!(no_writer);
    assert_eq!(
        recorded
            .err()
            .expect("bounded metadata SQL wait must refuse")
            .code,
        "authority-busy"
    );
    drop(sql);
    let original = f
        .store
        .record_native_consultation_return_applied(&receipt)
        .unwrap();
    assert_eq!(
        f.store
            .record_native_consultation_return_applied(&receipt)
            .unwrap(),
        original
    );
    let sql = f.store.connection().unwrap();
    let (done, completed) = mpsc::channel();
    let (early, no_writer, result) = std::thread::scope(|scope| {
        let store = &f.store;
        let receipt = &receipt;
        let worker = scope.spawn(move || {
            let result = store.complete_native_consultation_return(receipt);
            let _ = done.send(());
            result
        });
        let early = completed.recv_timeout(Duration::from_millis(30));
        let proof = f
            .store
            .try_acquire_authority(&[crate::AuthorityScope::conversation(
                &f.cid,
                crate::AuthorityMode::Write,
            )
            .unwrap()]);
        let no_writer = proof.is_ok();
        drop(proof);
        drop(sql);
        (early, no_writer, worker.join().unwrap())
    });
    assert!(early.is_err() && no_writer);
    assert!(
        result.unwrap().is_none(),
        "unretired candidate must not be reported complete"
    );
    let sql = f.store.connection().unwrap();
    let state:(bool,u64)=sql.query_row("SELECT started,(SELECT attempt FROM operation_effects e WHERE e.id=r.effect_id) FROM native_consultation_returns r WHERE operation_record_id=?1",[origin.record_id()],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(state, (true, 1));
}
