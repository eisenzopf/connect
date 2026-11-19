//! Real owned negative token + durable ledger, not a generic failed/Unknown row.
//! Closed-observer facts are controlled STORE facts only; no opaque native/Core
//! ClosedOriginalCandidate or physical source-restoration proof is fabricated.
use super::*;
use crate::{
    NativeReplacementPublicVideoClosedExpected as ClosedExpected,
    NativeReplacementPublicVideoClosedGuard as ClosedGuard,
    NativeReplacementPublicVideoClosedObservation as ClosedObservation,
};
struct ClosedFacts<'a> {
    reservation: &'a NativeReplacementPublicVideoReservation,
    calls: AtomicUsize,
}
struct HeldClose<'a>(&'a ClosedFacts<'a>);
impl ClosedObservation for ClosedFacts<'_> {
    fn hold_closed(
        &self,
        expected: &ClosedExpected<'_>,
    ) -> crate::Result<Option<Box<dyn ClosedGuard + '_>>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(expected.matches_original_reservation(self.reservation));
        assert_eq!(
            expected.original().operation_record_id(),
            self.reservation.operation_record_id()
        );
        assert!(expected.original().current());
        Ok(Some(Box::new(HeldClose(self))))
    }
}
impl ClosedGuard for HeldClose<'_> {
    fn reason(&self) -> crate::Result<wire::DiscardReason> {
        Ok(wire::DiscardReason::Refused)
    }
    fn publish(&self, commit: Box<dyn FnOnce() -> crate::Result<()> + '_>) -> crate::Result<()> {
        assert_eq!(self.0.calls.load(Ordering::SeqCst), 1);
        commit()
    }
}
fn negative(m: &ReadyFixture) -> crate::NativeReplacementCommitNotSubmitted {
    let read = m
        .scene
        .f
        .store
        .try_native_replacement_scene_write(&m.scene.origin, &m.scene.sources)
        .unwrap();
    let attempt = m
        .scene
        .f
        .store
        .begin_native_replacement_commit(m.scene.origin.clone(), &read)
        .unwrap();
    // This exact branch never invokes Core. A captured uncertain/confirmed
    // attempt has no conversion into this privately owned negative type.
    let capture = attempt.not_submitted();
    drop(read);
    capture
}
fn record_negative(
    m: &ReadyFixture,
    token: &crate::NativeReplacementCommitNotSubmitted,
) -> connect_protocol::transitions::OperationRecord {
    retry_effect_free_metadata(&m.scene.f, || {
        m.scene
            .f
            .store
            .record_native_replacement_commit_not_submitted(token)
    })
    .unwrap()
}
#[test]
fn typed_cleanup_requires_actual_same_owned_negative_durable_fact_and_preserves_value_clone_lineage(
) {
    let m = ready_fixture();
    issue(
        &m.scene.f,
        &m.scene.reservation,
        &m.scene.command,
        Stage::CutBoundPostF,
    );
    let captured = negative(&m);
    let before = Store::inspect_snapshot(m.scene.f.store.path()).unwrap();
    let wait = crate::AuthorityWait::new(std::time::Duration::from_secs(1));
    assert_eq!(
        m.scene
            .f
            .store
            .native_replacement_not_submitted_unpublished_read(m.prepared.clone(), &captured, &wait)
            .err()
            .unwrap()
            .code,
        "forbidden",
        "owned type alone does not prove durable negative recording"
    );
    assert_eq!(
        Store::inspect_snapshot(m.scene.f.store.path()).unwrap(),
        before
    );
    let recorded = record_negative(&m, &captured);
    assert_eq!(recorded.state, OperationState::Preparing);
    assert_eq!(
        recorded
            .effects
            .iter()
            .filter(|e| e.provider == "native-replacement-commit"
                && e.state == connect_protocol::transitions::EffectState::Failed)
            .count(),
        1
    );
    // Acquire cleanup custody AFTER the durable metadata recorder settles;
    // this new cleanup-only wait cannot renew any source/target preparation.
    let wait = crate::AuthorityWait::new(std::time::Duration::from_secs(1));
    let before = Store::inspect_snapshot(m.scene.f.store.path()).unwrap();
    assert_eq!(
        m.scene
            .f
            .store
            .native_replacement_known_unpublished_read(m.prepared.clone(), &wait)
            .err()
            .unwrap()
            .code,
        "forbidden",
        "ordinary historical predicate STILL excludes every Commit step"
    );
    let cloned = Arc::new(m.prepared.as_ref().clone());
    assert!(!Arc::ptr_eq(&cloned, &m.prepared));
    let read = m
        .scene
        .f
        .store
        .native_replacement_not_submitted_unpublished_read(cloned, &captured, &wait)
        .unwrap();
    assert!(read.current());
    assert_eq!(
        read.operation_record_id(),
        m.scene.reservation.operation_record_id()
    );
    assert_eq!(read.original_expires_at(), m.scene.reservation.expires_at());
    assert_eq!(
        Store::inspect_snapshot(m.scene.f.store.path()).unwrap(),
        before
    );
    let closed = ClosedFacts {
        reservation: &m.scene.reservation,
        calls: AtomicUsize::new(0),
    };
    let result = m
        .scene
        .f
        .store
        .record_native_replacement_public_video_discarded(&read, &closed)
        .unwrap();
    assert_eq!(closed.calls.load(Ordering::SeqCst), 1);
    let value = serde_json::to_value(result.phase).unwrap();
    assert_eq!(value["phase"], "discarded");
    assert_eq!(value["source_cut_id"], "controlled-original-cut");
    assert_eq!(value["binding"]["stream_id"], "public-b");
    assert_eq!(value["binding"]["primary_ssrc"], 0);
    assert_eq!(value["reason"], "refused");
    assert_eq!(readiness_rows(&m), 1);
    assert_eq!(m.scene.f.count("native_replacement_public_video_phases"), 3);
}
#[test]
fn typed_cleanup_real_generic_expiry_recovery_then_owned_negative_survives_cancel_revoke_and_original_expiry_without_new_grant(
) {
    let m = ready_fixture();
    issue(
        &m.scene.f,
        &m.scene.reservation,
        &m.scene.command,
        Stage::CutBoundPostF,
    );
    let captured = negative(&m);
    // Prove the exact already-owned preparation worker lease is what the
    // actual public batch will select. No immutable deadline/row is rewritten.
    let (same_lease, same_cutoff, until): (bool, bool, i64) = m.scene.f.store.try_connection().unwrap().query_row(
        "SELECT o.lease_owner=n.reservation_id,o.lease_expires_at=n.expires_at,n.expires_at FROM operations o JOIN native_replacements n ON n.operation_record_id=o.record_id WHERE o.record_id=?1",
        [m.scene.reservation.operation_record_id()], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).unwrap();
    assert!(same_lease && same_cutoff);
    assert_eq!(until, m.scene.reservation.expires_at());
    // Actual public startup/worker recovery, not raw fabricated Unknown SQL.
    // The negative token remains owned and Core was never invoked.
    let _expired = crate::test_clock::Clock::at(m.scene.reservation.expires_at());
    assert!(m.scene.f.store.recover_transition_work(64).unwrap() > 0);
    let recorded = record_negative(&m, &captured);
    assert_eq!(recorded.state, OperationState::Unknown);
    // Cancel under the original credential before revocation. Preparation's
    // expiry grants neither source nor target readmission here.
    custody::cancel(&m.scene.f, &m.scene.key, &m.scene.command);
    let admin = m.scene.f.store.revalidate(&m.scene.f.admin).unwrap();
    m.scene.f.store.revoke(&admin, &m.scene.key).unwrap();
    let before = Store::inspect_snapshot(m.scene.f.store.path()).unwrap();
    let counts = [
        m.scene.f.count("connections"),
        m.scene.f.count("operation_effects"),
        m.scene.f.count("journal"),
    ];
    let wait = crate::AuthorityWait::new(std::time::Duration::from_secs(1));
    assert!(m
        .scene
        .f
        .store
        .native_replacement_known_unpublished_read(m.prepared.clone(), &wait)
        .is_err());
    let read = m
        .scene
        .f
        .store
        .native_replacement_not_submitted_unpublished_read(m.prepared.clone(), &captured, &wait)
        .unwrap();
    assert!(read.current());
    assert!(read.original_expires_at() <= crate::now());
    assert_eq!(
        Store::inspect_snapshot(m.scene.f.store.path()).unwrap(),
        before
    );
    let closed = ClosedFacts {
        reservation: &m.scene.reservation,
        calls: AtomicUsize::new(0),
    };
    let result = m
        .scene
        .f
        .store
        .record_native_replacement_public_video_discarded(&read, &closed)
        .unwrap();
    assert_eq!(result.operation.state, OperationState::Unknown);
    assert!(result.operation.cancel_requested);
    assert_eq!(
        counts,
        [
            m.scene.f.count("connections"),
            m.scene.f.count("operation_effects"),
            m.scene.f.count("journal")
        ]
    );
    assert!(m
        .scene
        .f
        .store
        .native_replacement_origin(&m.scene.key, m.scene.command.clone())
        .is_err());
    wait.cancel();
    assert!(!read.current());
    let before = Store::inspect_snapshot(m.scene.f.store.path()).unwrap();
    assert!(m
        .scene
        .f
        .store
        .record_native_replacement_public_video_discarded(&read, &closed)
        .is_err());
    assert_eq!(
        Store::inspect_snapshot(m.scene.f.store.path()).unwrap(),
        before
    );
}
#[test]
fn typed_cleanup_refuses_foreign_original_negative_and_actual_commit_unknown_or_confirmed_without_minting_read(
) {
    let owner = ready_fixture();
    let token = negative(&owner);
    record_negative(&owner, &token);
    let other = ready_fixture();
    let before = Store::inspect_snapshot(other.scene.f.store.path()).unwrap();
    let wait = crate::AuthorityWait::new(std::time::Duration::from_secs(1));
    assert_eq!(
        other
            .scene
            .f
            .store
            .native_replacement_not_submitted_unpublished_read(
                other.prepared.clone(),
                &token,
                &wait
            )
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    assert_eq!(
        Store::inspect_snapshot(other.scene.f.store.path()).unwrap(),
        before
    );
    for confirmed in [false, true] {
        let m = ready_fixture();
        issue(
            &m.scene.f,
            &m.scene.reservation,
            &m.scene.command,
            Stage::CutBoundPostF,
        );
        if confirmed {
            let capture = capture(&m.scene);
            assert_eq!(
                capture.activation().origin().connection_id(),
                m.scene.reservation.candidate_connection_id()
            );
        } else {
            let read = m
                .scene
                .f
                .store
                .try_native_replacement_scene_write(&m.scene.origin, &m.scene.sources)
                .unwrap();
            let attempt = m
                .scene
                .f
                .store
                .begin_native_replacement_commit(m.scene.origin.clone(), &read)
                .unwrap();
            let uncertain = attempt.uncertain();
            drop(read);
            assert_eq!(
                m.scene
                    .f
                    .store
                    .native_replacement_commit_uncertain(&uncertain)
                    .unwrap()
                    .state,
                OperationState::Unknown
            );
        }
        let before = Store::inspect_snapshot(m.scene.f.store.path()).unwrap();
        let wait = crate::AuthorityWait::new(std::time::Duration::from_secs(1));
        assert!(m
            .scene
            .f
            .store
            .native_replacement_known_unpublished_read(m.prepared.clone(), &wait)
            .is_err());
        // The actual uncertain/confirmed types cannot be passed as a negative.
        // A genuine but FOREIGN negative token is not a fallback conversion.
        assert_eq!(
            m.scene
                .f
                .store
                .native_replacement_not_submitted_unpublished_read(
                    m.prepared.clone(),
                    &token,
                    &wait
                )
                .err()
                .unwrap()
                .code,
            "forbidden"
        );
        assert_eq!(
            Store::inspect_snapshot(m.scene.f.store.path()).unwrap(),
            before
        );
    }
}
#[test]
fn typed_cleanup_sql_exclusion_and_cancelled_own_wait_leave_durable_negative_and_all_metadata_unchanged(
) {
    let m = ready_fixture();
    let token = negative(&m);
    record_negative(&m, &token);
    let wait = crate::AuthorityWait::new(std::time::Duration::from_millis(50));
    let before = Store::inspect_snapshot(m.scene.f.store.path()).unwrap();
    let sql = m.scene.f.store.try_connection().unwrap();
    let refused = m
        .scene
        .f
        .store
        .native_replacement_not_submitted_unpublished_read(m.prepared.clone(), &token, &wait);
    assert_eq!(refused.err().unwrap().code, "authority-busy");
    drop(sql);
    assert_eq!(
        Store::inspect_snapshot(m.scene.f.store.path()).unwrap(),
        before
    );
    // A fresh cleanup-only budget after prior metadata refusal is not a source,
    // target or preparation renewal. Its own cancellation is still terminal.
    let cancelled = crate::AuthorityWait::new(std::time::Duration::from_secs(1));
    cancelled.cancel();
    assert!(m
        .scene
        .f
        .store
        .native_replacement_not_submitted_unpublished_read(m.prepared.clone(), &token, &cancelled)
        .is_err());
    assert_eq!(
        Store::inspect_snapshot(m.scene.f.store.path()).unwrap(),
        before
    );
}
