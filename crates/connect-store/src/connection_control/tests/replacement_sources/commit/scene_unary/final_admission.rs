//! Actual Store custody/SQL tests; no Core publication or media-readiness evidence.
use super::*;
fn wait() -> crate::AuthorityWait {
    crate::AuthorityWait::new(std::time::Duration::from_millis(100))
}
fn record(m: &Movement) -> String {
    m.f.store
        .connection()
        .unwrap()
        .query_row(
            "SELECT record_id FROM operations WHERE operation_id=?1",
            [&m.command.operation_id],
            |r| r.get(0),
        )
        .unwrap()
}
fn prepare(m: &Movement) -> crate::NativeReplacementCommitPreparation {
    let read =
        m.f.store
            .try_native_replacement_scene_read(&m.origin, &m.sources)
            .unwrap();
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    let p =
        m.f.store
            .prepare_native_replacement_commit(m.origin.clone(), &read)
            .unwrap();
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
    drop(read);
    p
}
fn begin(
    m: &Movement,
    p: &crate::NativeReplacementCommitPreparation,
) -> crate::NativeReplacementCommitAttempt {
    let read =
        m.f.store
            .try_native_replacement_commit_scene_write(&m.origin, &m.sources)
            .unwrap();
    let a =
        m.f.store
            .try_begin_prepared_native_replacement_commit(p, &read)
            .unwrap()
            .unwrap();
    assert!(
        Arc::ptr_eq(a.activation(), p.activation()),
        "same inactive installation metadata, never a second candidate"
    );
    drop(read);
    a
}
#[test]
fn native_commit_grantfree_preparation_releases_actual_source_writer_and_preserves_same_attempt() {
    let m = scene();
    let stable = counts(&m);
    let p = prepare(&m);
    assert_eq!(counts(&m), stable);
    let scope = crate::AuthorityScope::connection(
        m.sources[0].connection_id(),
        crate::AuthorityMode::Write,
    )
    .unwrap();
    let writer = m.f.store.try_acquire_authority(&[scope]).unwrap();
    assert!(
        m.f.store
            .try_native_media_read(&m.sources[0], None)
            .is_err(),
        "actual held Writer must be observable"
    );
    assert!(
        m.f.store
            .try_native_media_read(&m.sources[2], None)
            .unwrap()
            .current(),
        "unrelated source stays usable"
    );
    drop(writer);
    assert!(
        m.f.store
            .try_native_media_read(&m.sources[0], None)
            .unwrap()
            .current(),
        "preparation cannot hide a source Reader/Writer"
    );
    let a = begin(&m, &p);
    assert_eq!(
        a.attempt_binding(),
        p.attempt_binding(),
        "the only persisted IDs are the original prepared IDs"
    );
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    // Resolve the fixture identity before taking its actual SQL mutex.
    let record = record(&m);
    let identity:(String,String,u64,u64)=m.f.store.connection().unwrap().query_row(
        "SELECT s.attempt_id,s.effect_id,e.attempt,e.submission_generation FROM native_replacement_steps s JOIN operation_effects e ON e.id=s.effect_id WHERE s.operation_record_id=?1 AND s.phase='commit'",[&record],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!((identity.2, identity.3), (1, 1));
    for _ in 0..2 {
        let read = m
            .f
            .store
            .try_native_replacement_commit_attempt_scene_write(&a.admission(), &m.sources, &wait())
            .unwrap();
        assert!(read.current() && read.expires_at() <= p.expires_at());
        assert!(read.permits_receive(
            &read.media().target().runtime_connection_id,
            m.sources[2].runtime_connection_id()
        ));
        assert_eq!(
            fingerprint(read.media().target()),
            fingerprint(p.activation().target())
        );
        drop(read);
        assert_eq!(
            Store::inspect_snapshot(m.f.store.path()).unwrap(),
            before,
            "re-admission cannot mutate rows/counters/lease"
        );
    }
    let read =
        m.f.store
            .try_native_replacement_commit_scene_write(&m.origin, &m.sources)
            .unwrap();
    assert!(
        m.f.store
            .try_begin_prepared_native_replacement_commit(&p, &read)
            .is_err(),
        "same prepared IDs cannot begin twice"
    );
    drop(read);
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
}
#[test]
fn native_commit_same_attempt_readmission_refuses_changed_ledger_identity_currentness_and_foreign_scene(
) {
    for change in [
        "effect-unknown",
        "effect-confirmed",
        "counter",
        "submission",
        "step-observed",
        "attempt",
        "runtime",
        "cancel",
        "expiry",
        "third-revoked",
        "third-generation",
    ] {
        let m = scene();
        let p = prepare(&m);
        let a = begin(&m, &p);
        let record = record(&m);
        {
            let c = m.f.change_fixture();
            match change {
                "effect-unknown" => {
                    c.execute("UPDATE operation_effects SET state='unknown' WHERE id=(SELECT effect_id FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='commit')",[&record]).unwrap();
                }
                "effect-confirmed" => {
                    c.execute("UPDATE operation_effects SET state='confirmed' WHERE id=(SELECT effect_id FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='commit')",[&record]).unwrap();
                }
                "counter" => {
                    let sql="UPDATE operation_effects SET attempt=2 WHERE id=(SELECT effect_id FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='commit')";
                    assert!(
                        c.execute(sql, [&record]).is_err(),
                        "actual immutable ledger guard"
                    );
                    c.execute_batch("DROP TRIGGER operation_effect_immutable")
                        .unwrap();
                    c.execute(sql, [&record]).unwrap();
                }
                "submission" => {
                    let sql="UPDATE operation_effects SET submission_generation=2 WHERE id=(SELECT effect_id FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='commit')";
                    assert!(
                        c.execute(sql, [&record]).is_err(),
                        "actual immutable ledger guard"
                    );
                    c.execute_batch("DROP TRIGGER operation_effect_immutable")
                        .unwrap();
                    c.execute(sql, [&record]).unwrap();
                }
                "step-observed" => {
                    c.execute("UPDATE native_replacement_steps SET observation_json='{}' WHERE operation_record_id=?1 AND phase='commit'",[&record]).unwrap();
                }
                "attempt" => {
                    let sql="UPDATE native_replacement_steps SET attempt_id='foreign-attempt' WHERE operation_record_id=?1 AND phase='commit'";
                    assert!(
                        c.execute(sql, [&record]).is_err(),
                        "actual immutable ledger guard"
                    );
                    c.execute_batch("DROP TRIGGER native_replacement_step_immutable")
                        .unwrap();
                    c.execute(sql, [&record]).unwrap();
                }
                "runtime" => {
                    let sql="UPDATE native_replacements SET runtime_epoch='foreign-runtime' WHERE operation_record_id=?1";
                    assert!(
                        c.execute(sql, [&record]).is_err(),
                        "actual immutable ledger guard"
                    );
                    c.execute_batch("DROP TRIGGER native_replacement_immutable")
                        .unwrap();
                    c.execute(sql, [&record]).unwrap();
                }
                "cancel" => {
                    c.execute(
                        "UPDATE operations SET cancel_requested=1 WHERE record_id=?1",
                        [&record],
                    )
                    .unwrap();
                }
                "expiry" => {
                    let sql = "UPDATE operations SET expires_at=?2 WHERE record_id=?1";
                    let expired = crate::now() - 1;
                    assert!(
                        c.execute(sql, params![&record, expired]).is_err(),
                        "actual immutable deadline guard"
                    );
                    c.execute_batch("DROP TRIGGER operation_deadline_immutable")
                        .unwrap();
                    c.execute(sql, params![&record, expired]).unwrap();
                }
                "third-revoked" => {
                    c.execute(
                        "UPDATE credentials SET revoked=1 WHERE id=?1",
                        [&m.f.keys[2]],
                    )
                    .unwrap();
                }
                "third-generation" => {
                    c.execute(
                        "UPDATE connections SET generation=generation+1 WHERE id=?1",
                        [m.sources[2].connection_id()],
                    )
                    .unwrap();
                }
                _ => unreachable!(),
            }
        }
        let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
        assert!(
            m.f.store
                .try_native_replacement_commit_attempt_scene_write(
                    &a.admission(),
                    &m.sources,
                    &wait()
                )
                .is_err(),
            "{change}"
        );
        assert_eq!(
            Store::inspect_snapshot(m.f.store.path()).unwrap(),
            before,
            "{change}"
        );
    }
    let m = scene();
    let other = scene();
    let p = prepare(&m);
    let a = begin(&m, &p);
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    assert!(m
        .f
        .store
        .try_native_replacement_commit_attempt_scene_write(&a.admission(), &other.sources, &wait())
        .is_err());
    let other_preparation = prepare(&other);
    let other_attempt = begin(&other, &other_preparation);
    assert_ne!(a.attempt_binding().1, other_attempt.attempt_binding().1);
    assert!(
        m.f.store
            .try_native_replacement_commit_attempt_scene_write(
                &other_attempt.admission(),
                &m.sources,
                &wait()
            )
            .is_err(),
        "foreign owned handle cannot borrow this Store/scene"
    );
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
}
#[test]
fn native_commit_prepared_facts_do_not_survive_actual_permission_withdrawal_or_scene_change() {
    for change in ["cancel", "third-generation", "third-revoked"] {
        let m = scene();
        let p = prepare(&m);
        // change_fixture owns the SAME connection mutex used by record().
        let record = record(&m);
        {
            let c = m.f.change_fixture();
            match change {
                "cancel" => {
                    c.execute(
                        "UPDATE operations SET cancel_requested=1 WHERE record_id=?1",
                        [&record],
                    )
                    .unwrap();
                }
                "third-generation" => {
                    c.execute(
                        "UPDATE connections SET generation=generation+1 WHERE id=?1",
                        [m.sources[2].connection_id()],
                    )
                    .unwrap();
                }
                "third-revoked" => {
                    c.execute(
                        "UPDATE credentials SET revoked=1 WHERE id=?1",
                        [&m.f.keys[2]],
                    )
                    .unwrap();
                }
                _ => unreachable!(),
            }
        }
        let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
        let result =
            m.f.store
                .try_native_replacement_commit_scene_write(&m.origin, &m.sources)
                .and_then(|r| {
                    m.f.store
                        .try_begin_prepared_native_replacement_commit(&p, &r)
                });
        assert!(result.is_err(), "{change}");
        assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
    }
}

#[test]
fn native_prepared_final_begin_original_wait_refuses_before_dml_and_rolls_back_before_commit() {
    for stage in [
        "pre-cancelled",
        "original-expired",
        "prepared-begin-before-dml",
        "prepared-begin-before-commit",
    ] {
        let m = scene();
        let planning =
            m.f.store
                .try_native_replacement_scene_read(&m.origin, &m.sources)
                .unwrap();
        let p = m
            .f
            .store
            .prepare_native_replacement_commit_until(m.origin.clone(), &planning, crate::now() + 5)
            .unwrap();
        assert!(p.expires_at() <= planning.expires_at());
        drop(planning);
        let wait = crate::AuthorityWait::new(std::time::Duration::from_secs(1));
        let read =
            m.f.store
                .try_native_replacement_commit_scene_write_with_wait(
                    &m.origin,
                    &m.sources,
                    p.expires_at(),
                    &wait,
                )
                .unwrap();
        let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
        if stage == "pre-cancelled" {
            wait.cancel();
        }
        let actual = if stage == "original-expired" {
            let expired = crate::AuthorityWait::new(std::time::Duration::ZERO);
            m.f.store
                .try_begin_prepared_native_replacement_commit_with_wait(&p, &read, &expired)
        } else if stage == "pre-cancelled" {
            m.f.store
                .try_begin_prepared_native_replacement_commit_with_wait(&p, &read, &wait)
        } else {
            let cancelled = wait.clone();
            let hit = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let observed = hit.clone();
            let actual = Store::with_native_replacement_commit_entry_probe_for_test(
                stage,
                move || {
                    observed.store(true, std::sync::atomic::Ordering::Release);
                    cancelled.cancel();
                },
                || {
                    m.f.store
                        .try_begin_prepared_native_replacement_commit_with_wait(&p, &read, &wait)
                },
            );
            assert!(
                hit.load(std::sync::atomic::Ordering::Acquire),
                "actual original transaction boundary not reached"
            );
            actual
        };
        assert!(
            actual.is_err(),
            "original cutoff/cancel must not persist Started: {stage}"
        );
        drop(read);
        assert_eq!(
            Store::inspect_snapshot(m.f.store.path()).unwrap(),
            before,
            "actual transaction rollback/no new step/effect/journal: {stage}"
        );
    }
    // Same existing selector: a real raw connection holder produces typed None
    // before BEGIN; normal return releases the holder and panic clears TLS.
    {
        use crate::native_prepared_begin_observation_for_test::NativePreparedBeginObservationForTest;
        let m = scene();
        let p = prepare(&m);
        let wait = crate::AuthorityWait::new(std::time::Duration::from_secs(1));
        let read =
            m.f.store
                .try_native_replacement_commit_scene_write_with_wait(
                    &m.origin,
                    &m.sources,
                    p.expires_at(),
                    &wait,
                )
                .unwrap();
        let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
        let observation = Arc::new(NativePreparedBeginObservationForTest::new(
            p.attempt_binding(),
        ));
        observation.exclude_next_entry();
        let result =
            m.f.store
                .with_native_prepared_begin_observation_for_test(observation.clone(), &wait, || {
                    m.f.store
                        .try_begin_prepared_native_replacement_commit_with_wait(&p, &read, &wait)
                })
                .unwrap();
        assert!(result.is_none());
        let observed = observation.snapshot();
        assert_eq!(
            (
                observed.calls,
                observed.pre_entry_busy,
                observed.none,
                observed.sql_begins,
                observed.exclusion_holds
            ),
            (1, 1, 1, 0, 1)
        );
        assert_eq!(
            (
                observed.some,
                observed.errors,
                observed.binding_mismatches,
                observed.scope_refusals
            ),
            (0, 0, 0, 0)
        );
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _: crate::Result<Option<()>> =
                m.f.store.with_native_prepared_begin_observation_for_test(
                    observation.clone(),
                    &wait,
                    || panic!("scoped observation cleanup"),
                );
        }));
        assert!(panic.is_err());
        // Successful snapshot proves the earlier real holder was released;
        // this callback-only panic held no connection and cannot poison Store.
        assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
        let after_panic = observation.snapshot();
        let fresh = Arc::new(NativePreparedBeginObservationForTest::new(
            p.attempt_binding(),
        ));
        let expired = crate::AuthorityWait::new(std::time::Duration::ZERO);
        let result = m.f.store.with_native_prepared_begin_observation_for_test(
            fresh.clone(),
            &expired,
            || {
                m.f.store
                    .try_begin_prepared_native_replacement_commit_with_wait(&p, &read, &expired)
            },
        );
        assert!(result.is_err());
        assert_eq!(fresh.snapshot().calls, 1);
        assert_eq!(fresh.snapshot().errors, 1);
        assert_eq!(fresh.snapshot().scope_refusals, 0);
        assert_eq!(
            observation.snapshot(),
            after_panic,
            "old TLS observation cannot leak into a later call"
        );
        drop(read);
        assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
    }
    let m = scene();
    let read =
        m.f.store
            .try_native_replacement_scene_read(&m.origin, &m.sources)
            .unwrap();
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    assert!(m
        .f
        .store
        .prepare_native_replacement_commit_until(m.origin.clone(), &read, crate::now())
        .is_err());
    drop(read);
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
}

#[test]
fn native_prepared_shorter_cutoff_rolls_back_at_actual_dml_and_commit_boundaries() {
    for stage in [
        "expired-before-entry",
        "prepared-begin-before-dml",
        "prepared-begin-before-commit",
    ] {
        // The preparation deliberately expires at base + 1 policy second.
        // Wall-clock setup can otherwise consume that interval before the
        // requested SQL boundary. Step the existing thread-local policy clock
        // at the real boundary; the monotonic AuthorityWait remains untouched.
        let base = crate::now();
        let clock = std::rc::Rc::new(crate::test_clock::Clock::at(base));
        let m = scene();
        let planning =
            m.f.store
                .try_native_replacement_scene_read(&m.origin, &m.sources)
                .unwrap();
        let longer = planning.expires_at();
        let p =
            m.f.store
                .prepare_native_replacement_commit_until(m.origin.clone(), &planning, base + 1)
                .unwrap();
        assert!(p.expires_at() < longer);
        drop(planning);
        let wait = crate::AuthorityWait::new(std::time::Duration::from_secs(10));
        // Deliberately preserve a longer legitimate read/wait: ONLY the once
        // shortened preparation cutoff can refuse after the actual probe hold.
        let read =
            m.f.store
                .try_native_replacement_commit_scene_write_with_wait(
                    &m.origin, &m.sources, longer, &wait,
                )
                .unwrap();
        assert!(read.expires_at() > p.expires_at());
        let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
        let expires = p.expires_at();
        let hit = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let observed = hit.clone();
        let boundary_clock = clock.clone();
        let expired_before_entry = stage == "expired-before-entry";
        if expired_before_entry {
            clock.set(expires);
        }
        let probe = if expired_before_entry {
            "prepared-begin-before-dml"
        } else {
            stage
        };
        let result = Store::with_native_replacement_commit_entry_probe_for_test(
            probe,
            move || {
                assert_eq!(crate::now(), base, "boundary entered before expiry");
                observed.store(true, std::sync::atomic::Ordering::Release);
                boundary_clock.set(expires);
            },
            || {
                m.f.store
                    .try_begin_prepared_native_replacement_commit_with_wait(&p, &read, &wait)
            },
        );
        assert_eq!(
            hit.load(std::sync::atomic::Ordering::Acquire),
            !expired_before_entry,
            "exact requested boundary: {stage}"
        );
        assert_eq!(crate::now(), expires, "same original cutoff: {stage}");
        assert!(
            wait.check().is_ok() && read.current(),
            "longer read/wait must still be valid"
        );
        assert_eq!(
            result
                .err()
                .expect("original preparation cutoff must refuse")
                .code,
            "forbidden",
            "only the shorter original policy cutoff expired: {stage}"
        );
        drop(read);
        assert_eq!(
            Store::inspect_snapshot(m.f.store.path()).unwrap(),
            before,
            "same real transaction fully rolled back: {stage}"
        );
    }
}

#[test]
fn same_started_writer_readmission_revokes_exact_old_epoch_without_sql_mutation_and_fresh_read_recovers(
) {
    let m = scene();
    let preparation = prepare(&m);
    let attempt = begin(&m, &preparation);
    let original_ids = {
        let ids = attempt.attempt_binding();
        (ids.0.to_owned(), ids.1.to_owned(), ids.2.to_owned())
    };
    let sources = m.sources.clone();
    let store = Arc::new(m.f.store);
    let register = |target: &Arc<crate::NativeMediaOrigin>| {
        let weak = Arc::downgrade(&store);
        let target_copy = target.clone();
        store
            .register_native_media_snapshot_for(target, None, move || {
                weak.upgrade()
                    .ok_or_else(Error::forbidden)?
                    .try_native_media_read(&target_copy, None)
            })
            .unwrap()
    };
    let affected = register(&sources[0]);
    let disjoint = register(&sources[2]);
    let mut watcher = store.start_media_authorization_watcher().unwrap();
    let admit = |snapshot: &crate::NativeMediaSnapshot| {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            match snapshot.admit() {
                Ok(read) => break read,
                Err(error)
                    if error.code == "authority-busy" && std::time::Instant::now() < until =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(1))
                }
                Err(error) => panic!("actual current snapshot refused: {error:?}"),
            }
        }
    };
    let sql_before = Store::inspect_snapshot(store.path()).unwrap();
    for turn in 0..3 {
        let original = admit(&affected);
        let epoch = original.packet_epoch().unwrap();
        let stamp = original.snapshot_stamp().unwrap();
        assert!(original.current() && epoch.current());
        // Keep only metadata, never a Reader blocking the tested Writer.
        drop(original);
        let healthy = admit(&disjoint);
        let healthy_epoch = healthy.packet_epoch().unwrap();
        assert!(healthy_epoch.current());
        drop(healthy);
        let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let read = loop {
            match store.try_native_replacement_commit_attempt_scene_write(
                &attempt.admission(),
                &sources,
                &wait(),
            ) {
                Ok(read) => break read,
                Err(error)
                    if matches!(
                        error.code.as_str(),
                        "authority-busy" | "storage-unavailable"
                    ) && std::time::Instant::now() < until =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(1))
                }
                Err(error) => panic!("same started Writer admission refused: {error:?}"),
            }
        };
        assert!(read.current());
        assert!(!epoch.current(),"announced exact old-source Writer invalidates its original queued epoch even without SQL mutation");
        let (revocation, _) = epoch.sample_at_for_test(std::time::Instant::now());
        assert!(revocation.revoked && revocation.until_minus_sample_ns>0,
            "observe actual invalidation while original epoch deadline still live, never infer it from expiry: {revocation:?}");
        assert_eq!(
            revocation.first_revocation_state,
            "first_transition_before_original_deadline_cause_uncertain"
        );
        assert!(
            healthy_epoch.current(),
            "a disjoint source must not inherit old/candidate invalidation"
        );
        assert!(
            affected.admit_original(stamp).is_err(),
            "held same-started Writer cannot lend stale queued authority"
        );
        drop(read);
        assert_eq!(
            Store::inspect_snapshot(store.path()).unwrap(),
            sql_before,
            "same-started readmission cannot mutate operation/effect/lease/journal counts"
        );
        let fresh = admit(&affected);
        assert!(fresh.current() && fresh.packet_epoch().unwrap().current());
        assert!(fresh.snapshot_stamp().unwrap() != stamp);
        assert!(
            !epoch.current(),
            "refresh never revives the original queued epoch"
        );
        assert!(
            affected.admit_original(stamp).is_err(),
            "old packet cannot borrow the fresh generation"
        );
        assert!(healthy_epoch.current());
        drop(fresh);
        let ids = attempt.attempt_binding();
        assert_eq!(
            (ids.0, ids.1, ids.2),
            (
                original_ids.0.as_str(),
                original_ids.1.as_str(),
                original_ids.2.as_str()
            )
        );
        eprintln!("SAME_STARTED_WRITER_EPOCH turn={turn} old_revoked=true disjoint_current=true sql_unchanged=true fresh_recovered=true old_stamp_refused=true same_attempt=true");
    }
    watcher.shutdown().unwrap();
}

#[test]
fn native_cold_commit_metadata_actual_sql_exclusion_preserves_facts_and_one_begin() {
    let m = scene();
    let stable = counts(&m);
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    let planning =
        m.f.store
            .try_native_replacement_scene_read(&m.origin, &m.sources)
            .unwrap();
    {
        let _sql = m.f.store.connection().unwrap();
        let error =
            m.f.store
                .prepare_native_replacement_commit(m.origin.clone(), &planning)
                .err()
                .unwrap();
        assert_eq!(
            error.code, "authority-busy",
            "actual initial SQL mutex exclusion"
        );
    }
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
    let prepared =
        m.f.store
            .prepare_native_replacement_commit_until(m.origin.clone(), &planning, crate::now() + 5)
            .unwrap();
    let ids = prepared.attempt_binding();
    let ids = (ids.0.to_owned(), ids.1.to_owned(), ids.2.to_owned());
    drop(planning);
    let planning =
        m.f.store
            .try_native_replacement_scene_read(&m.origin, &m.sources)
            .unwrap();
    {
        let _sql = m.f.store.connection().unwrap();
        let error =
            m.f.store
                .revalidate_prepared_native_replacement_commit(&prepared, &planning)
                .err()
                .unwrap();
        assert_eq!(
            error.code, "authority-busy",
            "actual revalidation SQL mutex exclusion"
        );
    }
    assert_eq!(counts(&m), stable);
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
    m.f.store
        .revalidate_prepared_native_replacement_commit(&prepared, &planning)
        .unwrap();
    assert_eq!(
        prepared.attempt_binding(),
        (ids.0.as_str(), ids.1.as_str(), ids.2.as_str())
    );
    drop(planning);
    let attempt = begin(&m, &prepared);
    assert_eq!(attempt.attempt_binding(), prepared.attempt_binding());
    let after = Store::inspect_snapshot(m.f.store.path()).unwrap();
    let record = record(&m);
    let rows:(u64,u64,u64)=m.f.store.connection().unwrap().query_row(
        "SELECT count(*),min(e.attempt),min(e.submission_generation) FROM native_replacement_steps s JOIN operation_effects e ON e.id=s.effect_id WHERE s.operation_record_id=?1 AND s.phase='commit'",
        [&record], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(rows, (1, 1, 1));
    let read =
        m.f.store
            .try_native_replacement_commit_scene_write(&m.origin, &m.sources)
            .unwrap();
    assert!(m
        .f
        .store
        .try_begin_prepared_native_replacement_commit(&prepared, &read)
        .is_err());
    drop(read);
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), after);
}

#[test]
fn native_cold_commit_metadata_sql_fault_and_changed_or_expired_facts_stay_terminal() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    let m = scene();
    let prepared = prepare(&m);
    let planning =
        m.f.store
            .try_native_replacement_scene_read(&m.origin, &m.sources)
            .unwrap();
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    {
        let sql = m.f.store.connection().unwrap();
        sql.flush_prepared_statement_cache();
        sql.authorizer(Some(|action: AuthContext<'_>| {
            if matches!(
                action.action,
                AuthAction::Read {
                    table_name: "native_replacement_steps",
                    ..
                }
            ) {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }));
    }
    let initial =
        m.f.store
            .prepare_native_replacement_commit(m.origin.clone(), &planning)
            .err()
            .unwrap();
    let revalidate =
        m.f.store
            .revalidate_prepared_native_replacement_commit(&prepared, &planning)
            .err()
            .unwrap();
    {
        let sql = m.f.store.connection().unwrap();
        sql.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
        sql.flush_prepared_statement_cache();
    }
    assert_eq!(
        initial.code, "storage-unavailable",
        "actual denied SQLite read is not Busy"
    );
    assert_eq!(revalidate.code, "storage-unavailable");
    drop(planning);
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);

    for change in ["cancel", "revoked", "expired"] {
        let base = crate::now();
        let clock = crate::test_clock::Clock::at(base);
        let m = scene();
        let prepared = prepare(&m);
        let record = record(&m);
        if change == "expired" {
            clock.set(prepared.expires_at());
        } else {
            let sql = m.f.change_fixture();
            if change == "cancel" {
                sql.execute(
                    "UPDATE operations SET cancel_requested=1 WHERE record_id=?1",
                    [&record],
                )
                .unwrap();
            } else {
                sql.execute(
                    "UPDATE credentials SET revoked=1 WHERE id=?1",
                    [&m.f.keys[2]],
                )
                .unwrap();
            }
        }
        let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
        let actual =
            m.f.store
                .try_native_replacement_scene_read(&m.origin, &m.sources)
                .and_then(|planning| {
                    m.f.store
                        .revalidate_prepared_native_replacement_commit(&prepared, &planning)
                });
        assert!(actual.is_err(), "changed planning facts: {change}");
        assert_ne!(
            actual.unwrap_err().code,
            "authority-busy",
            "permanent refusal cannot acquire retry eligibility"
        );
        assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
        let count:u64=m.f.store.connection().unwrap().query_row("SELECT count(*) FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='commit'",[&record],|r|r.get(0)).unwrap();
        assert_eq!(count, 0, "no Commit begin after terminal planning refusal");
    }
}

#[test]
fn prepared_commit_keeps_current_media_through_journal_and_fences_at_cut() {
    use crate::native_prepared_begin_observation_for_test::NativePreparedBeginObservationForTest;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let m = scene();
    let preparation = prepare(&m);
    let sources = m.sources.clone();
    let affected = sources[0].clone();
    let healthy = sources[1].clone();
    let started = m.started.clone();
    let store = Arc::new(m.f.store);
    let (snapshot, mut watcher) = super::super::positive_capture::publication(&store, &affected);
    let io = super::super::positive_capture::admit(&snapshot);
    let epoch = io.packet_epoch().unwrap();
    let stamp = io.snapshot_stamp().unwrap();
    let before = Store::inspect_snapshot(store.path()).unwrap();
    let observation = Arc::new(NativePreparedBeginObservationForTest::new(
        preparation.attempt_binding(),
    ));
    observation.exclude_next_entry();
    let issued = Arc::new(AtomicUsize::new(0));
    let wait = crate::AuthorityWait::new(std::time::Duration::from_secs(1));
    let none = loop {
        let result = store.with_native_prepared_admission_observation_for_test(
            observation.clone(),
            &wait,
            || {
                store.try_admit_and_begin_prepared_native_replacement_commit(
                    &preparation,
                    &sources,
                    &wait,
                    || {
                        issued.fetch_add(1, Ordering::AcqRel);
                        Ok(())
                    },
                )
            },
        );
        match result {
            Err(error) if error.code == "authority-busy" && issued.load(Ordering::Acquire) == 0 => {
                std::thread::yield_now();
                assert!(
                    wait.check().is_ok(),
                    "bounded pure admission did not reach actual exclusion"
                );
            }
            result => break result.unwrap(),
        }
    };
    assert!(none.is_none());
    assert_eq!(
        issued.load(Ordering::Acquire),
        0,
        "primary exclusion cannot issue durable uncertainty"
    );
    assert!(io.current() && epoch.current());
    assert!(
        snapshot.admit_original(stamp).unwrap().current(),
        "no WriterNotice before primary ownership"
    );
    assert_eq!(
        Store::inspect_snapshot(store.path()).unwrap(),
        before,
        "no BEGIN/DML/journal under exclusion"
    );
    let first = observation.snapshot();
    assert_eq!(first.calls, first.pre_entry_refusals + 1);
    assert_eq!(
        (first.pre_entry_busy, first.none, first.exclusion_holds),
        (1, 1, 1)
    );
    assert_eq!(
        (
            first.issued,
            first.sql_begins,
            first.before_dml,
            first.errors
        ),
        (0, 0, 0, 0)
    );
    let cancelled = crate::AuthorityWait::new(std::time::Duration::from_secs(1));
    cancelled.cancel();
    let refusal = store
        .with_native_prepared_admission_observation_for_test(
            observation.clone(),
            &cancelled,
            || {
                store.try_admit_and_begin_prepared_native_replacement_commit(
                    &preparation,
                    &sources,
                    &cancelled,
                    || {
                        issued.fetch_add(1, Ordering::AcqRel);
                        Ok(())
                    },
                )
            },
        )
        .unwrap_err();
    assert_eq!(refusal.code, "cancelled");
    assert_eq!(issued.load(Ordering::Acquire), 0);
    assert!(epoch.current());
    assert_eq!(Store::inspect_snapshot(store.path()).unwrap(), before);
    let sql_owned = Arc::new(AtomicBool::new(false));
    let packet_observer_current = Arc::new(AtomicBool::new(false));
    let inspect_store = store.clone();
    let seen_owned = sql_owned.clone();
    let seen_packet = packet_observer_current.clone();
    let affected_snapshot = snapshot.clone();
    let successful = Store::with_native_replacement_commit_entry_probe_for_test(
        "prepared-begin-before-dml",
        move || {
            // Actual same primary mutex is still held inside the real SQL TX;
            // no nested Store unit is admitted here. The unrelated packet path
            // must continue through its own original observer connection.
            seen_owned.store(
                matches!(
                    inspect_store.connection.try_lock(),
                    Err(std::sync::TryLockError::WouldBlock)
                ),
                Ordering::Release,
            );
            seen_packet.store(
                inspect_store
                    .try_native_media_read(&healthy, None)
                    .is_ok_and(|read| read.current())
                    && affected_snapshot
                        .admit_original(stamp)
                        .is_ok_and(|read| read.current()),
                Ordering::Release,
            );
        },
        || loop {
            let result = store.with_native_prepared_admission_observation_for_test(
                observation.clone(),
                &wait,
                || {
                    store.try_admit_and_begin_prepared_native_replacement_commit(
                        &preparation,
                        &sources,
                        &wait,
                        || {
                            issued.fetch_add(1, Ordering::AcqRel);
                            Ok(())
                        },
                    )
                },
            );
            match result {
                Err(error)
                    if error.code == "authority-busy" && issued.load(Ordering::Acquire) == 0 =>
                {
                    std::thread::yield_now();
                    assert!(
                        wait.check().is_ok(),
                        "bounded pure admission did not recover"
                    );
                }
                result => break result,
            }
        },
    )
    .unwrap()
    .unwrap();
    let (read, attempt) = successful;
    assert!(
        read.is_none(),
        "journal-only admission cannot yield a Core Writer"
    );
    assert_eq!(attempt.attempt_binding(), preparation.attempt_binding());
    assert!(Arc::ptr_eq(attempt.activation(), preparation.activation()));
    assert!(
        io.current() && epoch.current(),
        "operation bookkeeping must preserve active source media"
    );
    let final_wait = crate::AuthorityWait::new(std::time::Duration::from_secs(1));
    let primary = store.connection().unwrap();
    let unavailable = store
        .try_native_replacement_commit_attempt_scene_write(
            &attempt.admission(),
            &sources,
            &final_wait,
        )
        .unwrap_err();
    assert_eq!(unavailable.code, "authority-busy");
    assert!(
        io.current() && epoch.current(),
        "a cut with no primary SQL capacity must preserve active source media"
    );
    drop(primary);
    let shared_cut = loop {
        match store.try_native_replacement_commit_attempt_scene_cut(
            &attempt.admission(),
            &sources,
            &final_wait,
        ) {
            Ok(read) => break read,
            Err(error) if error.code == "authority-busy" => {
                assert!(final_wait.check().is_ok());
                std::thread::yield_now();
            }
            Err(error) => panic!("current Core cut proof refused: {error:?}"),
        }
    };
    assert!(shared_cut.current() && io.current() && epoch.current());
    assert!(snapshot.admit_original(stamp).is_ok());
    // The external proof excludes a real racing endpoint mutation while the
    // Core turn is active, even though live original media can keep flowing.
    let scope =
        crate::AuthorityScope::connection(affected.connection_id(), crate::AuthorityMode::Write)
            .unwrap();
    match store.try_acquire_authority(&[scope]) {
        Err(error) => assert_eq!(error.code, "authority-busy"),
        Ok(_) => panic!("concurrent endpoint mutation crossed the live Core cut proof"),
    }
    drop(shared_cut);
    drop(io);
    let read = loop {
        match store.try_native_replacement_commit_attempt_scene_write(
            &attempt.admission(),
            &sources,
            &final_wait,
        ) {
            Ok(read) => break read,
            Err(error) if error.code == "authority-busy" => {
                assert!(
                    final_wait.check().is_ok(),
                    "the original Core cut did not acquire its Writers"
                );
                std::thread::yield_now();
            }
            Err(error) => panic!("original Core cut refused: {error:?}"),
        }
    };
    assert!(read.current());
    drop(read);
    watcher.shutdown().unwrap();
    assert!(sql_owned.load(Ordering::Acquire) && packet_observer_current.load(Ordering::Acquire));
    assert_eq!(issued.load(Ordering::Acquire), 1);
    let observed = observation.snapshot();
    assert_eq!(
        observed.calls,
        observed.none + observed.pre_entry_refusals + observed.issued
    );
    assert_eq!((observed.none, observed.issued), (1, 1));
    assert!(
        observed.pre_entry_refusals >= 1,
        "actual pre-cancelled call was not observed"
    );
    assert_eq!(
        (
            observed.sql_begins,
            observed.before_dml,
            observed.some,
            observed.errors
        ),
        (1, 1, 1, 0)
    );
    assert!(
        !epoch.current(),
        "the actual affected Writer still invalidates its old media epoch"
    );
    let c = store.connection().unwrap();
    let exact: (u64, u64) = c.query_row(
        "SELECT (SELECT count(*) FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='commit'),(SELECT count(*) FROM operation_effects WHERE operation_record_id=?1 AND provider='native-replacement-commit')",
        [preparation.attempt_binding().0], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    drop(c);
    assert_eq!(
        exact,
        (1, 1),
        "one original journal begin/effect after release"
    );
    let not_submitted = attempt.not_submitted();
    store
        .record_native_replacement_commit_not_submitted(&not_submitted)
        .unwrap();
    // Controlled Store fixture has no actual Core/resource submission. Preserve
    // the same existing absence cleanup, not a fabricated published result.
    let absent = super::super::super::candidate::Absent(
        preparation
            .activation()
            .origin()
            .runtime_connection_id()
            .into(),
        preparation.activation().origin().generation(),
    );
    store
        .record_native_replacement_started_candidate_stopped(&started, &absent)
        .unwrap();
}
