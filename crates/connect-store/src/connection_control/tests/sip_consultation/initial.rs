//! Controlled call-intent/dialog custody. These are not physical SIP proofs.
use super::*;
use crate::{InitialTelephoneOrigin, InitialTelephoneParent, InitialTelephoneSignaling};
use std::sync::Arc;

struct ControlledAbsent(bool);
impl crate::TelephoneConnectionRuntimeObservation for ControlledAbsent {
    fn observe_absent(&self, id: &str, generation: u64) -> Result<()> {
        if self.0 && id == "controlled-private-core" && generation == 1 {
            Ok(())
        } else {
            Err(Error::forbidden())
        }
    }
}

fn prepared() -> (
    Fixture,
    NativeConsultationOrigin,
    InitialTelephonePolicy,
    crate::SipConsultationReservation,
    InitialTelephoneOrigin,
) {
    let (f, parent, policy) = fixture();
    let reservation = f
        .store
        .reserve_sip_consultation_leg(&parent, policy.clone())
        .unwrap();
    let initial = f
        .store
        .prepare_private_initial_telephone(&reservation)
        .unwrap();
    (f, parent, policy, reservation, initial)
}
fn signaling(f: &Fixture, initial: &InitialTelephoneOrigin) -> Arc<InitialTelephoneSignaling> {
    let attempt = Arc::new(
        f.store
            .begin_initial_telephone_allocation(initial)
            .unwrap()
            .unwrap(),
    );
    f.store
        .record_initial_telephone_allocation(
            &attempt,
            "controlled-private-core",
            "private-call@loopback",
        )
        .unwrap();
    Arc::new(
        f.store
            .begin_initial_telephone_signaling(attempt)
            .unwrap()
            .unwrap(),
    )
}

#[test]
fn private_call_intent_uses_original_consultation_and_reserved_id_without_public_invitation() {
    let (mut f, parent, policy) = fixture();
    f.store
        .connection()
        .unwrap()
        .execute("UPDATE sessions SET state='active' WHERE id='session'", [])
        .unwrap();
    f.store.limits.connections = 4;
    let reservation = f
        .store
        .reserve_sip_consultation_leg(&parent, policy)
        .unwrap();
    let before = counts(&f);
    let initial = f
        .store
        .prepare_private_initial_telephone(&reservation)
        .unwrap();
    assert_eq!(initial.connection_id(), reservation.connection_id());
    assert!(
        matches!(&initial.source().parent,InitialTelephoneParent::Consultation { record_id,connection_id } if record_id==parent.record_id() && connection_id==reservation.connection_id())
    );
    assert_eq!(
        initial
            .source()
            .audience
            .iter()
            .map(|m| m.participant_id.clone())
            .collect::<std::collections::BTreeSet<_>>(),
        [f.pids[0].clone(), f.pids[2].clone()].into()
    );
    assert_eq!(f.count("connections"), 4);
    let after = counts(&f);
    assert_eq!(after[0], before[0] + 1);
    assert_eq!(after[1], before[1]);
    assert_eq!(after[3], before[3]);
    assert_eq!(after[4], before[4] + 1);
    let c = f.store.connection().unwrap();
    let parent_receipts: u64 = c
        .query_row(
            "SELECT count(*) FROM operation_control_receipts WHERE operation_record_id=?1",
            [parent.record_id()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(parent_receipts, 0);
    let parent_state: (String, String) = c
        .query_row(
            "SELECT command_type,state FROM operations WHERE record_id=?1",
            [parent.record_id()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(parent_state, ("session.consult".into(), "accepted".into()));
    let members: u64 = c
        .query_row(
            "SELECT count(*) FROM operation_members WHERE operation_record_id=?1",
            [initial.record_id()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(members, 2);
    let hidden: u64 = c
        .query_row(
            "SELECT count(*) FROM journal WHERE operation_record_id=?1 AND recipient!=?2",
            params![initial.record_id(), initial.source().caller.identity_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(hidden, 0);
}

#[test]
fn private_call_replay_and_once_only_allocation_preserve_canonical_and_physical_ids() {
    let (f, _, _, reservation, initial) = prepared();
    let before = (
        f.count("connections"),
        f.count("operations"),
        f.count("journal"),
    );
    let again = f
        .store
        .prepare_private_initial_telephone(&reservation)
        .unwrap();
    assert_eq!(again.record_id(), initial.record_id());
    assert_eq!(again.operation_id(), initial.operation_id());
    assert_eq!(again.connection_id(), initial.connection_id());
    let attempt = Arc::new(
        f.store
            .begin_initial_telephone_allocation(&initial)
            .unwrap()
            .unwrap(),
    );
    assert!(f
        .store
        .begin_initial_telephone_allocation(&again)
        .unwrap()
        .is_none());
    assert!(f
        .store
        .begin_initial_telephone_signaling(attempt.clone())
        .is_err());
    for _ in 0..2 {
        f.store
            .record_initial_telephone_allocation(
                &attempt,
                "controlled-private-core",
                "private-call@loopback",
            )
            .unwrap();
    }
    assert!(f
        .store
        .record_initial_telephone_allocation(&attempt, "other-core", "private-call@loopback")
        .is_err());
    assert!(f
        .store
        .record_initial_telephone_allocation(
            &attempt,
            "controlled-private-core",
            "other-call@loopback"
        )
        .is_err());
    let signal = f
        .store
        .begin_initial_telephone_signaling(attempt.clone())
        .unwrap()
        .unwrap();
    assert_eq!(signal.runtime_connection_id(), "controlled-private-core");
    assert_eq!(signal.sip_call_id(), "private-call@loopback");
    assert!(f
        .store
        .begin_initial_telephone_signaling(attempt)
        .unwrap()
        .is_none());
    let replay = f
        .store
        .prepare_private_initial_telephone(&reservation)
        .unwrap();
    assert_eq!(replay.record_id(), initial.record_id());
    assert_eq!(f.count("connections"), before.0);
    assert_eq!(f.count("operations"), before.1);
    assert_eq!(f.count("operation_effects"), 4);
}

#[test]
fn private_call_intervals_fence_complete_original_execution_and_route_and_release_idle_readers() {
    let (f, _, policy, _, initial) = prepared();
    let attempt = f
        .store
        .begin_initial_telephone_allocation(&initial)
        .unwrap()
        .unwrap();
    let read = f.store.initial_telephone_allocation_read(&attempt).unwrap();
    assert!(read.current());
    assert!(read.source().is_private_candidate());
    assert_eq!(
        read.endpoint_actor().credential_id,
        policy.endpoint_credential_id
    );
    assert_eq!(read.route_actor().credential_id, policy.route_credential_id);
    for key in &f.keys {
        let identity = f.store.revalidate(key).unwrap().identity_id;
        assert!(f
            .store
            .try_acquire_authority(&[
                AuthorityScope::identity(&identity, AuthorityMode::Write).unwrap()
            ])
            .is_err());
    }
    drop(read);
    for key in &f.keys {
        let identity = f.store.revalidate(key).unwrap().identity_id;
        drop(
            f.store
                .try_acquire_authority(&[
                    AuthorityScope::identity(&identity, AuthorityMode::Write).unwrap()
                ])
                .unwrap(),
        );
    }
    drop(
        f.store
            .try_acquire_authority(&[
                AuthorityScope::conversation(&f.cid, AuthorityMode::Write).unwrap()
            ])
            .unwrap(),
    );
}

#[test]
fn private_call_current_authority_is_rechecked_before_every_allocation_and_signal_interval() {
    for fault in 0..6 {
        let (f, parent, policy, reservation, initial) = prepared();
        let signal = signaling(&f, &initial);
        let read = f.store.initial_telephone_signaling_read(&signal).unwrap();
        assert!(read.current());
        drop(read);
        let (sql, id) = match fault {
            0 => (
                "UPDATE credentials SET revoked=1 WHERE id=?1",
                f.keys[0].as_str(),
            ),
            1 => (
                "UPDATE credentials SET revoked=1 WHERE id=?1",
                policy.endpoint_credential_id.as_str(),
            ),
            2 => (
                "UPDATE credentials SET operations='[]' WHERE id=?1",
                policy.route_credential_id.as_str(),
            ),
            3 => (
                "UPDATE endpoints SET revision=revision+1 WHERE id=?1",
                policy.endpoint_id.as_str(),
            ),
            4 => (
                "UPDATE participants SET active=0 WHERE participant_id=?1",
                f.pids[1].as_str(),
            ),
            _ => (
                "UPDATE operations SET cancel_requested=1 WHERE record_id=?1",
                parent.record_id(),
            ),
        };
        f.store.connection().unwrap().execute(sql, [id]).unwrap();
        assert!(
            f.store.initial_telephone_signaling_read(&signal).is_err(),
            "fault {fault}"
        );
        assert!(
            f.store
                .prepare_private_initial_telephone(&reservation)
                .is_err(),
            "fault {fault}"
        );
        assert_eq!(f.count("connections"), 4);
        assert_eq!(f.count("initial_telephone_operations"), 1);
    }
}

#[test]
fn private_call_intent_cannot_retire_through_unused_metadata_cleanup_or_redial_after_abandon() {
    let (f, _, _, reservation, initial) = prepared();
    assert!(f
        .store
        .retire_unused_sip_consultation_reservation(&reservation)
        .is_err());
    f.store
        .connection()
        .unwrap()
        .execute("UPDATE credentials SET revoked=1 WHERE id=?1", [&f.keys[0]])
        .unwrap();
    for _ in 0..2 {
        f.store
            .abandon_initial_telephone_unstarted(&initial)
            .unwrap();
    }
    let state: (String, u64) = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT state,generation FROM connections WHERE id=?1",
            [initial.connection_id()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, ("retired".into(), 2));
    assert!(f
        .store
        .prepare_private_initial_telephone(&reservation)
        .is_err());
    assert!(f
        .store
        .begin_initial_telephone_allocation(&initial)
        .is_err());
    assert_eq!(f.count("connections"), 4);
    assert_eq!(f.count("initial_telephone_operations"), 1);
}

#[test]
fn private_call_captured_answer_cannot_take_public_acceptance_or_publish_ready_media() {
    let (f, _, _, _, initial) = prepared();
    let signal = signaling(&f, &initial);
    let receipts = f.count("operation_control_receipts");
    let capture = signal.capture_answer(signal.sip_call_id(), "PCMU").unwrap();
    for _ in 0..2 {
        let result = f.store.record_initial_telephone_answer(&capture).unwrap();
        assert!(result.media.is_none());
        assert_eq!(result.problem.as_deref(), Some("private-context-required"));
    }
    assert_eq!(f.count("operation_control_receipts"), receipts);
    let c = f.store.connection().unwrap();
    let row:(bool,Option<String>,Option<String>)=c.query_row("SELECT accepted,acceptance_record_id,connected_json FROM initial_telephone_answers WHERE operation_record_id=?1",[initial.record_id()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(row, (false, None, None));
    let row: (String, Option<String>, Option<String>) = c
        .query_row(
            "SELECT state,observed_runtime_id,readiness_json FROM connections WHERE id=?1",
            [initial.connection_id()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(row, ("unknown".into(), None, None));
    assert!(!private::observation_source_current(&f.store, &c, initial.connection_id()).unwrap());
}

#[test]
fn private_call_child_parent_and_original_policy_are_immutable() {
    let (f, parent, _, _, initial) = prepared();
    let c = f.store.connection().unwrap();
    for sql in ["UPDATE initial_telephone_operations SET parent_kind='invitation' WHERE operation_record_id=?1", "UPDATE initial_telephone_operations SET parent_record_id='another-root' WHERE operation_record_id=?1", "UPDATE initial_telephone_operations SET policy_json='{}' WHERE operation_record_id=?1"] {
        assert!(c.execute(sql,[initial.record_id()]).is_err());
    }
    let kind:(String,String)=c.query_row("SELECT parent_kind,parent_record_id FROM initial_telephone_operations WHERE operation_record_id=?1",[initial.record_id()],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(kind, ("consultation".into(), parent.record_id().into()));
}

#[test]
fn private_call_prepare_refuses_claimed_root_and_receipt_capacity_without_partial_intent() {
    for capacity in [false, true] {
        let (mut f, parent, policy) = fixture();
        let reservation = f
            .store
            .reserve_sip_consultation_leg(&parent, policy)
            .unwrap();
        let before = counts(&f);
        if capacity {
            f.store.limits.operation_receipts = f.count("operations");
        } else {
            f.store
                .connection()
                .unwrap()
                .execute(
                    "UPDATE operations SET lease_owner='another-worker' WHERE record_id=?1",
                    [parent.record_id()],
                )
                .unwrap();
        }
        assert!(f
            .store
            .prepare_private_initial_telephone(&reservation)
            .is_err());
        assert_eq!(f.count("connections"), 4);
        assert_eq!(counts(&f), before);
    }
}

#[test]
fn private_call_pending_allocation_prevents_false_parent_failure_until_owned_cleanup() {
    let (f, parent, _, _, initial) = prepared();
    let signal = signaling(&f, &initial);
    assert!(!f
        .store
        .fail_native_consultation_initialization(&parent, "private-call-refused")
        .unwrap());
    let command = connect_protocol::transitions::TransitionCommand {
        operation_id: parent.request().operation_id.clone(),
        conversation_id: parent.request().conversation_id.clone(),
        session_id: parent.request().session_id.clone(),
        connection_id: parent.request().connection_id.clone(),
        expected_revision: parent.request().expected_revision,
        intent: parent.request().intent.clone(),
        payload_extensions: parent.request().payload_extensions.clone(),
    };
    assert!(!f
        .store
        .fail_native_consultation_preflight(&f.keys[0], command)
        .unwrap());
    assert!(f
        .store
        .settle_initial_telephone_cleanup(&signal, &ControlledAbsent(false))
        .is_err());
    assert!(!f
        .store
        .fail_native_consultation_initialization(&parent, "private-call-refused")
        .unwrap());
    f.store
        .connection()
        .unwrap()
        .execute("UPDATE credentials SET revoked=1 WHERE id=?1", [&f.keys[0]])
        .unwrap();
    for _ in 0..2 {
        f.store
            .settle_initial_telephone_cleanup(&signal, &ControlledAbsent(true))
            .unwrap();
    }
    assert!(f
        .store
        .fail_native_consultation_initialization(&parent, "private-call-refused")
        .unwrap());
    let c = f.store.connection().unwrap();
    let state: String = c
        .query_row(
            "SELECT state FROM operations WHERE record_id=?1",
            [parent.record_id()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "failed");
    let effects: u64 = c
        .query_row(
            "SELECT count(*) FROM operation_effects WHERE operation_record_id=?1",
            [initial.record_id()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(effects, 2);
    let submitted:String=c.query_row("SELECT e.state FROM operation_effects e JOIN initial_telephone_operations i ON i.effect_id=e.id WHERE i.operation_record_id=?1",[initial.record_id()],|r|r.get(0)).unwrap();
    assert_eq!(submitted, "unknown");
}

#[test]
fn private_call_parent_cancel_preserves_unknown_child_custody_and_refuses_further_io() {
    let (f, parent, _, _, initial) = prepared();
    let signal = signaling(&f, &initial);
    f.store
        .execute_operation_action(
            &f.keys[0],
            connect_protocol::transitions::OperationAction::Cancel {
                operation_id: "cancel-private-call-root".into(),
                target_operation_id: parent.request().operation_id.clone(),
                reason: None,
            },
        )
        .unwrap();
    let state: (String, bool) = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT state,cancel_requested FROM operations WHERE record_id=?1",
            [parent.record_id()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, ("unknown".into(), true));
    assert!(f.store.initial_telephone_signaling_read(&signal).is_err());
    f.store
        .settle_initial_telephone_cleanup(&signal, &ControlledAbsent(true))
        .unwrap();
}

#[test]
fn private_call_parent_recovery_preserves_pending_custody_without_synthetic_completion() {
    let (f, parent, _, _, initial) = prepared();
    let signal = signaling(&f, &initial);
    // Controlled stale worker metadata triggers the normal recovery scanner;
    // it is not an assertion about an actual process or wire call.
    f.store.connection().unwrap().execute("UPDATE operations SET lease_owner='controlled-stale-root',worker_runtime='fenced',lease_expires_at=0 WHERE record_id=?1",[parent.record_id()]).unwrap();
    assert!(f.store.recover_transition_work(100).unwrap() > 0);
    let state: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT state FROM operations WHERE record_id=?1",
            [parent.record_id()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "unknown");
    assert!(f.store.initial_telephone_signaling_read(&signal).is_err());
    f.store
        .settle_initial_telephone_cleanup(&signal, &ControlledAbsent(true))
        .unwrap();
}

#[test]
fn private_call_sql_parent_binding_refuses_forged_audience_epoch_and_endpoint() {
    let (f, _, _, _, initial) = prepared();
    let original = serde_json::to_value(initial.source()).unwrap();
    for fault in 0..7 {
        let mut wrong = original.clone();
        match fault {
            0 => wrong["parent"]["kind"] = json!("invitation"),
            1 => wrong["audience"][0]["participant_id"] = json!(f.pids[1]),
            2 => wrong["audience"][1] = wrong["audience"][0].clone(),
            3 => wrong["audience"][0]["membership_epoch"] = json!(2),
            4 => wrong["endpoint_revision"] = json!(999),
            5 => wrong["session_id"] = json!("other-session"),
            _ => wrong["device_id"] = json!("foreign-device"),
        }
        let mut c = f.store.connection().unwrap();
        let tx = c.transaction().unwrap();
        tx.execute("INSERT INTO operations(record_id,tenant_id,owner_id,operation_id,digest,command_type,conversation_id,session_id,connection_id,intent_json,state,created_at,updated_at,expires_at,ledger_version) SELECT 'forged-private-child',tenant_id,owner_id,'forged-private-operation',digest,command_type,conversation_id,session_id,connection_id,intent_json,'accepted',created_at,updated_at,expires_at,ledger_version FROM operations WHERE record_id=?1",[initial.record_id()]).unwrap();
        let error=tx.execute("INSERT INTO initial_telephone_operations(operation_record_id,parent_record_id,parent_kind,caller_credential_id,target_participant_id,endpoint_id,source_json,policy_json) SELECT 'forged-private-child',parent_record_id,parent_kind,caller_credential_id,target_participant_id,endpoint_id,?2,policy_json FROM initial_telephone_operations WHERE operation_record_id=?1",params![initial.record_id(),wrong.to_string()]).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("initial telephone source binding mismatch"),
            "fault {fault}: {error}"
        );
        // Roll back the forged receipt as well as the refused binding.
    }
}

#[test]
fn private_call_pending_child_is_excluded_from_terminal_parent_purge_selection() {
    let (f, parent, _, _, initial) = prepared();
    let signal = signaling(&f, &initial);
    let old = crate::now() - 31 * 86400;
    let c = f.store.connection().unwrap();
    // Controlled inconsistent historical parent: the current initialization
    // API refuses to create this terminal state while its call is pending.
    c.execute(
        "UPDATE operations SET state='interrupted' WHERE record_id=?1",
        [parent.record_id()],
    )
    .unwrap();
    c.execute(
        "UPDATE operations SET state='failed',updated_at=?2 WHERE record_id=?1",
        params![parent.record_id(), old],
    )
    .unwrap();
    c.execute("UPDATE conversations SET closed=1 WHERE id=?1", [&f.cid])
        .unwrap();
    c.execute(
        "UPDATE conversations SET closed_at=?2 WHERE id=?1",
        params![f.cid, old],
    )
    .unwrap();
    drop(c);
    f.store.purge_transition_results(100).unwrap();
    let c = f.store.connection().unwrap();
    let retained: bool = c
        .query_row(
            "SELECT tombstoned=0 AND intent_json IS NOT NULL FROM operations WHERE record_id=?1",
            [parent.record_id()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(retained);
    drop(c);
    f.store
        .settle_initial_telephone_cleanup(&signal, &ControlledAbsent(true))
        .unwrap();
}
