//! Controlled source custody only: no handoff engine/media execution occurs here.
use super::*;
use connect_protocol::transitions::{
    OperationAction, OperationState, OutgoingDisposition, RoleScope, TransitionCommand,
    TransitionIntent,
};
use std::sync::Arc;
fn handoff_key(f: &Fixture) -> String {
    let owner = f.store.revalidate(&f.keys[0]).unwrap();
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope.operations.extend([
        "session.handoff".into(),
        "operation.cancel".into(),
        "operation.execute".into(),
    ]);
    f.store
        .issue(&owner.identity_id, scope, crate::now() + 600)
        .unwrap()
        .id
}
fn command(
    f: &Fixture,
    sources: &[Arc<crate::NativeMediaOrigin>],
    revision: u64,
) -> TransitionCommand {
    TransitionCommand {
        operation_id: "native-role-takeover".into(),
        conversation_id: f.cid.clone(),
        session_id: "session".into(),
        connection_id: Some(sources[0].connection_id().into()),
        expected_revision: revision,
        intent: TransitionIntent::Handoff {
            source_participant: f.pids[0].clone(),
            target_participant: f.pids[2].clone(),
            retained_connid: sources[1].connection_id().into(),
            candidate_connid: Some(sources[2].connection_id().into()),
            outgoing: OutgoingDisposition::Observe,
            role_scope: RoleScope::Session,
            expected_conversation_revision: None,
        },
        payload_extensions: std::collections::BTreeMap::from([(
            "policy_ref".into(),
            json!("target-policy-reference"),
        )]),
    }
}
fn fixture() -> (
    Fixture,
    Vec<Arc<crate::NativeMediaOrigin>>,
    String,
    TransitionCommand,
) {
    let (f, sources, consult, _) = consultation_reads::fixture();
    f.store
        .execute_operation_action(
            &f.keys[0],
            OperationAction::Cancel {
                operation_id: "cancel-initial-consult-intention".into(),
                target_operation_id: consult.operation_id,
                reason: None,
            },
        )
        .unwrap();
    let key = handoff_key(&f);
    let command = command(&f, &sources, 3);
    (f, sources, key, command)
}
#[test]
fn handoff_preflight_refusal_is_exact_attributable_and_idempotent_without_an_effect() {
    let (f, _, key, command) = fixture();
    let accepted = f
        .store
        .admit_transition_command(&key, command.clone())
        .unwrap();
    let mut changed = command.clone();
    changed
        .payload_extensions
        .insert("policy_ref".into(), json!("another-policy"));
    assert!(f
        .store
        .fail_native_handoff_preflight(&key, changed, "forbidden")
        .is_err());
    assert!(f
        .store
        .fail_native_handoff_preflight(&key, command.clone(), "not-ready")
        .is_err());
    let journal = f.count("journal");
    assert!(f
        .store
        .fail_native_handoff_preflight(&key, command.clone(), "forbidden")
        .unwrap());
    let record = f
        .store
        .execute_operation_action(
            &key,
            OperationAction::Get {
                operation_id: command.operation_id.clone(),
            },
        )
        .unwrap();
    assert_eq!(record.record_id, accepted.record_id);
    assert_eq!(record.state, OperationState::Failed);
    assert_eq!(
        record.problem,
        Some(json!({"code":"forbidden","source":"native-handoff"}))
    );
    assert!(record.effects.is_empty());
    assert!(record.result.is_none());
    assert_eq!(f.count("native_handoffs"), 0);
    assert_eq!(f.count("journal"), journal + 1);
    assert!(!f
        .store
        .fail_native_handoff_preflight(&key, command, "unsupported-media-profile")
        .unwrap());
    assert_eq!(f.count("journal"), journal + 1);
}
#[test]
fn handoff_preflight_refusal_requires_current_owner_grant() {
    let (f, _, key, command) = fixture();
    f.store
        .admit_transition_command(&key, command.clone())
        .unwrap();
    for ungranted in [&f.keys[0], &f.keys[1]] {
        assert!(f
            .store
            .fail_native_handoff_preflight(ungranted, command.clone(), "forbidden")
            .is_err());
    }
    let admin = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&admin, &key).unwrap();
    assert!(f
        .store
        .fail_native_handoff_preflight(&key, command.clone(), "forbidden")
        .is_err());
    let fresh = handoff_key(&f);
    let status = f
        .store
        .execute_operation_action(
            &fresh,
            OperationAction::Get {
                operation_id: command.operation_id.clone(),
            },
        )
        .unwrap();
    assert_eq!(status.state, OperationState::Accepted);
    assert!(f
        .store
        .fail_native_handoff_preflight(&fresh, command, "unsupported-media-profile")
        .unwrap());
}
#[test]
fn handoff_preflight_refusal_cannot_settle_prepared_or_uncertain_native_io() {
    for phase in ["prepared", "begun", "unknown"] {
        let (f, sources, origin) = handoff_custody::fixture(OutgoingDisposition::Observe);
        let r = origin.request();
        let command = TransitionCommand {
            operation_id: r.operation_id.clone(),
            conversation_id: r.conversation_id.clone(),
            session_id: r.session_id.clone(),
            connection_id: r.connection_id.clone(),
            expected_revision: r.expected_revision,
            intent: r.intent.clone(),
            payload_extensions: r.payload_extensions.clone(),
        };
        let lease = handoff_custody::prepare(&f, &sources, &origin, OutgoingDisposition::Observe);
        if phase != "prepared" {
            let attempt = handoff_custody::begin(&f, lease);
            if phase == "unknown" {
                f.store.native_handoff_uncertain(attempt).unwrap();
            }
        }
        let before = f.count("journal");
        assert!(!f
            .store
            .fail_native_handoff_preflight(&f.keys[0], command.clone(), "forbidden")
            .unwrap());
        let record = f
            .store
            .execute_operation_action(
                &f.keys[0],
                OperationAction::Get {
                    operation_id: command.operation_id,
                },
            )
            .unwrap();
        assert_eq!(
            record.state,
            if phase == "unknown" {
                OperationState::Unknown
            } else {
                OperationState::Preparing
            }
        );
        assert_eq!(record.effects.len(), 1);
        assert!(record.problem.is_none());
        assert_eq!(f.count("journal"), before);
    }
}
#[test]
fn handoff_source_requires_an_exact_accepted_intention_and_retains_no_execution_claim() {
    let (f, sources, key, command) = fixture();
    assert!(f
        .store
        .native_handoff_origin(&key, command.clone())
        .is_err());
    let accepted = f
        .store
        .admit_transition_command(&key, command.clone())
        .unwrap();
    let effects = f.count("operation_effects");
    let origin = f
        .store
        .native_handoff_origin(&key, command.clone())
        .unwrap();
    assert_eq!(origin.record_id(), accepted.record_id);
    assert_eq!(format!("{origin:?}"), "NativeHandoffOrigin(redacted)");
    let context = f.store.native_handoff_context(&origin).unwrap();
    assert_eq!(
        context.source_connection_id.as_deref(),
        Some(sources[0].connection_id())
    );
    assert_eq!(
        context.target_connection_id.as_deref(),
        Some(sources[2].connection_id())
    );
    assert_eq!(context.policy_ref, "target-policy-reference");
    assert_eq!(context.consultation_record_id, None);
    assert_eq!(f.count("operation_effects"), effects);
    let record = f
        .store
        .execute_operation_action(
            &key,
            OperationAction::Get {
                operation_id: command.operation_id.clone(),
            },
        )
        .unwrap();
    assert_eq!(record.state, OperationState::Accepted);
    assert!(record.effects.is_empty());
    assert!(record.result.is_none());
    let mut changed = command;
    changed
        .payload_extensions
        .insert("policy_ref".into(), json!("different-reference"));
    assert!(f.store.native_handoff_origin(&key, changed).is_err());
}
#[test]
fn handoff_source_policy_reference_shape_never_substitutes_for_current_permission() {
    for value in [None, Some(json!({"approved":true})), Some(json!(""))] {
        let (f, _, key, mut command) = fixture();
        command.payload_extensions.remove("policy_ref");
        if let Some(v) = value {
            command.payload_extensions.insert("policy_ref".into(), v);
        }
        f.store
            .admit_transition_command(&key, command.clone())
            .unwrap();
        assert!(f.store.native_handoff_origin(&key, command).is_err());
        assert_eq!(f.count("native_consultation_attempts"), 0);
    }
    let (f, _, key, command) = fixture();
    f.store
        .admit_transition_command(&key, command.clone())
        .unwrap();
    let origin = f
        .store
        .native_handoff_origin(&key, command.clone())
        .unwrap();
    let owner = f.store.revalidate(&f.keys[0]).unwrap();
    let ungranted = f
        .store
        .issue(
            &owner.identity_id,
            Scope::collaboration(vec!["*".into()]),
            crate::now() + 600,
        )
        .unwrap();
    assert!(f
        .store
        .native_handoff_origin(&ungranted.id, command.clone())
        .is_err());
    let operator = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&operator, &key).unwrap();
    assert!(f.store.native_handoff_context(&origin).is_err());
    let fresh = handoff_key(&f);
    let current = f.store.native_handoff_origin(&fresh, command).unwrap();
    assert_eq!(current.record_id(), origin.record_id());
}
#[test]
fn handoff_source_replay_rechecks_generation_membership_cancellation_and_worker_custody() {
    for change in ["generation", "membership", "cancel", "worker"] {
        let (f, sources, key, command) = fixture();
        f.store
            .admit_transition_command(&key, command.clone())
            .unwrap();
        let origin = f
            .store
            .native_handoff_origin(&key, command.clone())
            .unwrap();
        match change {
            "generation" => {
                f.store
                    .connection()
                    .unwrap()
                    .execute(
                        "UPDATE connections SET generation=generation+1 WHERE id=?1",
                        [sources[2].connection_id()],
                    )
                    .unwrap();
            }
            "membership" => {
                f.store
                    .connection()
                    .unwrap()
                    .execute(
                        "UPDATE participants SET active=0 WHERE participant_id=?1",
                        [&f.pids[2]],
                    )
                    .unwrap();
            }
            "cancel" => {
                f.store
                    .execute_operation_action(
                        &key,
                        OperationAction::Cancel {
                            operation_id: "cancel-handoff-source".into(),
                            target_operation_id: command.operation_id.clone(),
                            reason: None,
                        },
                    )
                    .unwrap();
            }
            "worker" => {
                assert!(f
                    .store
                    .claim_transition(&key, &command.operation_id, 30)
                    .unwrap()
                    .is_some());
            }
            _ => unreachable!(),
        }
        assert!(f.store.native_handoff_context(&origin).is_err(), "{change}");
        assert!(
            f.store.native_handoff_origin(&key, command).is_err(),
            "{change}"
        );
    }
}
#[test]
fn handoff_source_has_no_idle_authority_and_cannot_cross_installation_or_restart() {
    let (f, _, key, command) = fixture();
    f.store
        .admit_transition_command(&key, command.clone())
        .unwrap();
    let origin = f.store.native_handoff_origin(&key, command).unwrap();
    let writer = f
        .store
        .try_acquire_authority(&[crate::AuthorityScope::conversation(
            &f.cid,
            crate::AuthorityMode::Write,
        )
        .unwrap()])
        .unwrap();
    drop(writer);
    let other = Store::open_administration(f.store.path(), Limits::default()).unwrap();
    assert!(other.native_handoff_context(&origin).is_err());
    drop(other);
    let path = f.store.path().to_path_buf();
    let Fixture { _dir, store, .. } = f;
    drop(store);
    let successor = Store::open(&path, Limits::default()).unwrap();
    assert!(successor.native_handoff_context(&origin).is_err());
    drop(successor);
    drop(_dir);
}
#[test]
fn handoff_source_resolves_its_original_private_pair_without_borrowing_old_execution_authority() {
    let (f, mut sources, candidate, private, execution, receipt) =
        consultation_audiences::established_fixture();
    f.store
        .record_native_consultation_decoded(&receipt)
        .unwrap();
    f.store
        .complete_native_consultation(&receipt)
        .unwrap()
        .unwrap();
    let operator = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&operator, &execution).unwrap();
    sources[2] = candidate;
    let key = handoff_key(&f);
    let mut command = command(&f, &sources, 4);
    command.connection_id = None;
    if let TransitionIntent::Handoff {
        candidate_connid, ..
    } = &mut command.intent
    {
        *candidate_connid = None;
    }
    command
        .payload_extensions
        .insert("consultation_record_id".into(), json!(private.record_id()));
    f.store
        .admit_transition_command(&key, command.clone())
        .unwrap();
    let origin = f.store.native_handoff_origin(&key, command).unwrap();
    let context = f.store.native_handoff_context(&origin).unwrap();
    assert_eq!(
        context.source_connection_id.as_deref(),
        Some(sources[0].connection_id())
    );
    assert_eq!(
        context.target_connection_id.as_deref(),
        Some(sources[2].connection_id())
    );
    assert_eq!(context.retained_connection_id, sources[1].connection_id());
    assert_eq!(
        context.consultation_record_id.as_deref(),
        Some(private.record_id())
    );
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE connections SET generation=generation+1 WHERE id=?1",
            [sources[2].connection_id()],
        )
        .unwrap();
    assert!(f.store.native_handoff_context(&origin).is_err());
}
