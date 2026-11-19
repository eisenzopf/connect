//! Controlled Store Close-observer facts ONLY. No ClosedOriginalCandidate,
//! native close, backend/Core terminal join or media proof is fabricated here.
use super::*;
use crate::{
    NativeReplacementPublicVideoClosedExpected as ClosedExpected,
    NativeReplacementPublicVideoClosedGuard as ClosedGuard,
    NativeReplacementPublicVideoClosedObservation as ClosedObservation,
};
#[derive(Clone, Copy)]
enum Closing {
    Commit,
    Refuse,
    Skip,
    CancelAndSwallow,
}
struct ControlledClosed<'a> {
    original: &'a NativeReplacementPublicVideoReservation,
    operation: &'a str,
    wait: &'a crate::AuthorityWait,
    reason: wire::DiscardReason,
    available: bool,
    reason_error: bool,
    closing: Closing,
    holds: AtomicUsize,
    publishes: AtomicUsize,
    callback_errors: AtomicUsize,
}
impl<'a> ControlledClosed<'a> {
    fn new(
        r: &'a NativeReplacementPublicVideoReservation,
        c: &'a TransitionCommand,
        wait: &'a crate::AuthorityWait,
    ) -> Self {
        Self {
            original: r,
            operation: &c.operation_id,
            wait,
            reason: wire::DiscardReason::Cancelled,
            available: true,
            reason_error: false,
            closing: Closing::Commit,
            holds: AtomicUsize::new(0),
            publishes: AtomicUsize::new(0),
            callback_errors: AtomicUsize::new(0),
        }
    }
}
struct HeldClosed<'a, 'r>(&'a ControlledClosed<'r>);
impl ClosedObservation for ControlledClosed<'_> {
    fn hold_closed(
        &self,
        expected: &ClosedExpected<'_>,
    ) -> crate::Result<Option<Box<dyn ClosedGuard + '_>>> {
        self.holds.fetch_add(1, Ordering::SeqCst);
        if !expected.matches_original_reservation(self.original)
            || expected.operation_id() != self.operation
        {
            return Err(crate::Error::forbidden());
        }
        assert!(expected.intent() == self.original.intent());
        assert_eq!(
            expected.original().operation_record_id(),
            self.original.operation_record_id()
        );
        assert_eq!(
            expected.original().candidate_connection_id(),
            self.original.candidate_connection_id()
        );
        assert_eq!(
            expected.original().candidate_runtime_connection_id(),
            self.original.candidate_runtime_connection_id()
        );
        assert_eq!(
            expected.original().original_expires_at(),
            self.original.expires_at()
        );
        assert!(expected.original().current());
        Ok(self
            .available
            .then(|| Box::new(HeldClosed(self)) as Box<dyn ClosedGuard + '_>))
    }
}
impl ClosedGuard for HeldClosed<'_, '_> {
    fn reason(&self) -> crate::Result<wire::DiscardReason> {
        if self.0.reason_error {
            Err(crate::Error::forbidden())
        } else {
            Ok(self.0.reason)
        }
    }
    fn publish(&self, commit: Box<dyn FnOnce() -> crate::Result<()> + '_>) -> crate::Result<()> {
        self.0.publishes.fetch_add(1, Ordering::SeqCst);
        match self.0.closing {
            Closing::Commit => commit(),
            Closing::Refuse => Err(crate::Error::forbidden()),
            Closing::Skip => Ok(()),
            Closing::CancelAndSwallow => {
                // Cancel ONLY this already-owned cleanup interval. No source,
                // target, preparation or new authority is obtained here.
                self.0.wait.cancel();
                let error = commit()
                    .err()
                    .expect("real callback must reject cancelled cleanup custody");
                assert_eq!(error.code, "forbidden");
                self.0.callback_errors.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        }
    }
}
fn discarded_value(value: crate::PublicVideoOperationSnapshot) -> serde_json::Value {
    json!({"operation": value.operation, "intent": value.intent, "phase": value.phase})
}
#[test]
fn closed_issuer_historical_cancel_revoke_expiry_marks_unbound_once_without_binding_or_permission_renewal(
) {
    let (f, key, c, prepared, _, _) = setup(1, None);
    let r = retry_effect_free_metadata(&f, || {
        f.store
            .reserve_native_replacement_public_video_intent(prepared.clone())
    })
    .unwrap();
    custody::cancel(&f, &key, &c);
    let old = snapshot(&f, &key, &c);
    let admin = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&admin, &key).unwrap();
    let _expired = crate::test_clock::Clock::at(r.expires_at());
    let wait = crate::AuthorityWait::new(std::time::Duration::from_secs(1));
    let read = f
        .store
        .native_replacement_known_unpublished_read(prepared, &wait)
        .unwrap();
    assert!(read.current());
    assert_eq!(read.original_expires_at(), r.expires_at());
    let counts = [
        f.count("connections"),
        f.count("operation_effects"),
        f.count("journal"),
    ];
    let closed = ControlledClosed::new(&r, &c, &wait);
    let result = discarded_value(
        f.store
            .record_native_replacement_public_video_discarded(&read, &closed)
            .unwrap(),
    );
    assert_eq!(result["operation"], old["operation"]);
    assert_eq!(result["intent"], old["intent"]);
    assert!(
        result["phase"].is_null(),
        "unbound cleanup must not invent MID/SSRC/Discarded binding"
    );
    assert_eq!(f.count("native_replacement_public_video_phases"), 0);
    assert_eq!(f.count("native_replacement_public_video_terminals"), 1);
    assert_eq!(
        counts,
        [
            f.count("connections"),
            f.count("operation_effects"),
            f.count("journal")
        ]
    );
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let replay = discarded_value(
        f.store
            .record_native_replacement_public_video_discarded(&read, &closed)
            .unwrap(),
    );
    assert_eq!(replay, result);
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    assert!(
        f.store.native_replacement_origin(&key, c.clone()).is_err(),
        "historical cleanup never readmits revoked/expired Root authority"
    );
    let mut changed = ControlledClosed::new(&r, &c, &wait);
    changed.reason = wire::DiscardReason::Refused;
    assert!(f
        .store
        .record_native_replacement_public_video_discarded(&read, &changed)
        .is_err());
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    wait.cancel();
    assert!(!read.current());
    let untouched = ControlledClosed::new(&r, &c, &wait);
    assert!(f
        .store
        .record_native_replacement_public_video_discarded(&read, &untouched)
        .is_err());
    assert_eq!(untouched.holds.load(Ordering::SeqCst), 0);
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
}
#[test]
fn closed_issuer_bound_discard_preserves_only_original_binding_and_known_cut_and_exact_replay() {
    for count in 1..=3 {
        let (f, key, c, prepared, _, _) = setup(1, None);
        let r = retry_effect_free_metadata(&f, || {
            f.store
                .reserve_native_replacement_public_video_intent(prepared.clone())
        })
        .unwrap();
        for stage in [
            Stage::NegotiatedNeverFed,
            Stage::CutBoundPostF,
            Stage::DecodedReady,
        ]
        .into_iter()
        .take(count)
        {
            issue(&f, &r, &c, stage);
        }
        let prior = snapshot(&f, &key, &c);
        custody::cancel(&f, &key, &c);
        let cancelled = snapshot(&f, &key, &c);
        let wait = crate::AuthorityWait::new(std::time::Duration::from_secs(1));
        let read = f
            .store
            .native_replacement_known_unpublished_read(prepared, &wait)
            .unwrap();
        let closed = ControlledClosed::new(&r, &c, &wait);
        let result = discarded_value(
            f.store
                .record_native_replacement_public_video_discarded(&read, &closed)
                .unwrap(),
        );
        assert_eq!(result["operation"], cancelled["operation"]);
        assert_eq!(result["intent"], prior["intent"]);
        assert_eq!(result["phase"]["phase"], "discarded");
        assert_eq!(result["phase"]["binding"], prior["phase"]["binding"]);
        assert_eq!(result["phase"]["reason"], "cancelled");
        assert_eq!(
            result["phase"]["source_cut_id"],
            if count == 1 {
                serde_json::Value::Null
            } else {
                prior["phase"]["source_cut_id"].clone()
            }
        );
        assert!(result["phase"].get("public_readiness").is_none());
        assert_eq!(
            f.count("native_replacement_public_video_phases"),
            count as u64 + 1
        );
        assert_eq!(f.count("native_replacement_public_video_terminals"), 0);
        let before = Store::inspect_snapshot(f.store.path()).unwrap();
        assert_eq!(
            discarded_value(
                f.store
                    .record_native_replacement_public_video_discarded(&read, &closed)
                    .unwrap()
            ),
            result
        );
        let mut changed = ControlledClosed::new(&r, &c, &wait);
        changed.reason = wire::DiscardReason::OwnerRetired;
        assert!(f
            .store
            .record_native_replacement_public_video_discarded(&read, &changed)
            .is_err());
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
        assert_eq!(closed.publishes.load(Ordering::SeqCst), 2);
    }
}
#[test]
fn closed_issuer_no_owner_foreign_original_reason_error_skip_and_publish_error_preserve_original_snapshot(
) {
    for fault in 0..5 {
        let (f, key, c, prepared, _, _) = setup(1, None);
        let r = retry_effect_free_metadata(&f, || {
            f.store
                .reserve_native_replacement_public_video_intent(prepared.clone())
        })
        .unwrap();
        issue(&f, &r, &c, Stage::NegotiatedNeverFed);
        custody::cancel(&f, &key, &c);
        let wait = crate::AuthorityWait::new(std::time::Duration::from_secs(1));
        let read = f
            .store
            .native_replacement_known_unpublished_read(prepared, &wait)
            .unwrap();
        let before = Store::inspect_snapshot(f.store.path()).unwrap();
        let mut closed = ControlledClosed::new(&r, &c, &wait);
        match fault {
            0 => closed.available = false,
            1 => closed.reason_error = true,
            2 => closed.closing = Closing::Skip,
            3 => closed.closing = Closing::Refuse,
            4 => closed.closing = Closing::CancelAndSwallow,
            _ => unreachable!(),
        }
        let result = f
            .store
            .record_native_replacement_public_video_discarded(&read, &closed);
        let error = result.err().expect("faulty closed observer must refuse");
        assert_eq!(closed.holds.load(Ordering::SeqCst), 1);
        assert_eq!(
            closed.publishes.load(Ordering::SeqCst),
            if fault < 2 { 0 } else { 1 },
            "fault={fault}"
        );
        assert_eq!(
            error.code,
            match fault {
                0 => "authority-busy",
                1 | 3 => "forbidden",
                2 | 4 => "storage-unavailable",
                _ => unreachable!(),
            },
            "fault={fault}"
        );
        if fault == 4 {
            assert_eq!(closed.callback_errors.load(Ordering::SeqCst), 1);
            assert!(!read.current());
        }
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    }
    // A genuine different original reservation cannot supply close custody,
    // even though the controlled observer knows the same API/string profile.
    let (f, key, c, prepared, _, _) = setup(1, None);
    let r = retry_effect_free_metadata(&f, || {
        f.store
            .reserve_native_replacement_public_video_intent(prepared.clone())
    })
    .unwrap();
    custody::cancel(&f, &key, &c);
    let (foreign, _, other_c, other_p, _, _) = setup(1, None);
    let other_r = retry_effect_free_metadata(&foreign, || {
        foreign
            .store
            .reserve_native_replacement_public_video_intent(other_p.clone())
    })
    .unwrap();
    let wait = crate::AuthorityWait::new(std::time::Duration::from_secs(1));
    let read = f
        .store
        .native_replacement_known_unpublished_read(prepared, &wait)
        .unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let wrong = ControlledClosed::new(&other_r, &other_c, &wait);
    assert!(f
        .store
        .record_native_replacement_public_video_discarded(&read, &wrong)
        .is_err());
    assert_eq!(wrong.holds.load(Ordering::SeqCst), 1);
    assert_eq!(wrong.publishes.load(Ordering::SeqCst), 0);
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
}
#[test]
fn closed_issuer_sql_exclusion_and_cancelled_original_cleanup_wait_never_consult_close_observer() {
    let (f, key, c, prepared, _, _) = setup(1, None);
    let r = retry_effect_free_metadata(&f, || {
        f.store
            .reserve_native_replacement_public_video_intent(prepared.clone())
    })
    .unwrap();
    custody::cancel(&f, &key, &c);
    let wait = crate::AuthorityWait::new(std::time::Duration::from_secs(1));
    let read = f
        .store
        .native_replacement_known_unpublished_read(prepared, &wait)
        .unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let closed = ControlledClosed::new(&r, &c, &wait);
    let sql = f.store.try_connection().unwrap();
    assert_eq!(
        f.store
            .record_native_replacement_public_video_discarded(&read, &closed)
            .err()
            .unwrap()
            .code,
        "authority-busy"
    );
    drop(sql);
    assert_eq!(closed.holds.load(Ordering::SeqCst), 0);
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    wait.cancel();
    assert!(!read.current());
    assert!(f
        .store
        .record_native_replacement_public_video_discarded(&read, &closed)
        .is_err());
    assert_eq!(closed.holds.load(Ordering::SeqCst), 0);
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
}
