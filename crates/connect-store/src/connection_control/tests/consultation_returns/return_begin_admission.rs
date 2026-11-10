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
