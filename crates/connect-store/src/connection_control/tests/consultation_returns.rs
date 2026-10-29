//! Controlled return admission; these rows are not native restore evidence.
use super::*;
use crate::NativeMediaOrigin;
use connect_protocol::transitions::{
    OperationAction, OperationState, TransitionCommand, TransitionIntent,
};
use std::sync::Arc;

fn fixture() -> (Fixture, TransitionCommand, String, String) {
    let (f, command, key, parent, _) = fixture_full();
    (f, command, key, parent)
}
fn fixture_full() -> (
    Fixture,
    TransitionCommand,
    String,
    String,
    Vec<Arc<NativeMediaOrigin>>,
) {
    let (f, mut sources, candidate, origin, execution, receipt) =
        consultation_audiences::established_fixture();
    f.store
        .record_native_consultation_decoded(&receipt)
        .unwrap();
    f.store
        .complete_native_consultation(&receipt)
        .unwrap()
        .unwrap();
    let actor = f.store.revalidate(&f.keys[0]).unwrap();
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope
        .operations
        .extend(["session.consult.return".into(), "operation.cancel".into()]);
    let key = f
        .store
        .issue(&actor.identity_id, scope, crate::now() + 600)
        .unwrap()
        .id;
    let operator = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&operator, &execution).unwrap();
    let selection: Value = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT selection_json FROM native_consultation_attempts WHERE operation_record_id=?1",
            [origin.record_id()],
            |r| r.get::<_, String>(0),
        )
        .map(|s| serde_json::from_str(&s).unwrap())
        .unwrap();
    let command = TransitionCommand {
        operation_id: "return-to-public".into(),
        conversation_id: f.cid.clone(),
        session_id: "session".into(),
        connection_id: selection["private"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["participant_id"] == f.pids[0])
            .unwrap()["connection_id"]
            .as_str()
            .map(str::to_owned),
        expected_revision: 4,
        intent: TransitionIntent::ConsultReturn {
            consultation_record_id: origin.record_id().into(),
            retained_connid: selection["remote"]["connection_id"]
                .as_str()
                .unwrap()
                .into(),
        },
        payload_extensions: Default::default(),
    };
    sources.push(candidate);
    (f, command, key, origin.record_id().into(), sources)
}
#[test]
fn native_return_new_current_permission_and_exact_original_context_precede_zero_io_intention() {
    let (f, mut command, key, parent) = fixture();
    command
        .payload_extensions
        .insert("future".into(), json!({"ordered":["b","a"],"null":null}));
    assert!(!f.store.revalidate(&key).unwrap().permits("session.consult"));
    assert!(f
        .store
        .admit_transition_command(&f.keys[0], command.clone())
        .is_err());
    let before = f.count("operations");
    let accepted = f
        .store
        .admit_transition_command(&key, command.clone())
        .unwrap();
    assert_eq!(accepted.state, OperationState::Accepted);
    assert!(accepted.effects.is_empty());
    assert_eq!(f.count("operations"), before + 1);
    let replay = f
        .store
        .admit_transition_command(&key, command.clone())
        .unwrap();
    assert_eq!(accepted, replay);
    let mut changed = command.clone();
    changed.payload_extensions.clear();
    assert_eq!(
        f.store
            .admit_transition_command(&key, changed)
            .unwrap_err()
            .code,
        "conflict"
    );
    assert!(f
        .store
        .claim_transition(&f.admin, &command.operation_id, 60)
        .unwrap()
        .is_none());
    assert_eq!(
        f.store
            .claim_transition_cleanup(&f.admin, &command.operation_id, 60)
            .unwrap_err()
            .code,
        "conflict"
    );
    let c = f.store.connection().unwrap();
    assert_eq!(c.query_row("SELECT consultation_record_id FROM native_consultation_returns WHERE operation_record_id=?1",[&accepted.record_id],|r|r.get::<_,String>(0)).unwrap(),parent);
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM native_private_content_floors WHERE is_candidate=0",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
}
#[test]
fn native_return_changed_primary_context_revision_and_original_epoch_refuse_before_reservation() {
    for change in ["context", "primary", "remote", "revision"] {
        let (f, mut command, key, _) = fixture();
        match change {
            "context" => {
                let TransitionIntent::ConsultReturn {
                    consultation_record_id,
                    ..
                } = &mut command.intent
                else {
                    panic!()
                };
                *consultation_record_id = "not-an-applied-context".into();
            }
            "primary" => command.connection_id = Some("missing-primary".into()),
            "remote" => {
                let TransitionIntent::ConsultReturn {
                    retained_connid, ..
                } = &mut command.intent
                else {
                    panic!()
                };
                *retained_connid = command.connection_id.clone().unwrap();
            }
            "revision" => command.expected_revision = 3,
            _ => unreachable!(),
        }
        let count = f.count("operations");
        assert!(f.store.admit_transition_command(&key, command).is_err());
        assert_eq!(f.count("operations"), count);
        assert_eq!(f.count("native_consultation_returns"), 0);
    }
}
#[test]
fn native_return_cancelled_before_submission_keeps_private_floor_and_current_authority_on_replay() {
    let (f, command, key, _) = fixture();
    f.store
        .admit_transition_command(&key, command.clone())
        .unwrap();
    let cancelled = f
        .store
        .execute_operation_action(
            &key,
            OperationAction::Cancel {
                operation_id: "cancel-return".into(),
                target_operation_id: command.operation_id.clone(),
                reason: None,
            },
        )
        .unwrap();
    assert_eq!(cancelled.state, OperationState::Completed);
    let status = f
        .store
        .execute_operation_action(
            &key,
            OperationAction::Get {
                operation_id: command.operation_id.clone(),
            },
        )
        .unwrap();
    assert_eq!(status.state, OperationState::Cancelled);
    assert!(status.effects.is_empty());
    let c = f.store.connection().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM native_private_content_floors WHERE is_candidate=0",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    drop(c);
    let operator = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&operator, &key).unwrap();
    assert!(f.store.admit_transition_command(&key, command).is_err());
}
#[test]
fn native_return_known_original_remote_departure_is_admitted_as_intent_without_claiming_a_live_call(
) {
    let (f, command, key, _) = fixture();
    let remote = command.intent.retained();
    {
        let c = f.store.connection().unwrap();
        c.execute(
            "UPDATE native_connection_lifetimes SET retired=1 WHERE connection_id=?1",
            [remote],
        )
        .unwrap();
        c.execute(
            "UPDATE connections SET state='retired',generation=generation+1 WHERE id=?1",
            [remote],
        )
        .unwrap();
    }
    let accepted = f.store.admit_transition_command(&key, command).unwrap();
    assert_eq!(accepted.state, OperationState::Accepted);
    assert!(accepted.effects.is_empty());
    assert!(accepted.result.is_none());
}

#[test]
fn native_return_primary_departure_requires_original_retirement_observation_before_reservation() {
    for observed in [false, true] {
        let (f, command, key, _) = fixture();
        let primary = command.connection_id.clone().unwrap();
        let c = f.store.connection().unwrap();
        c.execute(
            "UPDATE connections SET state='retired',generation=generation+1 WHERE id=?1",
            [&primary],
        )
        .unwrap();
        if observed {
            c.execute(
                "UPDATE native_connection_lifetimes SET retired=1 WHERE connection_id=?1",
                [&primary],
            )
            .unwrap();
        }
        drop(c);
        let before = f.count("operations");
        let result = f.store.admit_transition_command(&key, command);
        if observed {
            let receipt = result.unwrap();
            assert_eq!(receipt.state, OperationState::Accepted);
            assert!(receipt.effects.is_empty() && receipt.result.is_none());
        } else {
            assert_eq!(result.unwrap_err().code, "not-ready");
            assert_eq!(f.count("operations"), before);
        }
    }
}

fn return_plan(
    f: &Fixture,
    origin: &crate::NativeConsultationReturnOrigin,
    sources: &[Arc<NativeMediaOrigin>],
) -> Value {
    let c = f.store.connection().unwrap();
    let context:Value=c.query_row("SELECT context_selection_json FROM native_consultation_returns WHERE operation_record_id=?1",[origin.record_id()],|r|r.get::<_,String>(0)).map(|s|serde_json::from_str(&s).unwrap()).unwrap();
    let revisions=sources.iter().map(|s|{
        let revision:u64=c.query_row("SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1",[s.connection_id()],|r|r.get(0)).unwrap();
        (s.runtime_connection_id().to_owned(),json!(revision))
    }).collect::<serde_json::Map<_,_>>();
    json!({"operation_id":origin.record_id(),"consultation_record_id":origin.consultation_record_id(),"restore":true,"known_departed_connections":[],"audience_revision":1,"private_connections":context["private"].as_array().unwrap().iter().map(|s|s["runtime_connection_id"].clone()).collect::<Vec<_>>(),"activate_muted":null,"receivers":{},"expected_receiver_revisions":revisions})
}
fn prepared(
    f: &Fixture,
    command: TransitionCommand,
    key: &str,
    sources: &[Arc<NativeMediaOrigin>],
) -> (
    crate::NativeConsultationReturnLease,
    crate::NativeConsultationReturnOrigin,
) {
    f.store
        .admit_transition_command(key, command.clone())
        .unwrap();
    let origin = f
        .store
        .native_consultation_return_origin(key, command)
        .unwrap();
    let plan = return_plan(f, &origin, sources);
    let read = f
        .store
        .try_native_return_read(&origin, sources, &[])
        .unwrap();
    // Only the purpose-bound restore view can reopen the original public pair.
    assert!(read
        .media(sources[1].connection_id(), Some(sources[0].connection_id()))
        .is_some());
    assert!(read
        .media(sources[1].connection_id(), Some(sources[3].connection_id()))
        .is_none());
    assert!(read
        .media(sources[3].connection_id(), Some(sources[0].connection_id()))
        .is_none());
    let lease = f
        .store
        .prepare_native_consultation_return(read, plan)
        .unwrap()
        .unwrap();
    (lease, origin)
}
fn observation(plan: &Value) -> Value {
    let revisions = plan["expected_receiver_revisions"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(id, n)| (id.clone(), json!(n.as_u64().unwrap() + 1)))
        .collect::<serde_json::Map<_, _>>();
    json!({"operation_id":plan["operation_id"],"audience_revision":2,"receiver_revisions":revisions,"restored":true,"departed_connections":[],"participant_handoff":null,"participant_replacement":null})
}
#[test]
fn native_return_reservation_refuses_contended_sql_or_authority_without_effect() {
    let (f, command, key, _, sources) = fixture_full();
    f.store
        .admit_transition_command(&key, command.clone())
        .unwrap();
    let origin = f
        .store
        .native_consultation_return_origin(&key, command)
        .unwrap();
    let plan = return_plan(&f, &origin, &sources);
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let effects_before = f.count("operation_effects");
    let input = || {
        f.store
            .try_native_return_read(&origin, &sources, &[])
            .unwrap()
    };

    let writer = rusqlite::Connection::open(f.store.path()).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    let read = input();
    let started = std::time::Instant::now();
    let refusal = f
        .store
        .prepare_native_consultation_return(read, plan.clone())
        .unwrap_err();
    assert!(started.elapsed() < std::time::Duration::from_millis(500));
    assert_eq!(refusal.code, "authority-busy");
    drop(f.store.try_connection().unwrap());
    writer.execute_batch("ROLLBACK").unwrap();
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);

    let identity = f.store.revalidate(&key).unwrap().identity_id;
    let reader = f
        .store
        .try_acquire_authority(&[crate::AuthorityScope::identity(
            &identity,
            crate::AuthorityMode::Read,
        )
        .unwrap()])
        .unwrap();
    let read = input();
    let started = std::time::Instant::now();
    let refusal = f
        .store
        .prepare_native_consultation_return(read, plan.clone())
        .unwrap_err();
    assert!(started.elapsed() < std::time::Duration::from_millis(500));
    assert_eq!(refusal.code, "authority-busy");
    drop(f.store.try_connection().unwrap());
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    drop(reader);

    // Failed metadata admission must preserve the actual private audience.
    assert!(f
        .store
        .try_native_media_read(&sources[0], Some(&sources[3]))
        .is_ok());
    assert!(f
        .store
        .try_native_media_read(&sources[1], Some(&sources[0]))
        .is_err());
    let lease = f
        .store
        .prepare_native_consultation_return(input(), plan.clone())
        .unwrap()
        .unwrap();
    assert_eq!(lease.plan().unwrap(), plan);
    assert_eq!(f.count("operation_effects"), effects_before + 1);
}
#[test]
fn native_return_single_attempt_and_captured_recording_lifts_only_primary_floor_after_revocation() {
    let (f, command, key, parent, sources) = fixture_full();
    let (lease, origin) = prepared(&f, command.clone(), &key, &sources);
    let plan = lease.plan().unwrap();
    let duplicate = f
        .store
        .prepare_native_consultation_return(
            f.store
                .try_native_return_read(&origin, &sources, &[])
                .unwrap(),
            plan.clone(),
        )
        .unwrap()
        .unwrap();
    let read = f
        .store
        .native_consultation_return_attempt_read(&lease)
        .unwrap();
    let (attempt, read) = f
        .store
        .begin_native_consultation_return(lease, read)
        .unwrap();
    drop(read);
    assert!(f
        .store
        .begin_native_consultation_return(
            duplicate,
            f.store
                .try_native_return_read(&origin, &sources, &[])
                .unwrap()
        )
        .is_err());
    assert!(f
        .store
        .try_native_media_read(&sources[1], Some(&sources[0]))
        .is_err());
    let receipt = attempt.applied(observation(&plan)).unwrap();
    let admin = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&admin, &key).unwrap();
    assert!(f
        .store
        .native_consultation_return_origin(&key, command)
        .is_err());
    let recorded = f
        .store
        .record_native_consultation_return_applied(&receipt)
        .unwrap();
    assert_eq!(recorded.effects.len(), 1);
    assert_eq!(
        f.store
            .record_native_consultation_return_applied(&receipt)
            .unwrap(),
        recorded
    );
    let c = f.store.connection().unwrap();
    assert_eq!(c.query_row("SELECT COUNT(*) FROM native_private_content_floors WHERE operation_record_id=?1 AND is_candidate=0",[&parent],|r|r.get::<_,u64>(0)).unwrap(),0);
    assert_eq!(c.query_row("SELECT COUNT(*) FROM native_private_content_floors WHERE operation_record_id=?1 AND is_candidate=1",[&parent],|r|r.get::<_,u64>(0)).unwrap(),1);
    assert!(c
        .query_row(
            "SELECT restored_at FROM native_private_lifetimes WHERE operation_record_id=?1",
            [&parent],
            |r| r.get::<_, Option<i64>>(0)
        )
        .unwrap()
        .is_some());
    drop(c);
    assert!(f
        .store
        .try_native_media_read(&sources[1], Some(&sources[0]))
        .is_ok());
    assert!(f
        .store
        .try_native_media_read(&sources[1], Some(&sources[3]))
        .is_err());
    assert!(f
        .store
        .complete_native_consultation_return(&receipt)
        .unwrap()
        .is_none());
    {
        let c = f.store.connection().unwrap();
        c.execute(
            "UPDATE native_connection_lifetimes SET retired=1 WHERE connection_id=?1",
            [sources[3].connection_id()],
        )
        .unwrap();
        c.execute(
            "UPDATE connections SET state='retired',generation=generation+1 WHERE id=?1",
            [sources[3].connection_id()],
        )
        .unwrap();
    }
    let completed = f
        .store
        .complete_native_consultation_return(&receipt)
        .unwrap()
        .unwrap();
    assert_eq!(completed.state, OperationState::Completed);
    assert_eq!(
        completed.result.as_ref().unwrap()["remote_continuing"],
        true
    );
    assert_eq!(
        completed.result.as_ref().unwrap()["retired_connection_ids"],
        json!([sources[3].connection_id()])
    );
    assert_eq!(
        f.store
            .complete_native_consultation_return(&receipt)
            .unwrap()
            .unwrap(),
        completed
    );
}
#[test]
fn native_return_uncertainty_and_cancel_never_lift_private_floor_or_allow_second_attempt() {
    let (f, command, key, parent, sources) = fixture_full();
    let (lease, origin) = prepared(&f, command.clone(), &key, &sources);
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
    f.store
        .native_consultation_return_uncertain(attempt)
        .unwrap();
    let replay = f.store.admit_transition_command(&key, command).unwrap();
    assert_eq!(replay.state, OperationState::Unknown);
    assert!(f
        .store
        .try_native_return_read(&origin, &sources, &[])
        .is_err());
    let _ = plan;
    let c = f.store.connection().unwrap();
    assert_eq!(c.query_row("SELECT COUNT(*) FROM native_private_content_floors WHERE operation_record_id=?1 AND is_candidate=0",[parent],|r|r.get::<_,u64>(0)).unwrap(),1);
}
#[test]
fn native_return_duplicate_private_ports_and_changed_plan_revisions_refuse_before_effect() {
    for malformed in ["duplicate", "revision", "context", "candidate-public"] {
        let (f, command, key, _, sources) = fixture_full();
        f.store
            .admit_transition_command(&key, command.clone())
            .unwrap();
        let origin = f
            .store
            .native_consultation_return_origin(&key, command)
            .unwrap();
        let mut plan = return_plan(&f, &origin, &sources);
        match malformed {
            "duplicate" => {
                let duplicate = plan["private_connections"][0].clone();
                plan["private_connections"]
                    .as_array_mut()
                    .unwrap()
                    .push(duplicate);
            }
            "revision" => {
                plan["expected_receiver_revisions"][sources[0].runtime_connection_id()] =
                    json!(9007199254740991u64)
            }
            "context" => plan["consultation_record_id"] = json!("other-root"),
            "candidate-public" => {
                plan["receivers"] =
                    json!({sources[1].runtime_connection_id():[sources[3].runtime_connection_id()]})
            }
            _ => unreachable!(),
        }
        let count = f.count("operation_effects");
        assert!(f
            .store
            .prepare_native_consultation_return(
                f.store
                    .try_native_return_read(&origin, &sources, &[])
                    .unwrap(),
                plan
            )
            .is_err());
        assert_eq!(f.count("operation_effects"), count);
    }
}

#[test]
fn native_return_original_remote_departure_after_admission_uses_only_surviving_physical_authority()
{
    return_after_source_departure(1);
}
#[test]
fn native_return_original_primary_departure_after_admission_uses_only_surviving_physical_authority()
{
    return_after_source_departure(0);
}
fn return_after_source_departure(index: usize) {
    let (f, command, key, parent, mut sources) = fixture_full();
    f.store
        .admit_transition_command(&key, command.clone())
        .unwrap();
    let remote = sources.remove(index);
    {
        let c = f.store.connection().unwrap();
        c.execute(
            "UPDATE native_connection_lifetimes SET retired=1 WHERE connection_id=?1",
            [remote.connection_id()],
        )
        .unwrap();
        c.execute(
            "UPDATE connections SET state='retired',generation=generation+1 WHERE id=?1",
            [remote.connection_id()],
        )
        .unwrap();
    }
    let origin = f
        .store
        .native_consultation_return_origin(&key, command)
        .unwrap();
    let mut plan = return_plan(&f, &origin, &sources);
    plan["known_departed_connections"] = json!([remote.runtime_connection_id()]);
    let read = f
        .store
        .try_native_return_read(&origin, &sources, &[])
        .unwrap();
    assert!(read.media(remote.connection_id(), None).is_none());
    let lease = f
        .store
        .prepare_native_consultation_return(read, plan.clone())
        .unwrap()
        .unwrap();
    let read = f
        .store
        .native_consultation_return_attempt_read(&lease)
        .unwrap();
    let (attempt, read) = f
        .store
        .begin_native_consultation_return(lease, read)
        .unwrap();
    drop(read);
    let mut applied = observation(&plan);
    applied["departed_connections"] = plan["known_departed_connections"].clone();
    let receipt = attempt.applied(applied).unwrap();
    f.store
        .record_native_consultation_return_applied(&receipt)
        .unwrap();
    let candidate = sources.last().unwrap();
    {
        let c = f.store.connection().unwrap();
        c.execute(
            "UPDATE native_connection_lifetimes SET retired=1 WHERE connection_id=?1",
            [candidate.connection_id()],
        )
        .unwrap();
        c.execute(
            "UPDATE connections SET state='retired',generation=generation+1 WHERE id=?1",
            [candidate.connection_id()],
        )
        .unwrap();
    }
    let completed = f
        .store
        .complete_native_consultation_return(&receipt)
        .unwrap()
        .unwrap();
    let result = completed.result.unwrap();
    assert_eq!(result["remote_continuing"], false);
    assert!(result["retired_connection_ids"]
        .as_array()
        .unwrap()
        .iter()
        .any(|id| id == remote.connection_id()));
    assert!(!result["retained_connection_ids"]
        .as_array()
        .unwrap()
        .iter()
        .any(|id| id == remote.connection_id()));
    let c = f.store.connection().unwrap();
    assert_eq!(c.query_row("SELECT COUNT(*) FROM native_private_content_floors WHERE operation_record_id=?1 AND is_candidate=0",[parent],|r|r.get::<_,u64>(0)).unwrap(),0);
}

#[test]
fn native_return_restart_preserves_private_floor_and_never_reuses_predecessor_restore_custody() {
    use connect_protocol::transitions::EffectState;
    for stage in ["admitted", "prepared", "started", "captured"] {
        let (f, command, key, parent, sources) = fixture_full();
        let (old_lease, receipt) = if stage == "admitted" {
            f.store
                .admit_transition_command(&key, command.clone())
                .unwrap();
            (None, None)
        } else {
            let (lease, _) = prepared(&f, command.clone(), &key, &sources);
            if stage == "prepared" {
                (Some(lease), None)
            } else {
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
                // Controlled captured metadata, not an actual Core application.
                let receipt = if stage == "captured" {
                    Some(attempt.applied(observation(&plan)).unwrap())
                } else {
                    drop(attempt);
                    None
                };
                (None, receipt)
            }
        };
        let path = f.store.path().to_path_buf();
        let Fixture { _dir, store, .. } = f;
        drop(store);
        let successor = Store::open(&path, Limits::default()).unwrap();
        let state = successor
            .execute_operation_action(
                &key,
                OperationAction::Get {
                    operation_id: command.operation_id.clone(),
                },
            )
            .unwrap();
        let possible = matches!(stage, "started" | "captured");
        assert_eq!(
            state.state,
            if possible {
                OperationState::Unknown
            } else {
                OperationState::Failed
            },
            "{stage}"
        );
        assert_eq!(state.effects.len(), usize::from(stage != "admitted"));
        for effect in &state.effects {
            assert_eq!(effect.attempt, u64::from(possible));
            assert_eq!(
                effect.state,
                if possible {
                    EffectState::Unknown
                } else {
                    EffectState::Cancelled
                }
            );
        }
        if let Some(lease) = old_lease {
            assert!(successor
                .native_consultation_return_attempt_read(&lease)
                .is_err());
        }
        if let Some(receipt) = receipt {
            assert!(successor
                .record_native_consultation_return_applied(&receipt)
                .is_err());
        }
        assert!(successor
            .native_consultation_return_origin(&key, command)
            .is_err());
        let c = successor.connection().unwrap();
        let floors: u64 = c
            .query_row(
                "SELECT COUNT(*) FROM native_private_content_floors WHERE operation_record_id=?1",
                [&parent],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(floors, 2, "{stage}: restart cannot invent restoration");
        let observed: bool = c.query_row("SELECT applied_json IS NOT NULL FROM native_consultation_returns WHERE consultation_record_id=?1", [&parent], |r|r.get(0)).unwrap();
        assert!(!observed);
        let live: u64 = c
            .query_row(
                "SELECT COUNT(*) FROM native_connection_lifetimes WHERE runtime_epoch=?1 AND retired=0",
                [successor.runtime_epoch()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            live, 0,
            "predecessor native resources cannot survive restart"
        );
        drop(c);
        for source in &sources {
            assert!(successor.try_native_media_read(source, None).is_err());
        }
        drop(successor);
        drop(_dir);
    }
}
