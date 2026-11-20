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
