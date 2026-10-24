//! Controlled native plans/observations exercise custody; no Core media is run.
fn fixture() -> (
    Fixture,
    Vec<Arc<NativeMediaOrigin>>,
    connect_protocol::transitions::TransitionCommand,
    NativeConsultationOrigin,
) {
    let (f, mut sources, command, origin) = super::consultation_reads::fixture();
    // Current custody requires a freshly registered, muted private physical leg.
    // Retire the controlled old public slot instead of adopting it as private.
    {
        let c = f.store.connection().unwrap();
        c.execute(
            "UPDATE native_connection_lifetimes SET retired=1 WHERE connection_id=?1",
            [sources[2].connection_id()],
        )
        .unwrap();
        c.execute(
            "UPDATE connections SET state='retired',generation=generation+1 WHERE id=?1",
            [sources[2].connection_id()],
        )
        .unwrap();
    }
    sources[2] = super::consultation_audiences::candidate(&f, &origin);
    (f, sources, command, origin)
}
use super::*;
use crate::{
    NativeConsultationApplied, NativeConsultationAttempt, NativeConsultationLease,
    NativeConsultationOrigin, NativeMediaOrigin,
};
use connect_protocol::transitions::{EffectState, OperationAction, OperationState};
use std::sync::Arc;
fn plan(sources: &[Arc<NativeMediaOrigin>], origin: &NativeConsultationOrigin) -> Value {
    let (p, r, c) = (
        sources[0].runtime_connection_id(),
        sources[1].runtime_connection_id(),
        sources[2].runtime_connection_id(),
    );
    json!({"operation_id":origin.record_id(),"audience_revision":0,"private_connections":[p,c],"receivers":{p:[c],c:[p],r:[]},"activate_muted":c,"expected_receiver_revisions":{p:1,r:1,c:2},"future":{"preserved":true}})
}
fn observed(plan: &Value) -> Value {
    let revisions = plan["expected_receiver_revisions"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(id, v)| (id.clone(), json!(v.as_u64().unwrap() + 1)))
        .collect::<serde_json::Map<_, _>>();
    json!({"operation_id":plan["operation_id"],"audience_revision":1,"receiver_revisions":revisions,"restored":false,"departed_connections":[],"participant_handoff":null,"participant_replacement":null})
}
fn prepare(
    f: &Fixture,
    sources: &[Arc<NativeMediaOrigin>],
    origin: &NativeConsultationOrigin,
) -> NativeConsultationLease {
    let read = f
        .store
        .try_native_consultation_read(
            origin,
            &sources[1],
            &[sources[0].clone(), sources[2].clone()],
        )
        .unwrap();
    f.store
        .prepare_native_consultation(read, plan(sources, origin))
        .unwrap()
        .unwrap()
}
fn begin(f: &Fixture, lease: NativeConsultationLease) -> NativeConsultationAttempt {
    let read = f.store.native_consultation_attempt_read(&lease).unwrap();
    let (attempt, read) = f.store.begin_native_consultation(lease, read).unwrap();
    assert!(read.current());
    drop(read);
    attempt
}
fn capture(
    f: &Fixture,
    sources: &[Arc<NativeMediaOrigin>],
    origin: &NativeConsultationOrigin,
) -> NativeConsultationApplied {
    begin(f, prepare(f, sources, origin))
        .applied(observed(&plan(sources, origin)))
        .unwrap()
}
fn status(f: &Fixture) -> connect_protocol::transitions::OperationRecord {
    f.store
        .execute_operation_action(
            &f.keys[0],
            OperationAction::Get {
                operation_id: "native-consultation".into(),
            },
        )
        .unwrap()
}

#[test]
fn stopped_native_preparation_releases_only_primary_floor_and_never_authorizes_a_retry() {
    let (f, sources, _, origin) = fixture();
    let lease = prepare(&f, &sources, &origin);
    let proof = lease.preparation();
    assert!(
        f.store
            .try_native_media_read(&sources[0], None)
            .unwrap()
            .target()
            .publication_suspended
    );
    assert!(f
        .store
        .try_native_media_read(&sources[0], Some(&sources[1]))
        .is_err());
    assert!(f
        .store
        .settle_native_consultation_preparation(&proof)
        .unwrap());
    assert!(!f
        .store
        .settle_native_consultation_preparation(&proof)
        .unwrap());
    let result = status(&f);
    assert_eq!(result.state, OperationState::Failed);
    assert_eq!(result.effects[0].state, EffectState::Cancelled);
    assert!(f.store.native_consultation_attempt_read(&lease).is_err());
    let c = f.store.connection().unwrap();
    assert!(
        crate::connection_control::private_consultation::private_partner(
            &c,
            sources[0].connection_id()
        )
        .unwrap()
        .is_none()
    );
    assert!(
        crate::connection_control::private_consultation::private_partner(
            &c,
            sources[2].connection_id()
        )
        .unwrap()
        .is_some()
    );
    let facts:(bool,u64)=c.query_row("SELECT n.started,e.attempt FROM native_consultation_attempts n JOIN operation_effects e ON e.id=n.effect_id WHERE n.operation_record_id=?1",[origin.record_id()],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(facts, (false, 0));
    drop(c);
    assert!(
        !f.store
            .try_native_media_read(&sources[0], Some(&sources[1]))
            .unwrap()
            .target()
            .publication_suspended
    );
    assert!(f
        .store
        .try_native_media_read(&sources[2], Some(&sources[1]))
        .is_err());
}

#[test]
fn preparation_settlement_receipt_cannot_cancel_a_possible_native_submission() {
    let (f, sources, _, origin) = fixture();
    let lease = prepare(&f, &sources, &origin);
    let proof = lease.preparation();
    let attempt = begin(&f, lease);
    let before = status(&f);
    assert!(!f
        .store
        .settle_native_consultation_preparation(&proof)
        .unwrap());
    assert_eq!(status(&f), before);
    assert!(f
        .store
        .try_native_media_read(&sources[0], Some(&sources[1]))
        .is_err());
    f.store.native_consultation_uncertain(attempt).unwrap();
    assert!(!f
        .store
        .settle_native_consultation_preparation(&proof)
        .unwrap());
    assert_eq!(status(&f).state, OperationState::Unknown);
}

#[test]
fn unstarted_settlement_keeps_no_execution_grant_and_refuses_another_store() {
    let (f, sources, command, _) = fixture();
    let owner = f.store.revalidate(&f.keys[0]).unwrap();
    let key = f
        .store
        .issue(&owner.identity_id, owner.scope, crate::now() + 600)
        .unwrap()
        .id;
    let origin = f.store.native_consultation_origin(&key, command).unwrap();
    let lease = prepare(&f, &sources, &origin);
    let proof = lease.preparation();
    let (other, _, _, _) = fixture();
    assert!(other
        .store
        .settle_native_consultation_preparation(&proof)
        .is_err());
    // The execution credential is independent of every physical media key.
    f.store
        .connection()
        .unwrap()
        .execute("UPDATE credentials SET revoked=1 WHERE id=?1", [key])
        .unwrap();
    assert!(f.store.native_consultation_attempt_read(&lease).is_err());
    assert!(f
        .store
        .settle_native_consultation_preparation(&proof)
        .unwrap());
    assert_eq!(status(&f).state, OperationState::Failed);
    assert!(f
        .store
        .try_native_media_read(&sources[0], Some(&sources[1]))
        .is_ok());
}

#[test]
fn unstarted_settlement_rolls_back_recording_failure_and_retries_only_metadata() {
    let (f, sources, _, origin) = fixture();
    let lease = prepare(&f, &sources, &origin);
    let proof = lease.preparation();
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER refuse_stopped_preparation BEFORE UPDATE OF state ON operations WHEN NEW.state='failed' BEGIN SELECT RAISE(ABORT,'controlled recording outage'); END;").unwrap();
    assert!(f
        .store
        .settle_native_consultation_preparation(&proof)
        .is_err());
    let before = status(&f);
    assert_eq!(before.state, OperationState::Preparing);
    assert_eq!(before.effects[0].state, EffectState::Prepared);
    assert!(
        f.store
            .try_native_media_read(&sources[0], None)
            .unwrap()
            .target()
            .publication_suspended
    );
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER refuse_stopped_preparation;")
        .unwrap();
    assert!(f
        .store
        .settle_native_consultation_preparation(&proof)
        .unwrap());
    assert_eq!(status(&f).state, OperationState::Failed);
    assert!(f.store.native_consultation_attempt_read(&lease).is_err());
}

#[test]
fn stopped_preparation_floor_change_waits_for_existing_packet_authority() {
    let (f, sources, _, origin) = fixture();
    let lease = prepare(&f, &sources, &origin);
    let proof = lease.preparation();
    let held = f.store.try_native_media_read(&sources[0], None).unwrap();
    assert!(held.target().publication_suspended);
    let f = Arc::new(f);
    let worker = f.clone();
    let (entered, ready) = std::sync::mpsc::channel();
    let (finished, result) = std::sync::mpsc::channel();
    let job = std::thread::spawn(move || {
        entered.send(()).unwrap();
        finished
            .send(worker.store.settle_native_consultation_preparation(&proof))
            .unwrap();
    });
    ready
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    assert!(matches!(
        result.recv_timeout(std::time::Duration::from_millis(100)),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    ));
    assert!(held.current());
    assert!(
        crate::connection_control::private_consultation::private_partner(
            &f.store.connection().unwrap(),
            sources[0].connection_id()
        )
        .unwrap()
        .is_some()
    );
    drop(held);
    assert!(result
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap()
        .unwrap());
    job.join().unwrap();
    assert_eq!(status(&f).state, OperationState::Failed);
}
#[test]
fn native_consultation_exact_plan_precedes_one_attempt_without_broad_worker_permissions() {
    let (f, sources, _, origin) = fixture();
    assert!(!f
        .store
        .revalidate(&f.keys[0])
        .unwrap()
        .permits("operation.execute"));
    let lease = prepare(&f, &sources, &origin);
    let replay = prepare(&f, &sources, &origin);
    assert_eq!(lease.plan().unwrap(), plan(&sources, &origin));
    let prepared = status(&f);
    assert_eq!(prepared.state, OperationState::Preparing);
    assert_eq!(prepared.effects.len(), 1);
    assert_eq!(prepared.effects[0].attempt, 0);
    assert_eq!(prepared.effects[0].correlation_key, origin.record_id());
    let read = f
        .store
        .try_native_consultation_read(
            &origin,
            &sources[1],
            &[sources[0].clone(), sources[2].clone()],
        )
        .unwrap();
    let mut changed = plan(&sources, &origin);
    changed["audience_revision"] = json!(2);
    assert_eq!(
        f.store
            .prepare_native_consultation(read, changed)
            .unwrap_err()
            .code,
        "conflict"
    );
    let attempt = begin(&f, lease);
    let read = f.store.native_consultation_attempt_read(&replay).unwrap();
    assert_eq!(
        f.store
            .begin_native_consultation(replay, read)
            .err()
            .unwrap()
            .code,
        "reconciliation-required"
    );
    let pending = status(&f);
    assert_eq!(pending.effects[0].attempt, 1);
    assert_eq!(pending.effects[0].state, EffectState::Submitting);
    assert!(f
        .store
        .claim_transition(&f.admin, "native-consultation", 60)
        .unwrap()
        .is_none());
    assert!(f
        .store
        .observe_transition_effect(
            &f.admin,
            "native-consultation",
            &pending.effects[0].id,
            "native-consultation",
            "fake-native-confirmation",
            1,
            EffectState::Confirmed,
            None
        )
        .is_err());
    let receipt = attempt.applied(observed(&plan(&sources, &origin))).unwrap();
    let applied = f
        .store
        .record_native_consultation_applied(&receipt)
        .unwrap();
    assert_eq!(applied.state, OperationState::Preparing);
    assert!(applied.result.is_none());
    assert_eq!(applied.effects[0].state, EffectState::Confirmed);
    // Applied audience is not consulting audio readiness or overall completion.
    assert_eq!(
        f.store
            .record_native_consultation_applied(&receipt)
            .unwrap(),
        applied
    );
}
#[test]
fn native_consultation_plan_cannot_include_remote_private_audio_or_activate_the_public_primary() {
    for change in [0, 1, 2, 3] {
        let (f, sources, _, origin) = fixture();
        let mut p = plan(&sources, &origin);
        match change {
            0 => {
                p["receivers"][sources[1].runtime_connection_id()] =
                    json!([sources[0].runtime_connection_id()])
            }
            1 => p["activate_muted"] = json!(sources[0].runtime_connection_id()),
            2 => p["private_connections"] = json!([sources[0].runtime_connection_id()]),
            _ => p["expected_receiver_revisions"] = json!({}),
        }
        let read = f
            .store
            .try_native_consultation_read(
                &origin,
                &sources[1],
                &[sources[0].clone(), sources[2].clone()],
            )
            .unwrap();
        assert!(f.store.prepare_native_consultation(read, p).is_err());
        assert!(status(&f).effects.is_empty());
    }
}
#[test]
fn native_consultation_captured_application_recovers_recording_failure_and_late_cancel_without_old_readers(
) {
    let (f, sources, _, origin) = fixture();
    let receipt = capture(&f, &sources, &origin);
    let before = status(&f);
    {
        let c = f.store.connection().unwrap();
        c.execute_batch(&format!("CREATE TRIGGER refuse_consult_record BEFORE INSERT ON journal WHEN NEW.operation_record_id='{}' BEGIN SELECT RAISE(FAIL,'controlled record failure'); END",origin.record_id())).unwrap();
    }
    assert!(f
        .store
        .record_native_consultation_applied(&receipt)
        .is_err());
    assert_eq!(status(&f), before);
    f.store
        .execute(
            &f.keys[0],
            &Operation::SessionControl {
                request: Request {
                    operation_id: "role-during-consult-recording".into(),
                    conversation_id: f.cid.clone(),
                    session_id: "session".into(),
                    by_participant: None,
                    expected_revision: Some(3),
                    action: Action::Role {
                        participant_id: f.pids[0].clone(),
                        role: "observer".into(),
                    },
                },
            },
        )
        .unwrap();
    // The same controlled journal fault deliberately prevents cancellation too.
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER refuse_consult_record")
        .unwrap();
    f.store
        .execute_operation_action(
            &f.keys[0],
            OperationAction::Cancel {
                operation_id: "cancel-native-consult".into(),
                target_operation_id: "native-consultation".into(),
                reason: None,
            },
        )
        .unwrap();
    let cancelled = status(&f);
    assert_eq!(cancelled.state, OperationState::Unknown);
    assert!(cancelled.cancel_requested);
    let result = f
        .store
        .record_native_consultation_applied(&receipt)
        .unwrap();
    assert_eq!(result.state, OperationState::Unknown);
    assert_eq!(result.effects.len(), 1);
    assert_eq!(result.effects[0].attempt, 1);
    assert_eq!(result.effects[0].state, EffectState::Confirmed);
    assert_eq!(
        f.store
            .record_native_consultation_applied(&receipt)
            .unwrap(),
        result
    );
    let c = f.store.connection().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT topology_revision FROM sessions WHERE id='session'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        4
    );
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM connections WHERE state='ready'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        3
    );
}
#[test]
fn native_consultation_restart_distinguishes_unstarted_from_possible_native_application() {
    for started in [false, true] {
        let (f, sources, _, origin) = fixture();
        let lease = prepare(&f, &sources, &origin);
        let receipt = if started {
            Some(
                begin(&f, lease)
                    .applied(observed(&plan(&sources, &origin)))
                    .unwrap(),
            )
        } else {
            drop(lease);
            None
        };
        let path = f.store.path().to_path_buf();
        let key = f.keys[0].clone();
        let Fixture { _dir, store, .. } = f;
        drop(store);
        let successor = Store::open(&path, Limits::default()).unwrap();
        let state = successor
            .execute_operation_action(
                &key,
                OperationAction::Get {
                    operation_id: "native-consultation".into(),
                },
            )
            .unwrap();
        assert_eq!(
            state.state,
            if started {
                OperationState::Unknown
            } else {
                OperationState::Failed
            }
        );
        assert_eq!(state.effects[0].attempt, if started { 1 } else { 0 });
        assert_eq!(
            state.effects[0].state,
            if started {
                EffectState::Unknown
            } else {
                EffectState::Cancelled
            }
        );
        if let Some(receipt) = receipt {
            assert!(successor
                .record_native_consultation_applied(&receipt)
                .is_err());
        }
        drop(successor);
        drop(_dir);
    }
}
#[test]
fn native_consultation_engine_error_is_unknown_and_cannot_be_observed_through_generic_provider_hooks(
) {
    let (f, sources, _, origin) = fixture();
    let attempt = begin(&f, prepare(&f, &sources, &origin));
    f.store.native_consultation_uncertain(attempt).unwrap();
    let state = status(&f);
    assert_eq!(state.state, OperationState::Unknown);
    assert_eq!(state.effects[0].state, EffectState::Unknown);
    assert!(f
        .store
        .claim_transition_cleanup(&f.admin, "native-consultation", 60)
        .is_err());
    assert!(f
        .store
        .inspect_transition_for_cleanup(&f.admin, origin.record_id())
        .is_err());
    assert!(f
        .store
        .record_transition_evidence(
            &f.admin,
            "native-consultation",
            "fake-native-evidence",
            crate::EvidenceSource::LocalState,
            connect_protocol::transitions::ReconcileOutcome::Cancelled,
            None
        )
        .is_err());
}
#[test]
fn native_consultation_application_observation_must_match_every_pinned_receiver_revision() {
    for field in ["operation", "audience", "receiver", "restored"] {
        let (f, sources, _, origin) = fixture();
        let attempt = begin(&f, prepare(&f, &sources, &origin));
        let mut o = observed(&plan(&sources, &origin));
        match field {
            "operation" => o["operation_id"] = json!("another-record"),
            "audience" => o["audience_revision"] = json!(3),
            "receiver" => o["receiver_revisions"][sources[1].runtime_connection_id()] = json!(1),
            _ => o["restored"] = json!(true),
        }
        assert!(attempt.applied(o).is_err());
        assert_eq!(status(&f).effects[0].state, EffectState::Submitting);
    }
}

#[test]
fn native_consultation_scene_fences_outside_routes_without_issuing_private_receive_views() {
    let (f, sources, _, origin) = fixture();
    let outside = super::consultation_reads::ready(&f, 3);
    assert!(f
        .store
        .try_native_consultation_read(
            &origin,
            &sources[1],
            &[sources[0].clone(), sources[2].clone()]
        )
        .is_err());
    let read = f
        .store
        .try_native_consultation_scene_read(
            &origin,
            &crate::ConsultationMediaOrigin::Native(sources[1].clone()),
            &[sources[0].clone(), sources[2].clone()]
                .into_iter()
                .map(crate::ConsultationMediaOrigin::Native)
                .collect::<Vec<_>>(),
            &[outside.clone()],
            &[],
        )
        .unwrap();
    assert!(read.media(outside.connection_id(), None).is_some());
    assert!(read
        .media(outside.connection_id(), Some(sources[0].connection_id()))
        .is_none());
    let mut p = plan(&sources, &origin);
    p["expected_receiver_revisions"][outside.runtime_connection_id()] = json!(1);
    let lease = f
        .store
        .prepare_native_consultation(read, p.clone())
        .unwrap()
        .unwrap();
    let read = f.store.native_consultation_attempt_read(&lease).unwrap();
    let (attempt, read) = f.store.begin_native_consultation(lease, read).unwrap();
    drop(read);
    let receipt = attempt.applied(observed(&p)).unwrap();
    f.store
        .record_native_consultation_applied(&receipt)
        .unwrap();
    let c = f.store.connection().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1",
            [outside.connection_id()],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        2
    );
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM connections WHERE state='ready'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        4
    );
}
#[test]
fn native_consultation_cancel_before_begin_and_changed_receiver_revision_never_issue_an_attempt() {
    for cancel in [true, false] {
        let (f, sources, _, origin) = fixture();
        let lease = prepare(&f, &sources, &origin);
        let read = f.store.native_consultation_attempt_read(&lease).unwrap();
        if cancel {
            f.store
                .execute_operation_action(
                    &f.keys[0],
                    OperationAction::Cancel {
                        operation_id: "cancel-before-native-begin".into(),
                        target_operation_id: "native-consultation".into(),
                        reason: None,
                    },
                )
                .unwrap();
        } else {
            f.store.connection().unwrap().execute("UPDATE connections SET readiness_json=json_set(readiness_json,'$.stream_revision',3) WHERE id=?1",[sources[2].connection_id()]).unwrap();
        }
        assert!(f.store.begin_native_consultation(lease, read).is_err());
        let state = status(&f);
        assert_eq!(state.effects[0].attempt, 0);
        assert_eq!(
            state.effects[0].state,
            if cancel {
                EffectState::Cancelled
            } else {
                EffectState::Prepared
            }
        );
        let c = f.store.connection().unwrap();
        assert_eq!(
            c.query_row(
                "SELECT started FROM native_consultation_attempts WHERE operation_record_id=?1",
                [origin.record_id()],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            0
        );
    }
}
#[test]
fn native_consultation_historical_application_uses_original_epoch_and_never_revives_removed_routes()
{
    use connect_protocol::membership::{MembershipAction, MembershipOutcome, MembershipRequest};
    let (f, sources, _, origin) = fixture();
    let receipt = capture(&f, &sources, &origin);
    let manager = f
        .store
        .issue(
            &f.store.revalidate(&f.keys[1]).unwrap().identity_id,
            Scope::operator(),
            crate::now() + 600,
        )
        .unwrap()
        .id;
    let revision = f
        .store
        .membership_snapshot(&manager, &f.cid)
        .unwrap()
        .revision;
    let MembershipOutcome::Participant { revision, .. } = f
        .store
        .execute_membership(
            &manager,
            MembershipRequest {
                operation_id: "remove-consult-owner".into(),
                conversation_id: f.cid.clone(),
                expected_revision: revision,
                change: MembershipAction::Remove {
                    participant_id: f.pids[0].clone(),
                    reason: Some("controlled removal after capture".into()),
                },
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let identity = f.store.revalidate(&f.keys[0]).unwrap().identity_id;
    let MembershipOutcome::Participant { participant, .. } = f
        .store
        .execute_membership(
            &manager,
            MembershipRequest {
                operation_id: "readmit-consult-owner".into(),
                conversation_id: f.cid.clone(),
                expected_revision: revision,
                change: MembershipAction::Add {
                    identity_id: identity,
                    kind: IdentityKind::Human,
                    role: "participant".into(),
                },
            },
        )
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(participant.membership_epoch, 2);
    f.store
        .record_native_consultation_applied(&receipt)
        .unwrap();
    assert!(f
        .store
        .execute_operation_action(
            &f.keys[0],
            OperationAction::Get {
                operation_id: "native-consultation".into()
            }
        )
        .is_err());
    let c = f.store.connection().unwrap();
    assert_eq!(c.query_row("SELECT recipient_epoch FROM journal WHERE operation_record_id=?1 ORDER BY sequence DESC LIMIT 1",[origin.record_id()],|r|r.get::<_,u64>(0)).unwrap(),1);
    assert_eq!(
        c.query_row(
            "SELECT state FROM connections WHERE id=?1",
            [sources[0].connection_id()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "retired"
    );
    assert_eq!(
        c.query_row(
            "SELECT generation FROM connections WHERE id=?1",
            [sources[0].connection_id()],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        2
    );
}

#[test]
fn native_consultation_administration_inspection_preserves_live_foreign_attempt_custody() {
    for started in [false, true] {
        let (f, sources, _, origin) = fixture();
        let lease = prepare(&f, &sources, &origin);
        let attempt = if started {
            Some(begin(&f, lease))
        } else {
            drop(lease);
            None
        };
        let before = status(&f);
        let administration = Store::open_administration(f.store.path(), Limits::default()).unwrap();
        let read = administration
            .execute_operation_action(
                &f.keys[0],
                OperationAction::Get {
                    operation_id: "native-consultation".into(),
                },
            )
            .unwrap();
        assert_eq!(read, before);
        assert_eq!(administration.recover_transition_work(100).unwrap(), 0);
        assert_eq!(status(&f), before);
        assert!(administration
            .native_consultation_origin(
                &f.keys[0],
                connect_protocol::transitions::TransitionCommand {
                    payload_extensions: Default::default(),
                    operation_id: "native-consultation".into(),
                    conversation_id: f.cid.clone(),
                    session_id: "session".into(),
                    connection_id: origin.request().connection_id.clone(),
                    expected_revision: 3,
                    intent: origin.request().intent.clone()
                }
            )
            .is_err());
        drop(administration);
        if let Some(attempt) = attempt {
            f.store.native_consultation_uncertain(attempt).unwrap();
        }
    }
}

#[test]
fn native_consultation_decoded_progress_requires_applied_custody_and_records_once_without_completion(
) {
    let (f, sources, _, origin) = fixture();
    let receipt = capture(&f, &sources, &origin);
    assert!(f
        .store
        .record_native_consultation_decoded(&receipt)
        .is_err());
    f.store
        .record_native_consultation_applied(&receipt)
        .unwrap();
    f.store
        .record_native_consultation_decoded(&receipt)
        .unwrap();
    let record = status(&f);
    assert_eq!(record.state, OperationState::Preparing);
    assert!(record.result.is_none());
    assert!(f.store.connection().unwrap().query_row("SELECT decoded_audio_observed FROM native_consultation_attempts WHERE operation_record_id=?1",[origin.record_id()],|r|r.get::<_,bool>(0)).unwrap());
    let before = (f.count("journal"), f.count("operation_effects"));
    f.store
        .record_native_consultation_decoded(&receipt)
        .unwrap();
    assert_eq!(before, (f.count("journal"), f.count("operation_effects")));
    let other = Fixture::new();
    assert!(other
        .store
        .record_native_consultation_decoded(&receipt)
        .is_err());
    assert!(f
        .store
        .connection()
        .unwrap()
        .execute(
            "UPDATE native_consultation_attempts SET decoded_audio_observed=0",
            []
        )
        .is_err());
}

#[test]
fn native_consultation_actual_media_failure_preserves_applied_effect_and_records_original_cause_once(
) {
    let (f, sources, _, origin) = fixture();
    let receipt = capture(&f, &sources, &origin);
    assert!(f
        .store
        .record_native_consultation_interrupted(&receipt, "remote-ended")
        .is_err());
    f.store
        .record_native_consultation_applied(&receipt)
        .unwrap();
    f.store
        .record_native_consultation_interrupted(&receipt, "remote-ended")
        .unwrap();
    let record = status(&f);
    assert_eq!(record.state, OperationState::Unknown);
    assert_eq!(record.effects[0].state, EffectState::Confirmed);
    let before = f.count("journal");
    f.store
        .record_native_consultation_interrupted(&receipt, "remote-ended")
        .unwrap();
    assert_eq!(f.count("journal"), before);
    assert!(f
        .store
        .record_native_consultation_interrupted(&receipt, "timeout")
        .is_err());
    assert!(f
        .store
        .record_native_consultation_interrupted(&receipt, "provider-says-success")
        .is_err());
    let c = f.store.connection().unwrap();
    assert!(c.execute("UPDATE native_consultation_attempts SET media_failure=NULL WHERE operation_record_id=?1",[origin.record_id()]).is_err());
}
