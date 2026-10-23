//! Controlled Store captures only. These tests perform no SIP or Core IO.
use super::*;
use crate::{NativeReplacementPrepared, TelephoneReplacementAllocated};
use connect_protocol::transitions::{EffectState, OperationState};
use std::sync::Arc;

fn prepared(
    f: &Fixture,
    origin: &NativeReplacementOrigin,
    sources: &[Arc<crate::NativeMediaOrigin>],
) -> Arc<NativeReplacementPrepared> {
    let lease = prepare(f, origin, sources, &policy()).unwrap().unwrap();
    let expected = lease.plan().unwrap()["expected_preparation"].clone();
    let read = f
        .store
        .native_replacement_preparation_attempt_read(&lease)
        .unwrap();
    let (attempt, read) = f
        .store
        .begin_native_replacement_preparation(&lease, read)
        .unwrap();
    drop(read);
    let captured = Arc::new(attempt.prepared(expected).unwrap());
    f.store
        .record_native_replacement_prepared(&captured)
        .unwrap();
    captured
}
fn allocated(
    f: &Fixture,
    prepared: &NativeReplacementPrepared,
    key: &str,
) -> Arc<TelephoneReplacementAllocated> {
    let read = f
        .store
        .telephone_replacement_allocation_read(prepared, key, &policy())
        .unwrap();
    let (attempt, read) = f
        .store
        .begin_telephone_replacement_allocation(read)
        .unwrap();
    drop(read);
    let captured = Arc::new(allocation_capture(&attempt, "controlled-call@loopback").unwrap());
    f.store
        .record_telephone_replacement_allocated(&captured)
        .unwrap();
    captured
}
// Controlled snapshots only; production must populate these fields from the
// actual Core allocation and SIP context, never from this expected-value helper.
fn allocation_capture(
    attempt: &Arc<crate::TelephoneReplacementAttempt>,
    call_id: &str,
) -> Result<TelephoneReplacementAllocated> {
    let mut observation = attempt.expected_observation().clone();
    observation["call_reference"] = json!({"kind":"sip.call-id","value":call_id});
    attempt.allocated(observation)
}
fn caller(f: &Fixture, origin: &NativeReplacementOrigin) -> String {
    f.store
        .connection()
        .unwrap()
        .query_row(
            "SELECT credential_id FROM native_replacements WHERE operation_record_id=?1",
            [origin.record_id()],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn telephone_runtime_current_reader_binds_independent_credential_and_verified_route() {
    let (f, origin, sources, endpoint, route_key) = setup_runtime(true);
    let p = prepared(&f, &origin, &sources);
    let before = f.count("operation_effects");
    assert!(f
        .store
        .telephone_replacement_allocation_read(&p, &caller(&f, &origin), &policy())
        .is_err());
    let mut changed = policy();
    changed.confirmation_digits = "99".into();
    assert!(f
        .store
        .telephone_replacement_allocation_read(&p, &route_key, &changed)
        .is_err());
    let read = f
        .store
        .telephone_replacement_allocation_read(&p, &route_key, &policy())
        .unwrap();
    assert!(read.current());
    assert_eq!(read.route_actor().credential_id, route_key);
    assert_ne!(
        read.route_actor().identity_id,
        read.target_member().identity_id
    );
    assert_eq!(read.source().context().target_endpoint_id, endpoint);
    assert_eq!(
        read.destination().unwrap(),
        "sip:+14155550123@carrier.example:5060"
    );
    assert!(read.expires_at() <= crate::now() + 5);
    assert_eq!(f.count("operation_effects"), before);
    assert_eq!(f.count("sip_legs"), 0);
    assert!(format!("{read:?}").contains("redacted"));
}

#[test]
fn telephone_runtime_allocation_must_be_recorded_before_once_only_submission() {
    let (f, origin, sources, _, route_key) = setup_runtime(true);
    let p = prepared(&f, &origin, &sources);
    let connections = f.count("connections");
    let read = f
        .store
        .telephone_replacement_allocation_read(&p, &route_key, &policy())
        .unwrap();
    let (attempt, read) = f
        .store
        .begin_telephone_replacement_allocation(read)
        .unwrap();
    drop(read);
    let allocated = Arc::new(allocation_capture(&attempt, "controlled-call@loopback").unwrap());
    assert!(f
        .store
        .telephone_replacement_submission_read(allocated.clone())
        .is_err());
    let record = f
        .store
        .record_telephone_replacement_allocated(&allocated)
        .unwrap();
    assert_eq!(record.state, OperationState::Preparing);
    assert_eq!(
        f.store
            .record_telephone_replacement_allocated(&allocated)
            .unwrap()
            .revision,
        record.revision
    );
    let read = f
        .store
        .telephone_replacement_allocation_read(&p, &route_key, &policy())
        .unwrap();
    assert_eq!(
        f.store
            .begin_telephone_replacement_allocation(read)
            .unwrap_err()
            .code,
        "reconciliation-required"
    );
    let read = f
        .store
        .telephone_replacement_submission_read(allocated.clone())
        .unwrap();
    let (submission, read) = f
        .store
        .begin_telephone_replacement_submission(read)
        .unwrap();
    assert!(read.current());
    drop(read);
    let correlation: String = f.store.connection().unwrap().query_row(
        "SELECT e.correlation_key FROM operation_effects e JOIN native_replacement_steps s ON s.effect_id=e.id WHERE s.operation_record_id=?1 AND s.phase='start'",[origin.record_id()],|r|r.get(0)).unwrap();
    assert_eq!(correlation, allocated.call_id());
    let mut wrong = submission.expected_observation().clone();
    wrong["call_reference"]["value"] = json!("another-call@loopback");
    assert!(submission.submitted(wrong).is_err());
    let captured = submission
        .submitted(submission.expected_observation().clone())
        .unwrap();
    assert_eq!(captured.call_id(), allocated.call_id());
    let record = f
        .store
        .record_telephone_replacement_submitted(&captured)
        .unwrap();
    assert_eq!(record.state, OperationState::Preparing);
    assert!(
        record.result.is_none(),
        "submission cannot publish a replacement"
    );
    assert_eq!(
        f.store
            .record_telephone_replacement_submitted(&captured)
            .unwrap()
            .revision,
        record.revision
    );
    let read = f
        .store
        .telephone_replacement_submission_read(allocated)
        .unwrap();
    assert_eq!(
        f.store
            .begin_telephone_replacement_submission(read)
            .unwrap_err()
            .code,
        "reconciliation-required"
    );
    assert_eq!(f.count("native_replacement_steps"), 3);
    assert_eq!(f.count("connections"), connections);
    assert_eq!(f.count("sip_legs"), 0);
}

#[test]
fn telephone_runtime_unknown_allocation_can_record_late_knowledge_but_never_redial() {
    let (f, origin, sources, _, route_key) = setup_runtime(true);
    let p = prepared(&f, &origin, &sources);
    let read = f
        .store
        .telephone_replacement_allocation_read(&p, &route_key, &policy())
        .unwrap();
    let (attempt, read) = f
        .store
        .begin_telephone_replacement_allocation(read)
        .unwrap();
    drop(read);
    let uncertain = attempt.uncertain();
    let unknown = f
        .store
        .record_telephone_replacement_uncertain(&uncertain)
        .unwrap();
    assert_eq!(unknown.state, OperationState::Unknown);
    assert_eq!(
        f.store
            .record_telephone_replacement_uncertain(&uncertain)
            .unwrap()
            .revision,
        unknown.revision
    );
    let late = Arc::new(allocation_capture(&attempt, "late-controlled-call@loopback").unwrap());
    let record = f
        .store
        .record_telephone_replacement_allocated(&late)
        .unwrap();
    assert_eq!(record.state, OperationState::Unknown);
    assert!(record
        .effects
        .iter()
        .all(|e| e.state == EffectState::Confirmed));
    assert!(f.store.telephone_replacement_submission_read(late).is_err());
    assert!(f
        .store
        .telephone_replacement_allocation_read(&p, &route_key, &policy())
        .is_err());
    let changed = allocation_capture(&attempt, "different-call@loopback").unwrap();
    assert!(f
        .store
        .record_telephone_replacement_allocated(&changed)
        .is_err());
    assert_eq!(f.count("native_replacement_steps"), 2);
    assert!(f
        .store
        .claim_transition(&caller(&f, &origin), &origin.request().operation_id, 30)
        .unwrap()
        .is_none());
}

#[test]
fn telephone_runtime_unknown_submission_keeps_call_id_and_never_restores_root_state() {
    let (f, origin, sources, _, route_key) = setup_runtime(true);
    let p = prepared(&f, &origin, &sources);
    let a = allocated(&f, &p, &route_key);
    let read = f
        .store
        .telephone_replacement_submission_read(a.clone())
        .unwrap();
    let (attempt, read) = f
        .store
        .begin_telephone_replacement_submission(read)
        .unwrap();
    drop(read);
    let uncertain = attempt.uncertain();
    assert_eq!(
        f.store
            .record_telephone_replacement_uncertain(&uncertain)
            .unwrap()
            .state,
        OperationState::Unknown
    );
    let receipt = attempt
        .submitted(attempt.expected_observation().clone())
        .unwrap();
    let record = f
        .store
        .record_telephone_replacement_submitted(&receipt)
        .unwrap();
    assert_eq!(record.state, OperationState::Unknown);
    assert!(record.result.is_none());
    assert_eq!(receipt.call_id(), "controlled-call@loopback");
    assert!(f.store.telephone_replacement_submission_read(a).is_err());
    assert!(f
        .store
        .record_telephone_replacement_uncertain(&uncertain)
        .is_err());
    assert_eq!(f.count("native_replacement_steps"), 3);
    assert_eq!(f.count("sip_legs"), 0);
    assert_eq!(record.operation_id, origin.request().operation_id);
}

#[test]
fn telephone_runtime_current_route_fence_blocks_revocation_and_requires_fresh_authority() {
    let (f, origin, sources, _, route_key) = setup_runtime(true);
    let p = prepared(&f, &origin, &sources);
    let read = f
        .store
        .telephone_replacement_allocation_read(&p, &route_key, &policy())
        .unwrap();
    let scope = crate::AuthorityScope::identity(
        &read.route_actor().identity_id,
        crate::AuthorityMode::Write,
    )
    .unwrap();
    assert!(f
        .store
        .try_acquire_authority(std::slice::from_ref(&scope))
        .is_err());
    drop(read);
    let writer = f.store.try_acquire_authority(&[scope]).unwrap();
    assert!(f
        .store
        .telephone_replacement_allocation_read(&p, &route_key, &policy())
        .is_err());
    drop(writer);
    let admin = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&admin, &route_key).unwrap();
    assert!(f
        .store
        .telephone_replacement_allocation_read(&p, &route_key, &policy())
        .is_err());
    assert_eq!(f.count("native_replacement_steps"), 1);
}

#[test]
fn telephone_runtime_revocation_after_allocation_cannot_erase_capture_or_authorize_submission() {
    let (f, origin, sources, _, route_key) = setup_runtime(true);
    let p = prepared(&f, &origin, &sources);
    let read = f
        .store
        .telephone_replacement_allocation_read(&p, &route_key, &policy())
        .unwrap();
    let (attempt, read) = f
        .store
        .begin_telephone_replacement_allocation(read)
        .unwrap();
    drop(read);
    let receipt =
        Arc::new(allocation_capture(&attempt, "captured-before-revocation@loopback").unwrap());
    f.store
        .revoke(&f.store.revalidate(&f.admin).unwrap(), &route_key)
        .unwrap();
    let record = f
        .store
        .record_telephone_replacement_allocated(&receipt)
        .unwrap();
    assert_eq!(record.state, OperationState::Preparing);
    assert!(f
        .store
        .telephone_replacement_submission_read(receipt)
        .is_err());
    assert_eq!(f.count("native_replacement_steps"), 2);
}

#[test]
fn telephone_runtime_cancelled_root_records_known_allocation_without_revival() {
    let (f, origin, sources, _, route_key) = setup_runtime(true);
    let p = prepared(&f, &origin, &sources);
    let read = f
        .store
        .telephone_replacement_allocation_read(&p, &route_key, &policy())
        .unwrap();
    let (attempt, read) = f
        .store
        .begin_telephone_replacement_allocation(read)
        .unwrap();
    drop(read);
    let receipt =
        Arc::new(allocation_capture(&attempt, "captured-before-cancel@loopback").unwrap());
    f.store
        .execute_operation_action(
            &caller(&f, &origin),
            OperationAction::Cancel {
                operation_id: "cancel-telephone".into(),
                target_operation_id: origin.request().operation_id.clone(),
                reason: None,
            },
        )
        .unwrap();
    let cancelled = f
        .store
        .execute_operation_action(
            &caller(&f, &origin),
            OperationAction::Get {
                operation_id: origin.request().operation_id.clone(),
            },
        )
        .unwrap();
    assert_eq!(cancelled.state, OperationState::Unknown);
    let record = f
        .store
        .record_telephone_replacement_allocated(&receipt)
        .unwrap();
    assert_eq!(record.state, cancelled.state);
    assert!(record.cancel_requested);
    assert!(f
        .store
        .telephone_replacement_submission_read(receipt)
        .is_err());
    assert_eq!(f.count("native_replacement_steps"), 2);
}

#[test]
fn telephone_runtime_submission_restart_keeps_exact_correlation_and_no_second_attempt() {
    for captured in [false, true] {
        let (f, origin, sources, _, route_key) = setup_runtime(true);
        let p = prepared(&f, &origin, &sources);
        let a = allocated(&f, &p, &route_key);
        let read = f
            .store
            .telephone_replacement_submission_read(a.clone())
            .unwrap();
        let (attempt, read) = f
            .store
            .begin_telephone_replacement_submission(read)
            .unwrap();
        drop(read);
        let receipt = attempt
            .submitted(attempt.expected_observation().clone())
            .unwrap();
        if captured {
            f.store
                .record_telephone_replacement_submitted(&receipt)
                .unwrap();
        }
        let key = caller(&f, &origin);
        let path = f.store.path().to_path_buf();
        let Fixture { _dir, store, .. } = f;
        drop(store);
        let successor = Store::open(&path, Limits::default()).unwrap();
        assert!(successor
            .telephone_replacement_submission_write_read(attempt.clone())
            .is_err());
        let record = successor
            .execute_operation_action(
                &key,
                OperationAction::Get {
                    operation_id: origin.request().operation_id.clone(),
                },
            )
            .unwrap();
        assert_eq!(record.state, OperationState::Unknown);
        assert_eq!(record.effects.len(), 3);
        assert!(record.effects.iter().all(|e| e.attempt == 1));
        assert!(successor.telephone_replacement_submission_read(a).is_err());
        assert!(successor
            .record_telephone_replacement_submitted(&receipt)
            .is_err());
        assert!(successor
            .claim_transition(&key, &origin.request().operation_id, 30)
            .unwrap()
            .is_none());
        let (correlation,count): (String,u64) = successor.connection().unwrap().query_row(
            "SELECT e.correlation_key,(SELECT COUNT(*) FROM native_replacement_steps) FROM operation_effects e JOIN native_replacement_steps s ON s.effect_id=e.id WHERE s.operation_record_id=?1 AND s.phase='start'",[origin.record_id()],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(correlation, "controlled-call@loopback");
        assert_eq!(count, 3);
        drop(sources);
        drop(successor);
        drop(_dir);
    }
}

#[test]
fn telephone_runtime_wrong_phase_and_call_reference_cannot_create_a_new_effect() {
    let (f, origin, sources, _, route_key) = setup_runtime(true);
    let p = prepared(&f, &origin, &sources);
    let read = f
        .store
        .telephone_replacement_allocation_read(&p, &route_key, &policy())
        .unwrap();
    assert!(f
        .store
        .begin_telephone_replacement_submission(read)
        .is_err());
    let read = f
        .store
        .telephone_replacement_allocation_read(&p, &route_key, &policy())
        .unwrap();
    let (attempt, read) = f
        .store
        .begin_telephone_replacement_allocation(read)
        .unwrap();
    drop(read);
    assert!(allocation_capture(&attempt, "bad\r\nCall-ID").is_err());
    assert!(attempt
        .submitted(attempt.expected_observation().clone())
        .is_err());
    let mut wrong = attempt.expected_observation().clone();
    wrong["call_reference"] = json!({"kind":"sip.call-id","value":"controlled-call@loopback"});
    wrong["candidate"] = json!("foreign-core-binding");
    assert!(attempt.allocated(wrong).is_err());
    let mut invented_readiness = attempt.expected_observation().clone();
    invented_readiness["call_reference"] =
        json!({"kind":"sip.call-id","value":"controlled-call@loopback"});
    invented_readiness["media_ready"] = json!(true);
    assert!(attempt.allocated(invented_readiness).is_err());

    let receipt = allocation_capture(&attempt, "controlled-call@loopback").unwrap();
    assert!(!format!("{attempt:?} {receipt:?}").contains("controlled-call"));
    f.store
        .record_telephone_replacement_allocated(&receipt)
        .unwrap();
    assert_eq!(f.count("native_replacement_steps"), 2);
}

#[test]
fn telephone_runtime_expired_route_interval_refuses_begin_without_an_effect() {
    let (f, origin, sources, _, route_key) = setup_runtime(true);
    let p = prepared(&f, &origin, &sources);
    let owner = f.store.revalidate(&route_key).unwrap().identity_id;
    let short = f
        .store
        .issue(&owner, Scope::operator(), crate::now() + 2)
        .unwrap()
        .id;
    let read = f
        .store
        .telephone_replacement_allocation_read(&p, &short, &policy())
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2100));
    assert!(!read.current());
    assert!(f
        .store
        .begin_telephone_replacement_allocation(read)
        .is_err());
    assert_eq!(f.count("native_replacement_steps"), 1);
}

#[test]
fn telephone_runtime_route_credential_requires_its_own_dial_and_recipient_permissions() {
    let (f, origin, sources, _, route_key) = setup_runtime(true);
    let p = prepared(&f, &origin, &sources);
    let owner = f.store.revalidate(&route_key).unwrap().identity_id;
    for denied in ["dial", "recipients"] {
        let mut scope = Scope::operator();
        if denied == "dial" {
            scope.operations = vec!["session_get".into()];
        } else {
            scope.recipients.clear();
        }
        let key = f.store.issue(&owner, scope, crate::now() + 600).unwrap().id;
        assert!(
            f.store
                .telephone_replacement_allocation_read(&p, &key, &policy())
                .is_err(),
            "{denied}"
        );
    }
    assert_eq!(f.count("native_replacement_steps"), 1);
}

#[test]
fn telephone_runtime_current_endpoint_and_operator_are_required_after_allocation() {
    for revoked in ["endpoint", "operator"] {
        let (f, origin, sources, endpoint, route_key) = setup_runtime(true);
        let p = prepared(&f, &origin, &sources);
        let read = f
            .store
            .telephone_replacement_allocation_read(&p, &route_key, &policy())
            .unwrap();
        let (attempt, read) = f
            .store
            .begin_telephone_replacement_allocation(read)
            .unwrap();
        drop(read);
        let receipt =
            Arc::new(allocation_capture(&attempt, "known-before-current-loss@loopback").unwrap());
        if revoked == "endpoint" {
            f.store
                .execute_provisioning(
                    &f.admin,
                    ProvisioningCommand::RevokeEndpoint {
                        operation_id: "revoke-telephone-target".into(),
                        endpoint_id: endpoint,
                        expected_revision: 1,
                    },
                )
                .unwrap();
        } else {
            f.store
                .revoke(&f.store.revalidate(&f.admin).unwrap(), &caller(&f, &origin))
                .unwrap();
        }
        let record = f
            .store
            .record_telephone_replacement_allocated(&receipt)
            .unwrap();
        assert!(record.result.is_none());
        assert!(
            f.store
                .telephone_replacement_submission_read(receipt)
                .is_err(),
            "{revoked}"
        );
        assert_eq!(f.count("native_replacement_steps"), 2);
    }
}

fn submission_attempt(
    f: &Fixture,
    origin: &NativeReplacementOrigin,
    sources: &[Arc<crate::NativeMediaOrigin>],
    key: &str,
) -> Arc<crate::TelephoneReplacementAttempt> {
    let p = prepared(f, origin, sources);
    let captured = allocated(f, &p, key);
    let read = f
        .store
        .telephone_replacement_submission_read(captured)
        .unwrap();
    let (attempt, read) = f
        .store
        .begin_telephone_replacement_submission(read)
        .unwrap();
    drop(read);
    attempt
}

#[test]
fn telephone_runtime_each_physical_write_reacquires_exact_begun_submission_authority() {
    let (f, origin, sources, _, key) = setup_runtime(true);
    let p = prepared(&f, &origin, &sources);
    let read = f
        .store
        .telephone_replacement_allocation_read(&p, &key, &policy())
        .unwrap();
    let (allocation, read) = f
        .store
        .begin_telephone_replacement_allocation(read)
        .unwrap();
    drop(read);
    assert!(f
        .store
        .telephone_replacement_submission_write_read(allocation.clone())
        .is_err());
    let captured = Arc::new(allocation_capture(&allocation, "write-bound@loopback").unwrap());
    f.store
        .record_telephone_replacement_allocated(&captured)
        .unwrap();
    let read = f
        .store
        .telephone_replacement_submission_read(captured)
        .unwrap();
    let (attempt, read) = f
        .store
        .begin_telephone_replacement_submission(read)
        .unwrap();
    drop(read);
    let write = f
        .store
        .telephone_replacement_submission_write_read(attempt.clone())
        .unwrap();
    assert!(write.current());
    assert_eq!(write.call_id(), "write-bound@loopback");
    assert_eq!(
        write.destination().unwrap(),
        "sip:+14155550123@carrier.example:5060"
    );
    assert!(write.expires_at() <= crate::now() + 5);
    assert_eq!(write.route_actor().credential_id, key);
    assert_ne!(
        write.route_actor().identity_id,
        write.target_member().identity_id
    );
    let scope = crate::AuthorityScope::identity(
        &write.route_actor().identity_id,
        crate::AuthorityMode::Write,
    )
    .unwrap();
    assert!(f.store.try_acquire_authority(&[scope.clone()]).is_err());
    drop(write);
    let writer = f.store.try_acquire_authority(&[scope]).unwrap();
    assert!(f
        .store
        .telephone_replacement_submission_write_read(attempt.clone())
        .is_err());
    drop(writer);
    assert!(f
        .store
        .telephone_replacement_submission_write_read(attempt.clone())
        .unwrap()
        .current());
    assert!(f.store.connection().unwrap().execute("UPDATE operation_effects SET correlation_key='foreign-call' WHERE id IN (SELECT effect_id FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='start')", [origin.record_id()]).is_err(), "durable Call-ID correlation must remain immutable");
    assert!(f
        .store
        .telephone_replacement_submission_write_read(attempt)
        .unwrap()
        .current());
    assert_eq!(f.count("sip_legs"), 0);
}

#[test]
fn telephone_runtime_terminal_or_uncertain_submission_cannot_lend_another_write_interval() {
    for mode in ["known", "unknown", "cancelled"] {
        let (f, origin, sources, _, key) = setup_runtime(true);
        let attempt = submission_attempt(&f, &origin, &sources, &key);
        assert!(f
            .store
            .telephone_replacement_submission_write_read(attempt.clone())
            .unwrap()
            .current());
        let receipt = attempt
            .submitted(attempt.expected_observation().clone())
            .unwrap();
        if mode == "unknown" {
            f.store
                .record_telephone_replacement_uncertain(&attempt.uncertain())
                .unwrap();
        } else if mode == "cancelled" {
            f.store
                .execute_operation_action(
                    &caller(&f, &origin),
                    OperationAction::Cancel {
                        operation_id: "cancel-write-interval".into(),
                        target_operation_id: origin.request().operation_id.clone(),
                        reason: None,
                    },
                )
                .unwrap();
        } else {
            f.store
                .record_telephone_replacement_submitted(&receipt)
                .unwrap();
        }
        assert!(f
            .store
            .telephone_replacement_submission_write_read(attempt.clone())
            .is_err());
        // Late factual knowledge does not grant a retry or restore unknown state.
        let record = f
            .store
            .record_telephone_replacement_submitted(&receipt)
            .unwrap();
        if mode != "known" {
            assert_eq!(record.state, OperationState::Unknown);
        }
        if mode == "cancelled" {
            assert!(record.cancel_requested);
        }
        assert!(f
            .store
            .telephone_replacement_submission_write_read(attempt)
            .is_err());
        assert_eq!(f.count("native_replacement_steps"), 3);
        assert_eq!(f.count("sip_legs"), 0);
    }
}

#[test]
fn telephone_runtime_new_write_interval_refuses_revoked_route_credential() {
    let (f, origin, sources, _, key) = setup_runtime(true);
    let attempt = submission_attempt(&f, &origin, &sources, &key);
    assert!(f
        .store
        .telephone_replacement_submission_write_read(attempt.clone())
        .unwrap()
        .current());
    let admin = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&admin, &key).unwrap();
    assert!(f
        .store
        .telephone_replacement_submission_write_read(attempt)
        .is_err());
}

#[test]
fn telephone_preparation_unstarted_refusal_settles_exact_reservation_without_core_or_callback() {
    let (f, origin, sources, _, _) = setup_runtime(true);
    let lease = prepare(&f, &origin, &sources, &policy()).unwrap().unwrap();
    let before = f.count("connections");
    assert!(f
        .store
        .abort_native_replacement_preparation(&lease, "unrecognized")
        .is_err());
    let record = f
        .store
        .abort_native_replacement_preparation(&lease, "forbidden")
        .unwrap();
    assert_eq!(record.state, OperationState::Failed);
    assert!(record.result.is_none());
    assert_eq!(record.effects.len(), 1);
    assert_eq!(record.effects[0].attempt, 0);
    assert_eq!(record.effects[0].state, EffectState::Failed);
    assert_eq!(
        f.store
            .abort_native_replacement_preparation(&lease, "forbidden")
            .unwrap()
            .state,
        OperationState::Failed
    );
    assert!(f
        .store
        .native_replacement_preparation_attempt_read(&lease)
        .is_err());
    assert_eq!(f.count("connections"), before);
    assert_eq!(f.count("sip_legs"), 0);
}

#[test]
fn telephone_preparation_begun_attempt_cannot_be_reclassified_as_unstarted_or_begun_twice() {
    let (f, origin, sources, _, _) = setup_runtime(true);
    let lease = prepare(&f, &origin, &sources, &policy()).unwrap().unwrap();
    let expected = lease.plan().unwrap()["expected_preparation"].clone();
    let read = f
        .store
        .native_replacement_preparation_attempt_read(&lease)
        .unwrap();
    let (attempt, read) = f
        .store
        .begin_native_replacement_preparation(&lease, read)
        .unwrap();
    drop(read);
    assert_eq!(
        f.store
            .abort_native_replacement_preparation(&lease, "forbidden")
            .unwrap_err()
            .code,
        "reconciliation-required"
    );
    let read = f
        .store
        .native_replacement_preparation_attempt_read(&lease)
        .unwrap();
    assert_eq!(
        f.store
            .begin_native_replacement_preparation(&lease, read)
            .err()
            .unwrap()
            .code,
        "reconciliation-required"
    );
    let receipt = attempt.prepared(expected).unwrap();
    let record = f
        .store
        .record_native_replacement_prepared(&receipt)
        .unwrap();
    assert_eq!(record.state, OperationState::Preparing);
    assert_eq!(record.effects[0].state, EffectState::Confirmed);
    assert_eq!(record.effects[0].attempt, 1);
    assert_eq!(f.count("sip_legs"), 0);
}

#[test]
fn telephone_runtime_start_control_waits_for_sql_but_releases_authority_before_ring() {
    let (f, origin, sources, _, key) = setup_runtime(true);
    let attempt = submission_attempt(&f, &origin, &sources, &key);
    let sql = f.store.connection().unwrap();
    assert_eq!(
        f.store
            .telephone_replacement_submission_write_read(attempt.clone())
            .unwrap_err()
            .code,
        "authority-busy"
    );
    std::thread::scope(|scope| {
        let (tx, rx) = std::sync::mpsc::channel();
        let store = &f.store;
        let attempt = attempt.clone();
        scope.spawn(move || {
            tx.send(store.telephone_replacement_submission_control_read(attempt))
                .unwrap()
        });
        assert!(rx
            .recv_timeout(std::time::Duration::from_millis(50))
            .is_err());
        drop(sql);
        let read = rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap()
            .unwrap();
        assert!(read.current());
        assert_eq!(read.call_id(), "controlled-call@loopback");
        drop(read);
    });
    f.store
        .revoke(&f.store.revalidate(&f.admin).unwrap(), &key)
        .unwrap();
    assert!(f
        .store
        .telephone_replacement_submission_control_read(attempt.clone())
        .is_err());
    assert!(f
        .store
        .telephone_replacement_submission_write_read(attempt)
        .is_err());
    assert_eq!(f.count("native_replacement_steps"), 3);
    assert_eq!(f.count("sip_legs"), 0);
}

mod media;
