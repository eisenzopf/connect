#![allow(clippy::unwrap_used)]
use super::*;
use crate::Scope;
use connect_protocol::{Limits, Outcome};
use serde_json::json;
mod gateways;
mod packet_observers;
mod signal;
struct Fixture {
    _dir: tempfile::TempDir,
    store: Store,
    ids: Vec<String>,
    keys: Vec<String>,
    parts: Vec<String>,
    request: session_control::Request,
    policy: InitialTelephonePolicy,
}
impl Fixture {
    fn new() -> Self {
        Self::limits(Limits::default())
    }
    fn limits(limits: Limits) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::initialize(&dir.path().join("initial.sqlite3"), limits).unwrap();
        let mut ids = vec![];
        let mut keys = vec![];
        for name in ["caller", "phone", "other"] {
            let identity = store.create_identity(name).unwrap().id;
            keys.push(
                store
                    .issue(&identity, Scope::operator(), crate::now() + 3600)
                    .unwrap()
                    .id,
            );
            ids.push(identity);
        }
        let Outcome::Conversation(room) = store
            .execute(
                &keys[0],
                &Operation::Create {
                    request_id: "room".into(),
                    request: connect_protocol::conversations::CreateRequest::persistent(
                        ids[1..].to_vec(),
                    ),
                },
            )
            .unwrap()
        else {
            panic!()
        };
        let members = store
            .membership_snapshot(&keys[0], &room.id)
            .unwrap()
            .participants;
        let parts: Vec<String> = ids
            .iter()
            .map(|id| {
                members
                    .iter()
                    .find(|p| &p.identity_id == id)
                    .unwrap()
                    .participant_id
                    .clone()
            })
            .collect();
        let request = session_control::Request {
            operation_id: "initial-invitation".into(),
            conversation_id: room.id.clone(),
            session_id: "initial-session".into(),
            by_participant: Some(parts[0].clone()),
            expected_revision: None,
            action: session_control::Action::Invite {
                participants: vec![parts[1].clone()],
                medium: "voice".into(),
                intent: "call".into(),
                capabilities: json!({"audio":["g.711-mu"]}),
            },
        };
        store
            .execute(
                &keys[0],
                &Operation::SessionControl {
                    request: request.clone(),
                },
            )
            .unwrap();
        {
            let c = store.connection().unwrap();
            c.execute("INSERT INTO devices(id,identity_id,tenant_id,kind,platform) VALUES('initial-phone-device',?1,'tnt_default','phone','pstn')",[&ids[1]]).unwrap();
            c.execute("INSERT INTO endpoints(id,identity_id,tenant_id,device_id,transport,address,capabilities_json,verification_state,verification_evidence,verified_at,secret_ref,route_owner_id) VALUES('initial-phone',?1,'tnt_default','initial-phone-device','telephone','+15555550111','[\"audio\"]','verified','fixture',?2,'initial-trunk',?3)",params![ids[1],crate::now(),ids[0]]).unwrap();
        }
        let policy = InitialTelephonePolicy {
            endpoint_id: "initial-phone".into(),
            endpoint_credential_id: keys[1].clone(),
            route_credential_id: keys[0].clone(),
            route_reference: "initial-trunk".into(),
            sip_authority: "127.0.0.1:5099".into(),
            timeout_seconds: 30,
        };
        Self {
            _dir: dir,
            store,
            ids,
            keys,
            parts,
            request,
            policy,
        }
    }
    fn origin(&self) -> InitialTelephoneOrigin {
        self.store
            .prepare_initial_telephone(
                &self.keys[0],
                self.request.clone(),
                &self.parts[1],
                self.policy.clone(),
            )
            .unwrap()
    }
    fn allocate(&self) -> Arc<InitialTelephoneAttempt> {
        Arc::new(
            self.store
                .begin_initial_telephone_allocation(&self.origin())
                .unwrap()
                .unwrap(),
        )
    }
    fn signal(&self) -> InitialTelephoneSignaling {
        let a = self.allocate();
        self.store
            .record_initial_telephone_allocation(&a, "actual-core", "actual-wire@fixture")
            .unwrap();
        self.store
            .begin_initial_telephone_signaling(a)
            .unwrap()
            .unwrap()
    }
    fn cancel(&self, origin: &InitialTelephoneOrigin) -> OperationRecord {
        let decision = self
            .store
            .execute_operation_action(
                &self.keys[0],
                OperationAction::Cancel {
                    operation_id: "cancel-initial".into(),
                    target_operation_id: origin.operation_id().into(),
                    reason: None,
                },
            )
            .unwrap();
        assert_eq!(decision.state, OperationState::Completed);
        self.store
            .execute_operation_action(
                &self.keys[0],
                OperationAction::Get {
                    operation_id: origin.operation_id().into(),
                },
            )
            .unwrap()
    }
    fn connection_state(&self, origin: &InitialTelephoneOrigin) -> String {
        self.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT state FROM connections WHERE id=?1",
                [origin.connection_id()],
                |r| r.get(0),
            )
            .unwrap()
    }
}
#[test]
fn initial_invite_child_replay_preserves_ids_owner_endpoint_and_has_no_io_or_acceptance() {
    let f = Fixture::new();
    let a = f.origin();
    let b = f.origin();
    assert_eq!(a.record_id(), b.record_id());
    assert_eq!(a.connection_id(), b.connection_id());
    assert_eq!(a.source().caller.identity_id, f.ids[0]);
    assert_eq!(a.source().target.identity_id, f.ids[1]);
    assert_eq!(a.source().destination, "sip:+15555550111@127.0.0.1:5099");
    let c = f.store.connection().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM operation_effects WHERE operation_record_id=?1",
            [a.record_id()],
            |r| r.get::<_, u32>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        c.query_row(
            "SELECT state FROM session_participants WHERE session_id=?1 AND participant_id=?2",
            params![f.request.session_id, f.parts[1]],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "invited"
    );
    drop(c);
    assert!(f
        .store
        .claim_transition(&f.keys[0], a.operation_id(), 5)
        .unwrap()
        .is_none());
    let status = f
        .store
        .execute_operation_action(
            &f.keys[0],
            OperationAction::Get {
                operation_id: a.operation_id().into(),
            },
        )
        .unwrap();
    assert_eq!(status.state, OperationState::Accepted);
    assert!(f
        .store
        .execute_operation_action(
            &f.keys[1],
            OperationAction::Get {
                operation_id: a.operation_id().into()
            }
        )
        .is_err());
}
#[test]
fn initial_source_requires_actual_successful_exact_invitation_and_explicit_target() {
    let f = Fixture::new();
    assert!(f
        .store
        .prepare_initial_telephone(&f.keys[0], f.request.clone(), &f.parts[2], f.policy.clone())
        .is_err());
    let mut changed = f.request.clone();
    changed.operation_id = "uncommitted".into();
    assert!(f
        .store
        .prepare_initial_telephone(&f.keys[0], changed, &f.parts[1], f.policy.clone())
        .is_err());
    let mut changed = f.request.clone();
    changed.by_participant = None;
    assert!(f
        .store
        .prepare_initial_telephone(&f.keys[0], changed, &f.parts[1], f.policy.clone())
        .is_err());
    assert!(f
        .store
        .prepare_initial_telephone(&f.keys[2], f.request.clone(), &f.parts[1], f.policy.clone())
        .is_err());
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM initial_telephone_operations",
                [],
                |r| r.get::<_, u32>(0)
            )
            .unwrap(),
        0
    );
}
#[test]
fn initial_selected_policy_cannot_change_intent_or_borrow_another_identity() {
    let f = Fixture::new();
    let original = f.origin();
    let mut changed = f.policy.clone();
    changed.sip_authority = "127.0.0.1:5098".into();
    assert_eq!(
        f.store
            .prepare_initial_telephone(&f.keys[0], f.request.clone(), &f.parts[1], changed)
            .unwrap_err()
            .code,
        "conflict"
    );
    let mut changed = f.policy.clone();
    changed.endpoint_credential_id = f.keys[2].clone();
    assert!(f
        .store
        .prepare_initial_telephone(&f.keys[0], f.request.clone(), &f.parts[1], changed)
        .is_err());
    let mut changed = f.policy.clone();
    changed.route_reference = "another-trunk".into();
    assert!(f
        .store
        .prepare_initial_telephone(&f.keys[0], f.request.clone(), &f.parts[1], changed)
        .is_err());
    assert_eq!(f.origin().connection_id(), original.connection_id());
}
#[test]
fn initial_allocation_and_signaling_are_once_only_with_exact_immutable_dialog_binding() {
    let f = Fixture::new();
    let origin = f.origin();
    let a = Arc::new(
        f.store
            .begin_initial_telephone_allocation(&origin)
            .unwrap()
            .unwrap(),
    );
    assert!(f
        .store
        .begin_initial_telephone_allocation(&origin)
        .unwrap()
        .is_none());
    assert!(f
        .store
        .begin_initial_telephone_signaling(a.clone())
        .is_err());
    let read = f.store.initial_telephone_allocation_read(&a).unwrap();
    assert!(read.current());
    assert!(read.sip_call_id().is_none());
    drop(read);
    f.store
        .record_initial_telephone_allocation(&a, "actual-core", "actual-wire@fixture")
        .unwrap();
    f.store
        .record_initial_telephone_allocation(&a, "actual-core", "actual-wire@fixture")
        .unwrap();
    assert_eq!(
        f.store
            .record_initial_telephone_allocation(&a, "other-core", "actual-wire@fixture")
            .unwrap_err()
            .code,
        "conflict"
    );
    let signal = f
        .store
        .begin_initial_telephone_signaling(a.clone())
        .unwrap()
        .unwrap();
    assert!(f
        .store
        .begin_initial_telephone_signaling(a.clone())
        .unwrap()
        .is_none());
    assert!(f.store.initial_telephone_allocation_read(&a).is_err());
    let read = f.store.initial_telephone_signaling_read(&signal).unwrap();
    assert!(read.current());
    assert_eq!(read.runtime_connection_id(), Some("actual-core"));
    assert_eq!(read.sip_call_id(), Some("actual-wire@fixture"));
    let c = f.store.connection().unwrap();
    let effect: (u32, String) = c
        .query_row(
            "SELECT attempt,state FROM operation_effects WHERE operation_record_id=?1",
            [origin.record_id()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(effect, (1, "submitting".into()));
}
#[test]
fn initial_cancel_before_submission_retires_only_the_dormant_reservation() {
    let f = Fixture::new();
    let a = f.allocate();
    let result = f.cancel(a.origin());
    assert_eq!(result.state, OperationState::Cancelled);
    assert_eq!(f.connection_state(a.origin()), "retired");
    assert!(f.store.initial_telephone_allocation_read(&a).is_err());
    assert!(f
        .store
        .begin_initial_telephone_allocation(a.origin())
        .is_err());
}
#[test]
fn initial_cancel_after_submission_preserves_unknown_quota_and_forbids_generic_completion() {
    let f = Fixture::new();
    let signal = f.signal();
    let result = f.cancel(signal.origin());
    assert_eq!(result.state, OperationState::Unknown);
    assert_eq!(f.connection_state(signal.origin()), "unknown");
    assert!(f.store.initial_telephone_signaling_read(&signal).is_err());
    assert!(f
        .store
        .claim_transition(&f.keys[0], signal.origin().operation_id(), 5)
        .unwrap()
        .is_none());
    assert_eq!(
        f.store
            .execute_operation_action(
                &f.keys[0],
                OperationAction::Reconcile {
                    operation_id: "fake-completion".into(),
                    target_operation_id: signal.origin().operation_id().into(),
                    outcome: ReconcileOutcome::Completed,
                    evidence_id: "admin-says-ready".into()
                }
            )
            .unwrap_err()
            .code,
        "reconciliation-required"
    );
    assert_eq!(
        f.store
            .claim_transition_cleanup(&f.keys[0], signal.origin().operation_id(), 5)
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(
        f.store
            .record_transition_evidence(
                &f.keys[0],
                signal.origin().operation_id(),
                "invented-absence",
                EvidenceSource::ProviderStatus,
                ReconcileOutcome::Failed,
                None
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert!(f
        .store
        .inspect_transition_for_cleanup(&f.keys[0], signal.origin().record_id())
        .is_err());
}
#[test]
fn initial_recovery_without_signaling_can_retire_dormant_but_submission_stays_unknown() {
    for submitted in [false, true] {
        let f = Fixture::new();
        let a = f.allocate();
        if submitted {
            f.store
                .record_initial_telephone_allocation(&a, "actual-core", "actual-wire@fixture")
                .unwrap();
            f.store
                .begin_initial_telephone_signaling(a.clone())
                .unwrap()
                .unwrap();
        }
        let c = f.store.connection().unwrap();
        c.execute(
            "UPDATE operations SET worker_runtime='departed-runtime' WHERE record_id=?1",
            [a.origin().record_id()],
        )
        .unwrap();
        let row = rows::by_record(&c, a.origin().record_id()).unwrap();
        assert!(recover_one(&c, &row, &f.store).unwrap());
        assert_eq!(
            rows::by_record(&c, a.origin().record_id())
                .unwrap()
                .record
                .state,
            if submitted {
                OperationState::Unknown
            } else {
                OperationState::Failed
            }
        );
        drop(c);
        assert_eq!(
            f.connection_state(a.origin()),
            if submitted { "unknown" } else { "retired" }
        );
        assert!(f.store.initial_telephone_allocation_read(&a).is_err());
    }
}
#[test]
fn initial_withdrawal_in_allocation_to_signaling_gap_performs_no_submission() {
    let f = Fixture::new();
    let a = f.allocate();
    f.store
        .record_initial_telephone_allocation(&a, "actual-core", "actual-wire@fixture")
        .unwrap();
    f.store
        .revoke(&f.store.revalidate(&f.keys[0]).unwrap(), &f.keys[1])
        .unwrap();
    assert!(f.store.begin_initial_telephone_signaling(a).is_err());
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM operation_effects WHERE provider='sip-initial'",
                [],
                |r| r.get::<_, u32>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn initial_endpoint_policy_change_cannot_derive_a_second_child_under_the_same_invitation() {
    let f = Fixture::new();
    let original = f.origin();
    {
        let c = f.store.connection().unwrap();
        c.execute("INSERT INTO endpoints(id,identity_id,tenant_id,device_id,transport,address,capabilities_json,verification_state,verification_evidence,verified_at,secret_ref,route_owner_id) VALUES('initial-phone-other',?1,'tnt_default','initial-phone-device','telephone','+15555550112','[\"audio\"]','verified','fixture',?2,'initial-trunk',?3)",params![f.ids[1],crate::now(),f.ids[0]]).unwrap();
    }
    let mut changed = f.policy.clone();
    changed.endpoint_id = "initial-phone-other".into();
    assert_eq!(
        f.store
            .prepare_initial_telephone(&f.keys[0], f.request.clone(), &f.parts[1], changed)
            .unwrap_err()
            .code,
        "conflict"
    );
    let c = f.store.connection().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM initial_telephone_operations",
            [],
            |r| r.get::<_, u32>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        c.query_row(
            "SELECT endpoint_id FROM connections WHERE id=?1",
            [original.connection_id()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "initial-phone"
    );
}
#[test]
fn initial_unknown_attempt_continues_to_consume_capacity_for_a_new_invitation() {
    let mut limits = Limits::default();
    limits.connections = 1;
    let f = Fixture::limits(limits);
    let signal = f.signal();
    f.cancel(signal.origin());
    let mut next = f.request.clone();
    next.operation_id = "next-invite".into();
    next.session_id = "next-session".into();
    f.store
        .execute(
            &f.keys[0],
            &Operation::SessionControl {
                request: next.clone(),
            },
        )
        .unwrap();
    assert_eq!(
        f.store
            .prepare_initial_telephone(&f.keys[0], next, &f.parts[1], f.policy.clone())
            .unwrap_err()
            .code,
        "capacity-exceeded"
    );
    assert_eq!(f.connection_state(signal.origin()), "unknown");
}
#[test]
fn initial_database_dialog_and_once_only_markers_cannot_be_rewritten_or_deleted() {
    let f = Fixture::new();
    let signal = f.signal();
    let c = f.store.connection().unwrap();
    for sql in ["UPDATE initial_telephone_operations SET sip_call_id='forged' WHERE operation_record_id=?1","UPDATE initial_telephone_operations SET signaling_started=0,effect_id=NULL WHERE operation_record_id=?1","UPDATE initial_telephone_operations SET source_json='{}' WHERE operation_record_id=?1","DELETE FROM initial_telephone_operations WHERE operation_record_id=?1"] {
  assert!(c.execute(sql,[signal.origin().record_id()]).is_err());
 }
    let binding:(String,String)=c.query_row("SELECT runtime_connection_id,sip_call_id FROM initial_telephone_operations WHERE operation_record_id=?1",[signal.origin().record_id()],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(
        binding,
        ("actual-core".into(), "actual-wire@fixture".into())
    );
}
#[test]
fn initial_withdrawal_after_the_submission_marker_issues_no_fresh_packet_interval() {
    let f = Fixture::new();
    let signal = f.signal();
    let actor = f.store.revalidate(&f.keys[0]).unwrap();
    f.store.revoke(&actor, &f.keys[1]).unwrap();
    assert!(f.store.initial_telephone_signaling_read(&signal).is_err());
    let c = f.store.connection().unwrap();
    c.execute(
        "UPDATE operations SET worker_runtime='departed-runtime' WHERE record_id=?1",
        [signal.origin().record_id()],
    )
    .unwrap();
    let row = rows::by_record(&c, signal.origin().record_id()).unwrap();
    assert!(recover_one(&c, &row, &f.store).unwrap());
    assert_eq!(
        rows::by_record(&c, signal.origin().record_id())
            .unwrap()
            .record
            .effects[0]
            .state,
        EffectState::Unknown
    );
}

// Store-boundary tests use synthetic observations. Real transport proof is separate.
#[test]
fn answer_accepts_only_own_invited_participant_without_publishing_readiness() {
    let f = Fixture::new();
    let signal = Arc::new(f.signal());
    let capture = signal.capture_answer(signal.sip_call_id(), "pcmu").unwrap();
    let recorded = f.store.record_initial_telephone_answer(&capture).unwrap();
    assert!(recorded.problem.is_none());
    let media = recorded.media.unwrap();
    let read = f.store.try_initial_telephone_media_read(media).unwrap();
    assert!(read.current());
    assert!(!read.is_connected());
    assert_eq!(read.endpoint_actor().identity_id, f.ids[1]);
    assert_eq!(read.member().participant_id, f.parts[1]);
    assert_eq!(f.connection_state(signal.origin()), "preparing");
    drop(read);
    let own = f
        .store
        .execute_operation_action(
            &f.keys[1],
            OperationAction::Get {
                operation_id: format!("answer_{}", signal.origin().record_id()),
            },
        )
        .unwrap();
    assert_eq!(own.state, OperationState::Completed);
    let c = f.store.connection().unwrap();
    let state: String = c
        .query_row(
            "SELECT state FROM session_participants WHERE session_id=?1 AND participant_id=?2",
            params![f.request.session_id, f.parts[1]],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "present");
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM journal WHERE kind='connection_ready'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    drop(c);
    assert!(f
        .store
        .record_initial_telephone_answer(&capture)
        .unwrap()
        .media
        .is_some());
    let changed = signal.capture_answer(signal.sip_call_id(), "pcma").unwrap();
    assert_eq!(
        f.store
            .record_initial_telephone_answer(&changed)
            .unwrap_err()
            .code,
        "conflict"
    );
}
#[test]
fn observed_answer_survives_withdrawn_authority_without_accepting_or_redialing() {
    let f = Fixture::new();
    let signal = Arc::new(f.signal());
    let capture = signal.capture_answer(signal.sip_call_id(), "pcmu").unwrap();
    f.store
        .revoke(&f.store.revalidate(&f.keys[2]).unwrap(), &f.keys[0])
        .unwrap();
    let recorded = f.store.record_initial_telephone_answer(&capture).unwrap();
    assert!(recorded.media.is_none());
    assert!(recorded.problem.is_some());
    let c = f.store.connection().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM initial_telephone_answers WHERE accepted=0",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        c.query_row(
            "SELECT state FROM session_participants WHERE session_id=?1 AND participant_id=?2",
            params![f.request.session_id, f.parts[1]],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "invited"
    );
    assert_eq!(
        c.query_row(
            "SELECT state FROM operation_effects WHERE operation_record_id=?1",
            [signal.origin().record_id()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "confirmed"
    );
    drop(c);
    assert_eq!(f.connection_state(signal.origin()), "unknown");
}
#[test]
fn unsupported_answer_codec_keeps_fact_and_rolls_back_own_acceptance() {
    let f = Fixture::new();
    let signal = Arc::new(f.signal());
    let capture = signal.capture_answer(signal.sip_call_id(), "opus").unwrap();
    assert!(f
        .store
        .record_initial_telephone_answer(&capture)
        .unwrap()
        .media
        .is_none());
    let c = f.store.connection().unwrap();
    assert!(c
        .execute("UPDATE initial_telephone_answers SET accepted=1", [])
        .is_err());
    assert!(c.execute("UPDATE initial_telephone_answers SET accepted=1,acceptance_record_id=(SELECT parent_record_id FROM initial_telephone_operations)",[]).is_err());
    assert_eq!(
        c.query_row(
            "SELECT state FROM session_participants WHERE session_id=?1 AND participant_id=?2",
            params![f.request.session_id, f.parts[1]],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "invited"
    );
    assert_eq!(
        c.query_row("SELECT codec FROM initial_telephone_answers", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        "opus"
    );
}
struct ReadyFixture(InitialTelephoneReadyObservation);
impl InitialTelephoneRuntimeObservation for ReadyFixture {
    fn observe(&self) -> Result<InitialTelephoneReadyObservation> {
        Ok(self.0.clone())
    }
}
struct AbsentFixture(bool);
impl crate::TelephoneConnectionRuntimeObservation for AbsentFixture {
    fn observe_absent(&self, id: &str, generation: u64) -> Result<()> {
        if self.0 && id == "actual-core" && generation == 1 {
            Ok(())
        } else {
            Err(Error::forbidden())
        }
    }
}
#[test]
fn connected_answer_requires_exact_observation_and_cleanup_retries_emit_once() {
    let f = Fixture::new();
    let signal = Arc::new(f.signal());
    let capture = signal.capture_answer(signal.sip_call_id(), "pcmu").unwrap();
    let media = f
        .store
        .record_initial_telephone_answer(&capture)
        .unwrap()
        .media
        .unwrap();
    let mut value = InitialTelephoneReadyObservation {
        runtime_connection_id: "wrong-core".into(),
        call_id: signal.sip_call_id().into(),
        participant_id: f.parts[1].clone(),
        membership_epoch: signal.origin().source().target.membership_epoch,
        generation: 1,
        stream_revision: 1,
        participant_revision: 1,
        streams: vec!["telephone-audio".into()],
    };
    assert!(f
        .store
        .record_initial_telephone_connected(media.clone(), &ReadyFixture(value.clone()))
        .is_err());
    assert_eq!(f.connection_state(signal.origin()), "preparing");
    value.runtime_connection_id = "actual-core".into();
    assert_eq!(
        f.store
            .record_initial_telephone_connected(media.clone(), &ReadyFixture(value.clone()))
            .unwrap()
            .state,
        OperationState::Completed
    );
    assert_eq!(
        f.store
            .record_initial_telephone_connected(media.clone(), &ReadyFixture(value))
            .unwrap()
            .state,
        OperationState::Completed
    );
    assert!(f
        .store
        .try_initial_telephone_media_read(media.clone())
        .unwrap()
        .is_connected());
    assert!(f
        .store
        .settle_initial_telephone_cleanup(&signal, &AbsentFixture(false))
        .is_err());
    assert_eq!(f.connection_state(signal.origin()), "ready");
    f.store
        .settle_initial_telephone_cleanup(&signal, &AbsentFixture(true))
        .unwrap();
    f.store
        .settle_initial_telephone_cleanup(&signal, &AbsentFixture(true))
        .unwrap();
    assert_eq!(f.connection_state(signal.origin()), "retired");
    assert!(f.store.try_initial_telephone_media_read(media).is_err());
    let c = f.store.connection().unwrap();
    let audience: u64 = c
        .query_row("SELECT count(*) FROM initial_telephone_answers", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(audience, 1);
    let ready: u64 = c
        .query_row(
            "SELECT count(*) FROM journal WHERE kind='connection_ready'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let retired: u64 = c
        .query_row(
            "SELECT count(*) FROM journal WHERE kind='connection_retired'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(ready > 0);
    assert_eq!(ready, retired);
}

#[test]
fn connected_phone_uses_own_endpoint_and_route_after_caller_credential_is_revoked() {
    let mut f = Fixture::new();
    f.policy.route_credential_id = f
        .store
        .issue(&f.ids[0], Scope::operator(), crate::now() + 3600)
        .unwrap()
        .id;
    let signal = Arc::new(f.signal());
    let capture = signal.capture_answer(signal.sip_call_id(), "pcmu").unwrap();
    let media = f
        .store
        .record_initial_telephone_answer(&capture)
        .unwrap()
        .media
        .unwrap();
    let ready = ReadyFixture(InitialTelephoneReadyObservation {
        runtime_connection_id: signal.runtime_connection_id().into(),
        call_id: signal.sip_call_id().into(),
        participant_id: f.parts[1].clone(),
        membership_epoch: signal.origin().source().target.membership_epoch,
        generation: 1,
        stream_revision: 1,
        participant_revision: 1,
        streams: vec!["telephone-audio".into()],
    });
    f.store
        .record_initial_telephone_connected(media.clone(), &ready)
        .unwrap();
    f.store
        .revoke(&f.store.revalidate(&f.keys[2]).unwrap(), &f.keys[0])
        .unwrap();
    assert!(f
        .store
        .try_initial_telephone_media_read(media.clone())
        .unwrap()
        .is_connected());
    f.store
        .revoke(&f.store.revalidate(&f.keys[2]).unwrap(), &f.keys[1])
        .unwrap();
    assert!(f.store.try_initial_telephone_media_read(media).is_err());
    f.store
        .settle_initial_telephone_cleanup(&signal, &AbsentFixture(true))
        .unwrap();
    assert_eq!(f.connection_state(signal.origin()), "retired");
}

#[test]
fn initial_receive_projection_is_original_source_scoped_and_metadata_retry_is_exact() {
    let f = Fixture::new();
    let signal = Arc::new(f.signal());
    let capture = signal.capture_answer(signal.sip_call_id(), "pcmu").unwrap();
    let media = f
        .store
        .record_initial_telephone_answer(&capture)
        .unwrap()
        .media
        .unwrap();
    let mut value = InitialTelephoneReadyObservation {
        runtime_connection_id: signal.runtime_connection_id().into(),
        call_id: signal.sip_call_id().into(),
        participant_id: f.parts[1].clone(),
        membership_epoch: signal.origin().source().target.membership_epoch,
        generation: 1,
        stream_revision: 1,
        participant_revision: 1,
        streams: vec!["telephone-audio".into()],
    };
    f.store
        .record_initial_telephone_connected(media.clone(), &ReadyFixture(value.clone()))
        .unwrap();
    value.stream_revision = 2;
    assert!(f
        .store
        .record_initial_telephone_receive_projection(
            media.clone(),
            &f.parts[1],
            1,
            &ReadyFixture(value.clone())
        )
        .is_err());
    assert!(f
        .store
        .record_initial_telephone_receive_projection(
            media.clone(),
            &f.parts[2],
            1,
            &ReadyFixture(value.clone())
        )
        .is_err());
    f.store
        .record_initial_telephone_receive_projection(
            media.clone(),
            &f.parts[0],
            1,
            &ReadyFixture(value.clone()),
        )
        .unwrap();
    let revision: u64 = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT revision FROM operations WHERE record_id=?1",
            [media.record_id()],
            |r| r.get(0),
        )
        .unwrap();
    f.store
        .record_initial_telephone_receive_projection(
            media.clone(),
            &f.parts[0],
            1,
            &ReadyFixture(value.clone()),
        )
        .unwrap();
    value.stream_revision = 3;
    assert_eq!(
        f.store
            .record_initial_telephone_receive_projection(
                media.clone(),
                &f.parts[0],
                2,
                &ReadyFixture(value)
            )
            .unwrap_err()
            .code,
        "conflict"
    );
    let c = f.store.connection().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT revision FROM operations WHERE record_id=?1",
            [media.record_id()],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        revision
    );
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM initial_telephone_receive_projections",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        c.query_row(
            "SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1",
            [media.connection_id()],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        2
    );
    assert!(c
        .execute("DELETE FROM initial_telephone_receive_projections", [])
        .is_err());
    assert!(c
        .execute(
            "UPDATE initial_telephone_receive_projections SET membership_epoch=membership_epoch+1",
            []
        )
        .is_err());
}

#[test]
fn initial_confirmed_cleanup_retires_an_already_fenced_session_end_without_another_generation() {
    let f = Fixture::new();
    let signal = Arc::new(f.signal());
    let capture = signal.capture_answer(signal.sip_call_id(), "pcmu").unwrap();
    let media = f
        .store
        .record_initial_telephone_answer(&capture)
        .unwrap()
        .media
        .unwrap();
    let value = InitialTelephoneReadyObservation {
        runtime_connection_id: signal.runtime_connection_id().into(),
        call_id: signal.sip_call_id().into(),
        participant_id: f.parts[1].clone(),
        membership_epoch: signal.origin().source().target.membership_epoch,
        generation: 1,
        stream_revision: 1,
        participant_revision: 1,
        streams: vec!["telephone-audio".into()],
    };
    f.store
        .record_initial_telephone_connected(media.clone(), &ReadyFixture(value))
        .unwrap();
    let session = f
        .store
        .session_view(
            &f.keys[0],
            &f.request.conversation_id,
            &f.request.session_id,
        )
        .unwrap();
    f.store
        .execute(
            &f.keys[0],
            &Operation::SessionControl {
                request: session_control::Request {
                    operation_id: "initial-end".into(),
                    conversation_id: f.request.conversation_id.clone(),
                    session_id: f.request.session_id.clone(),
                    by_participant: Some(f.parts[0].clone()),
                    expected_revision: Some(session.topology_revision),
                    action: session_control::Action::End {
                        reason_code: 1000,
                        reason: "complete".into(),
                    },
                },
            },
        )
        .unwrap();
    assert_eq!(f.connection_state(signal.origin()), "retiring");
    assert!(f
        .store
        .try_initial_telephone_media_read(media.clone())
        .is_err());
    assert!(f
        .store
        .settle_initial_telephone_cleanup(&signal, &AbsentFixture(false))
        .is_err());
    f.store
        .settle_initial_telephone_cleanup(&signal, &AbsentFixture(true))
        .unwrap();
    f.store
        .settle_initial_telephone_cleanup(&signal, &AbsentFixture(true))
        .unwrap();
    let c = f.store.connection().unwrap();
    let state: (String, u64) = c
        .query_row(
            "SELECT state,generation FROM connections WHERE id=?1",
            [media.connection_id()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, ("retired".into(), 2));
}

#[test]
fn initial_local_absence_preserves_unknown_submission_and_records_separate_cleanup() {
    let f = Fixture::new();
    let signal = Arc::new(f.signal());
    f.store
        .settle_initial_telephone_cleanup(&signal, &AbsentFixture(true))
        .unwrap();
    let c = f.store.connection().unwrap();
    let outcome:String=c.query_row("SELECT state FROM operation_effects WHERE operation_record_id=?1 AND compensates_effect_id IS NULL",[signal.origin().record_id()],|r|r.get(0)).unwrap();
    assert_eq!(outcome, "unknown");
    let cleanup:String=c.query_row("SELECT state FROM operation_effects WHERE operation_record_id=?1 AND compensates_effect_id IS NOT NULL",[signal.origin().record_id()],|r|r.get(0)).unwrap();
    assert_eq!(cleanup, "confirmed");
    drop(c);
    assert_eq!(f.connection_state(signal.origin()), "retired");
    assert!(f
        .store
        .begin_initial_telephone_allocation(signal.origin())
        .is_err());
}

// Controlled runtime witnesses at the Store boundary; physical mixed-transport
// qualification is performed separately by the production listener tests.
fn ready_phone(
    f: &Fixture,
) -> (
    Arc<InitialTelephoneSignaling>,
    Arc<InitialTelephoneMediaOrigin>,
) {
    let signal = Arc::new(f.signal());
    let capture = signal.capture_answer(signal.sip_call_id(), "pcmu").unwrap();
    let media = f
        .store
        .record_initial_telephone_answer(&capture)
        .unwrap()
        .media
        .unwrap();
    f.store
        .record_initial_telephone_connected(
            media.clone(),
            &ReadyFixture(InitialTelephoneReadyObservation {
                runtime_connection_id: signal.runtime_connection_id().into(),
                call_id: signal.sip_call_id().into(),
                participant_id: f.parts[1].clone(),
                membership_epoch: 1,
                generation: 1,
                stream_revision: 1,
                participant_revision: 1,
                streams: vec!["telephone-audio".into()],
            }),
        )
        .unwrap();
    (signal, media)
}
fn creator_leaves(f: &Fixture) {
    let view = f
        .store
        .session_view(
            &f.keys[0],
            &f.request.conversation_id,
            &f.request.session_id,
        )
        .unwrap();
    f.store
        .execute(
            &f.keys[0],
            &Operation::SessionControl {
                request: session_control::Request {
                    operation_id: "unready-creator-leaves".into(),
                    conversation_id: f.request.conversation_id.clone(),
                    session_id: f.request.session_id.clone(),
                    by_participant: Some(f.parts[0].clone()),
                    expected_revision: Some(view.topology_revision),
                    action: session_control::Action::Leave {
                        participant_id: f.parts[0].clone(),
                    },
                },
            },
        )
        .unwrap();
}
fn activation_phase(f: &Fixture) -> String {
    f.store
        .connection()
        .unwrap()
        .query_row(
            "SELECT state FROM sessions WHERE id=?1",
            [&f.request.session_id],
            |r| r.get(0),
        )
        .unwrap()
}
#[test]
fn initial_ready_phone_waits_for_other_accepted_participants_and_rows_cannot_rebuild_origin_custody(
) {
    let f = Fixture::new();
    let (_, media) = ready_phone(&f);
    assert_eq!(activation_phase(&f), "accepted");
    assert_eq!(
        f.store.session_reconnect_page("").unwrap().1,
        vec![(
            f.request.conversation_id.clone(),
            f.request.session_id.clone()
        )]
    );
    f.store.session_ready_origins.lock().unwrap().clear();
    creator_leaves(&f);
    assert!(!f
        .store
        .reconcile_session_reconnect(&f.request.conversation_id, &f.request.session_id)
        .unwrap());
    assert_eq!(
        activation_phase(&f),
        "accepted",
        "committed ready rows cannot manufacture actual origin custody"
    );
    drop(f.store.try_initial_telephone_media_read(media).unwrap());
    assert!(!f
        .store
        .reconcile_session_reconnect(&f.request.conversation_id, &f.request.session_id)
        .unwrap());
    assert_eq!(activation_phase(&f), "active");
    let original = f
        .store
        .session_started_template(&f.request.conversation_id, &f.request.session_id)
        .unwrap()
        .0;
    assert_eq!(original.participants_present, vec![f.parts[1].clone()]);
    assert_eq!(original.active_connections.len(), 1);
    assert_eq!(original.active_connections[0].transport, "sip-interop");
    assert_eq!(
        original.negotiated_capabilities["audio_codecs"][0]["name"],
        "g.711-mu"
    );
    f.store
        .reconcile_session_reconnect(&f.request.conversation_id, &f.request.session_id)
        .unwrap();
    assert_eq!(
        f.store
            .session_started_template(&f.request.conversation_id, &f.request.session_id)
            .unwrap()
            .0,
        original
    );
}
#[test]
fn initial_ready_activation_rechecks_endpoint_route_generation_runtime_and_membership() {
    for mode in [
        "endpoint-credential",
        "route-credential",
        "endpoint",
        "generation",
        "runtime",
        "membership",
    ] {
        let f = Fixture::new();
        let _ready = ready_phone(&f);
        creator_leaves(&f);
        match mode {
            "endpoint-credential" | "route-credential" => {
                let id = if mode == "endpoint-credential" {
                    &f.keys[1]
                } else {
                    &f.keys[0]
                };
                f.store
                    .revoke(&f.store.revalidate(&f.keys[2]).unwrap(), id)
                    .unwrap();
            }
            mode => {
                let c = f.store.connection().unwrap();
                match mode {
                    "endpoint" => {
                        c.execute("UPDATE endpoints SET verification_state='unverified',revision=revision+1 WHERE id='initial-phone'",[]).unwrap();
                    }
                    "generation" => {
                        c.execute(
                            "UPDATE connections SET generation=generation+1 WHERE session_id=?1",
                            [&f.request.session_id],
                        )
                        .unwrap();
                    }
                    "runtime" => {
                        c.execute("UPDATE connections SET observed_runtime_id='foreign' WHERE session_id=?1",[&f.request.session_id]).unwrap();
                    }
                    "membership" => {
                        c.execute(
                            "UPDATE participants SET active=0 WHERE participant_id=?1",
                            [&f.parts[1]],
                        )
                        .unwrap();
                    }
                    _ => unreachable!(),
                }
            }
        }
        f.store
            .reconcile_session_reconnect(&f.request.conversation_id, &f.request.session_id)
            .unwrap();
        assert_eq!(activation_phase(&f), "accepted", "{mode}");
    }
}
#[test]
fn initial_ready_activation_storage_failure_retries_metadata_without_new_media_or_operation_effects(
) {
    let f = Fixture::new();
    let (_, media) = ready_phone(&f);
    creator_leaves(&f);
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER reject_start BEFORE INSERT ON session_started_facts BEGIN SELECT RAISE(ABORT,'isolated activation write failure'); END;").unwrap();
    assert!(f
        .store
        .reconcile_session_reconnect(&f.request.conversation_id, &f.request.session_id)
        .is_err());
    assert_eq!(activation_phase(&f), "accepted");
    assert!(f
        .store
        .try_initial_telephone_media_read(media)
        .unwrap()
        .is_connected());
    let before: u64 = f
        .store
        .connection()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM operation_effects", [], |r| r.get(0))
        .unwrap();
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_start;")
        .unwrap();
    f.store
        .reconcile_session_reconnect(&f.request.conversation_id, &f.request.session_id)
        .unwrap();
    assert_eq!(activation_phase(&f), "active");
    let c = f.store.connection().unwrap();
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM operation_effects", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        before
    );
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM session_started_facts", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM native_connection_flow_facts",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn unready_session_scan_does_not_queue_a_writer_behind_an_existing_media_interval() {
    let f = Fixture::new();
    let (_, media) = ready_phone(&f);
    let read = f.store.try_initial_telephone_media_read(media).unwrap();
    let began = std::time::Instant::now();
    assert!(!f
        .store
        .reconcile_session_reconnect(&f.request.conversation_id, &f.request.session_id)
        .unwrap());
    assert!(began.elapsed() < std::time::Duration::from_millis(250));
    assert!(read.current());
    assert_eq!(activation_phase(&f), "accepted");
}

#[test]
fn public_answer_cannot_use_private_connection_acceptance() {
    let f = Fixture::new();
    let signal = Arc::new(f.signal());
    let capture = signal.capture_answer(signal.sip_call_id(), "pcmu").unwrap();
    assert!(f
        .store
        .record_private_initial_telephone_answer(&capture)
        .is_err());
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row("SELECT count(*) FROM initial_telephone_answers", [], |r| {
                r.get::<_, u64>(0)
            })
            .unwrap(),
        0
    );
    assert!(f
        .store
        .record_initial_telephone_answer(&capture)
        .unwrap()
        .media
        .is_some());
}
