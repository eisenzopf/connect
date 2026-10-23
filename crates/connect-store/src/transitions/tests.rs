#![allow(clippy::unwrap_used)]
use super::*;
mod authority_tests;
mod consultation;
mod peer_operations;
use crate::{Scope, Store};
use connect_protocol::{Limits, Operation, Outcome};
use std::sync::Arc;
struct Fixture {
    dir: tempfile::TempDir,
    store: Arc<Store>,
    ids: Vec<String>,
    keys: Vec<String>,
    parts: Vec<String>,
    cid: String,
    sid: String,
}
impl Fixture {
    fn new() -> Self {
        Self::limits(Limits::default())
    }
    fn limits(limits: Limits) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::initialize(&dir.path().join("state.sqlite3"), limits).unwrap());
        let mut ids = vec![];
        let mut keys = vec![];
        for alias in ["alice", "bob", "carol"] {
            let id = store.create_identity(alias).unwrap().id;
            keys.push(
                store
                    .issue(&id, Scope::operator(), crate::now() + 3600)
                    .unwrap()
                    .id,
            );
            ids.push(id);
        }
        let Outcome::Conversation(conv) = store
            .execute(
                &keys[0],
                &Operation::Create {
                    request_id: "create".into(),
                    request: connect_protocol::conversations::CreateRequest::persistent(
                        ids[1..].to_vec(),
                    ),
                },
            )
            .unwrap()
        else {
            panic!()
        };
        let parts = ids
            .iter()
            .map(|id| {
                store
                    .membership_snapshot(&keys[0], &conv.id)
                    .unwrap()
                    .participants
                    .into_iter()
                    .find(|p| &p.identity_id == id)
                    .unwrap()
                    .participant_id
            })
            .collect::<Vec<_>>();
        let Outcome::Session(session) = store
            .execute(
                &keys[0],
                &Operation::SessionInvite {
                    request_id: "invite".into(),
                    conversation_id: conv.id.clone(),
                    participant_id: ids[1].clone(),
                    media: vec!["audio".into()],
                },
            )
            .unwrap()
        else {
            panic!()
        };
        store
            .execute(
                &keys[1],
                &Operation::SessionAccept {
                    request_id: "accept".into(),
                    conversation_id: conv.id.clone(),
                    session_id: session.id.clone(),
                },
            )
            .unwrap();
        // Trusted, isolated adapter rows. These qualify store fencing/transactions,
        // not physical media readiness; no network or provider is contacted.
        {
            let c = store.connection().unwrap();
            for n in 0..3 {
                let device = format!("dev-{n}");
                let ep = format!("ep-{n}");
                let connection = ["conn-alice", "conn-bob", "conn-carol"][n];
                c.execute("INSERT INTO devices(id,identity_id,tenant_id,kind,platform) VALUES(?1,?2,'tnt_default','fixture','fixture')",params![device,ids[n]]).unwrap();
                c.execute("INSERT INTO endpoints(id,identity_id,tenant_id,device_id,transport,address,capabilities_json,verification_state,verification_evidence,verified_at) VALUES(?1,?2,'tnt_default',?3,'webrtc',?4,'[\"audio\"]','verified','fixture-proof',?5)",params![ep,ids[n],device,format!("https://fixture.invalid/{n}"),crate::now()]).unwrap();
                c.execute("INSERT INTO session_participants(session_id,conversation_id,tenant_id,participant_id,membership_epoch,role,state) VALUES(?1,?2,'tnt_default',?3,1,'participant','present') ON CONFLICT(session_id,participant_id,membership_epoch) DO UPDATE SET role=excluded.role,state=excluded.state",params![session.id,conv.id,parts[n]]).unwrap();
                c.execute("INSERT INTO connections(id,session_id,participant_id,membership_epoch,identity_id,tenant_id,endpoint_id,generation,state,observed_runtime_id,observed_at,readiness_json) VALUES(?1,?2,?3,1,?4,'tnt_default',?5,1,?6,?7,?8,'{\"ingress\":true,\"egress\":true}')",params![connection,session.id,parts[n],ids[n],ep,if n==2{"ready"}else{"active"},store.runtime_epoch(),crate::now()]).unwrap();
            }
        }
        Self {
            dir,
            store,
            ids,
            keys,
            parts,
            cid: conv.id,
            sid: session.id,
        }
    }
    fn request(&self, id: &str) -> TransitionRequest {
        TransitionRequest {
            payload_extensions: Default::default(),
            operation_id: id.into(),
            conversation_id: self.cid.clone(),
            session_id: self.sid.clone(),
            connection_id: None,
            expected_revision: 0,
            expires_at: crate::now() + 300,
            intent: TransitionIntent::Consult {
                retained_connid: "conn-bob".into(),
                private_participants: vec![self.parts[0].clone(), self.parts[2].clone()],
                hold_treatment: HoldTreatment::Silence,
            },
        }
    }
    fn admit(&self, id: &str) -> OperationRecord {
        self.store
            .admit_transition(&self.keys[0], self.request(id))
            .unwrap()
    }
    fn get(&self, id: &str) -> OperationRecord {
        self.store
            .execute_operation_action(
                &self.keys[0],
                OperationAction::Get {
                    operation_id: id.into(),
                },
            )
            .unwrap()
    }
    fn claim(&self, id: &str) -> TransitionLease {
        self.store
            .claim_transition(&self.keys[0], id, 60)
            .unwrap()
            .unwrap()
            .0
    }
    fn cancel(&self, id: &str, target: &str) -> OperationRecord {
        self.store
            .execute_operation_action(
                &self.keys[0],
                OperationAction::Cancel {
                    operation_id: id.into(),
                    target_operation_id: target.into(),
                    reason: None,
                },
            )
            .unwrap()
    }
    fn effect(&self, lease: &TransitionLease, id: &str) -> OperationEffect {
        self.store
            .prepare_transition_effect(
                &self.keys[0],
                lease,
                id,
                "fixture",
                id,
                IdempotencyMode::None,
                None,
            )
            .unwrap();
        self.store
            .begin_transition_effect(&self.keys[0], lease, id)
            .unwrap()
    }
    fn observe(
        &self,
        op: &str,
        effect: &OperationEffect,
        event: &str,
        state: EffectState,
    ) -> OperationRecord {
        self.store
            .observe_transition_effect(
                &self.keys[0],
                op,
                &effect.id,
                "fixture",
                event,
                effect.submission_generation,
                state,
                Some(&format!("provider-{}", effect.id)),
            )
            .unwrap()
    }
    fn completion(&self) -> TransitionCompletion {
        TransitionCompletion {
            retained_connection_ids: vec!["conn-bob".into()],
            admitted_connection_ids: vec!["conn-carol".into()],
            retired_connection_ids: vec![],
            session_revision: 1,
            conversation_revision: None,
        }
    }
    fn commit(&self, lease: &TransitionLease) -> Result<OperationRecord> {
        self.store.commit_transition(&self.keys[0], lease, |c, r| {
            c.execute(
                "UPDATE sessions SET topology_revision=topology_revision+1 WHERE id=?1",
                [&r.session_id],
            )
            .map_err(|_| Error::storage())?;
            c.execute(
                "UPDATE connections SET state='active' WHERE id='conn-carol'",
                [],
            )
            .map_err(|_| Error::storage())?;
            Ok(self.completion())
        })
    }
}
#[test]
fn stable_owner_replay_conflict_current_scope_and_connection_epoch_are_enforced() {
    let f = Fixture::new();
    let request = f.request("one");
    let accepted = f
        .store
        .admit_transition(&f.keys[0], request.clone())
        .unwrap();
    assert_eq!(accepted.state, OperationState::Accepted);
    let rotated = f
        .store
        .issue(&f.ids[0], Scope::operator(), crate::now() + 600)
        .unwrap()
        .id;
    assert_eq!(
        f.store.admit_transition(&rotated, request.clone()).unwrap(),
        accepted
    );
    let mut changed = request.clone();
    changed.expires_at += 1;
    assert_eq!(
        f.store
            .admit_transition(&rotated, changed)
            .unwrap_err()
            .code,
        "conflict"
    );
    let narrowed = f
        .store
        .issue(
            &f.ids[0],
            Scope {
                operations: vec!["session.consult".into(), "operation.get".into()],
                recipients: vec![f.ids[1].clone()],
            },
            crate::now() + 600,
        )
        .unwrap();
    assert_eq!(
        f.store
            .execute_operation_action(
                &narrowed.id,
                OperationAction::Get {
                    operation_id: "one".into()
                }
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        f.store
            .execute_operation_action(
                &f.keys[1],
                OperationAction::Get {
                    operation_id: "one".into()
                }
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        f.store
            .admit_transition(&f.keys[0], f.request("competing"))
            .unwrap_err()
            .code,
        "conflict"
    );
    let lease = f.claim("one");
    assert_eq!(
        f.store
            .advance_transition(&rotated, &lease, OperationState::Ready)
            .unwrap()
            .state,
        OperationState::Ready
    );
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE connections SET generation=generation+1 WHERE id='conn-bob'",
            [],
        )
        .unwrap();
    assert_eq!(
        f.store
            .advance_transition(&rotated, &lease, OperationState::Ready)
            .unwrap_err()
            .code,
        "forbidden"
    );
}
#[test]
fn concurrent_workers_and_duplicate_submission_have_one_external_attempt() {
    let f = Fixture::new();
    f.admit("one");
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let mut joins = vec![];
    for _ in 0..2 {
        let store = f.store.clone();
        let key = f.keys[0].clone();
        let barrier = barrier.clone();
        joins.push(std::thread::spawn(move || {
            barrier.wait();
            store.claim_transition(&key, "one", 60).unwrap()
        }));
    }
    let leases = joins
        .into_iter()
        .filter_map(|j| j.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(leases.len(), 1);
    let lease = &leases[0].0;
    f.store
        .prepare_transition_effect(
            &f.keys[0],
            lease,
            "call-once",
            "fixture",
            "call-once",
            IdempotencyMode::None,
            None,
        )
        .unwrap();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut dispatches = vec![];
    for _ in 0..2 {
        let store = f.store.clone();
        let key = f.keys[0].clone();
        let lease = lease.clone();
        let calls = calls.clone();
        dispatches.push(std::thread::spawn(move || {
            let result = store.begin_transition_effect(&key, &lease, "call-once");
            if result.is_ok() {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            } // actual fixture callback, outside SQL
            result
        }));
    }
    let mut accepted = dispatches
        .into_iter()
        .filter_map(|j| j.join().unwrap().ok())
        .collect::<Vec<_>>();
    assert_eq!(accepted.len(), 1);
    let effect = accepted.pop().unwrap();
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);

    assert_eq!(effect.attempt, 1);
    assert_eq!(
        f.store
            .begin_transition_effect(&f.keys[0], lease, "call-once")
            .unwrap_err()
            .code,
        "reconciliation-required"
    );
    assert_eq!(f.get("one").effects[0].attempt, 1);
    f.observe("one", &effect, "accepted-once", EffectState::Confirmed);
    let before = f.get("one");
    f.observe("one", &effect, "accepted-once", EffectState::Confirmed);
    assert_eq!(f.get("one"), before);
    assert_eq!(
        f.store
            .observe_transition_effect(
                &f.keys[0],
                "one",
                &effect.id,
                "fixture",
                "accepted-once",
                effect.submission_generation,
                EffectState::Failed,
                Some("provider-call-once")
            )
            .unwrap_err()
            .code,
        "conflict"
    );
}
#[test]
fn backup_after_submit_is_unknown_reserved_and_never_automatically_submitted_again() {
    let f = Fixture::new();
    f.admit("one");
    let lease = f.claim("one");
    let effect = f.effect(&lease, "external");
    let backup = f.dir.path().join("restored.sqlite3");
    f.store.backup(&backup).unwrap();
    let restored = Store::open(&backup, Limits::default()).unwrap();
    let row = restored
        .execute_operation_action(
            &f.keys[0],
            OperationAction::Get {
                operation_id: "one".into(),
            },
        )
        .unwrap();
    assert_eq!(row.state, OperationState::Unknown);
    assert_eq!(row.effects[0].state, EffectState::Unknown);
    assert_eq!(row.effects[0].attempt, 1);
    assert!(restored
        .claim_transition(&f.keys[0], "one", 60)
        .unwrap()
        .is_none());
    assert_eq!(
        restored
            .admit_transition(&f.keys[0], f.request("competing"))
            .unwrap_err()
            .code,
        "forbidden"
    );
    // A restored graph needs a fresh trusted adapter observation. This local
    // fixture re-observes it explicitly; it cannot release the unknown reservation.
    restored.connection().unwrap().execute("UPDATE connections SET observed_runtime_id=?1,state=CASE WHEN id='conn-carol' THEN 'ready' ELSE 'active' END,readiness_json='{\"ingress\":true,\"egress\":true}'", [restored.runtime_epoch()]).unwrap();
    assert_eq!(
        restored
            .admit_transition(&f.keys[0], f.request("competing"))
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(
        restored
            .advance_transition(&f.keys[0], &lease, OperationState::Ready)
            .unwrap_err()
            .code,
        "forbidden"
    );
    restored
        .observe_transition_effect(
            &f.keys[0],
            "one",
            &effect.id,
            "fixture",
            "verified-failure",
            effect.submission_generation,
            EffectState::Failed,
            Some("provider-external"),
        )
        .unwrap();
    let evidence = restored
        .record_transition_evidence(
            &f.keys[0],
            "one",
            "probe-failed",
            EvidenceSource::ProviderStatus,
            ReconcileOutcome::Failed,
            None,
        )
        .unwrap();
    restored
        .execute_operation_action(
            &f.keys[0],
            OperationAction::Reconcile {
                operation_id: "settle".into(),
                target_operation_id: "one".into(),
                outcome: ReconcileOutcome::Failed,
                evidence_id: evidence,
            },
        )
        .unwrap();
    assert_eq!(
        restored
            .execute_operation_action(
                &f.keys[0],
                OperationAction::Get {
                    operation_id: "one".into()
                }
            )
            .unwrap()
            .state,
        OperationState::Failed
    );
    assert_eq!(
        restored
            .admit_transition(&f.keys[0], f.request("new-one"))
            .unwrap()
            .state,
        OperationState::Accepted
    );
}
#[test]
fn cancellation_before_commit_fences_late_worker_and_new_controller_can_reserve() {
    let f = Fixture::new();
    f.admit("one");
    let lease = f.claim("one");
    let cancel = f.cancel("cancel-one", "one");
    assert_eq!(
        cancel.result.as_ref().unwrap()["state_at_decision"],
        "cancelled"
    );
    assert_eq!(f.get("one").state, OperationState::Cancelled);
    assert_eq!(
        f.store
            .advance_transition(&f.keys[0], &lease, OperationState::Committing)
            .unwrap_err()
            .code,
        "forbidden"
    );
    f.admit("two");
    assert_eq!(f.commit(&lease).unwrap_err().code, "forbidden");
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT topology_revision FROM sessions WHERE id=?1",
                [&f.sid],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(f.cancel("cancel-one", "one"), cancel);
}
#[test]
fn confirmed_creation_requires_explicit_compensation_before_cancel_reconciliation() {
    let f = Fixture::new();
    f.admit("one");
    let lease = f.claim("one");
    let effect = f.effect(&lease, "create-candidate");
    f.observe("one", &effect, "created", EffectState::Confirmed);
    f.cancel("cancel-one", "one");
    assert_eq!(f.get("one").state, OperationState::Unknown);
    let evidence = f
        .store
        .record_transition_evidence(
            &f.keys[0],
            "one",
            "not-enough",
            EvidenceSource::OperatorAttestation,
            ReconcileOutcome::Cancelled,
            None,
        )
        .unwrap();
    assert_eq!(
        f.store
            .execute_operation_action(
                &f.keys[0],
                OperationAction::Reconcile {
                    operation_id: "bad-settle".into(),
                    target_operation_id: "one".into(),
                    outcome: ReconcileOutcome::Cancelled,
                    evidence_id: evidence
                }
            )
            .unwrap_err()
            .code,
        "reconciliation-required"
    );
    let cleanup = f
        .store
        .claim_transition_cleanup(&f.keys[0], "one", 60)
        .unwrap()
        .unwrap();
    assert_eq!(
        f.store
            .advance_transition(&f.keys[0], &cleanup, OperationState::Committing)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(f.commit(&cleanup).unwrap_err().code, "forbidden");
    f.store
        .prepare_transition_effect(
            &f.keys[0],
            &cleanup,
            "cleanup-candidate",
            "fixture",
            "cleanup-candidate",
            IdempotencyMode::StatusQuery,
            Some("create-candidate"),
        )
        .unwrap();
    let cleanup_effect = f
        .store
        .begin_transition_effect(&f.keys[0], &cleanup, "cleanup-candidate")
        .unwrap();
    f.observe("one", &cleanup_effect, "cleaned", EffectState::Confirmed);
    let evidence = f
        .store
        .record_transition_evidence(
            &f.keys[0],
            "one",
            "cleanup-probe",
            EvidenceSource::ProviderStatus,
            ReconcileOutcome::Cancelled,
            None,
        )
        .unwrap();
    f.store
        .execute_operation_action(
            &f.keys[0],
            OperationAction::Reconcile {
                operation_id: "settled-cancel".into(),
                target_operation_id: "one".into(),
                outcome: ReconcileOutcome::Cancelled,
                evidence_id: evidence,
            },
        )
        .unwrap();
    assert_eq!(f.get("one").state, OperationState::Cancelled);
    assert!(f
        .get("one")
        .effects
        .iter()
        .all(|e| e.state == EffectState::Confirmed));
}
#[test]
fn full_effect_budget_keeps_a_separate_bounded_compensation_lane() {
    let f = Fixture::new();
    f.admit("full");
    let lease = f.claim("full");
    for n in 0..32 {
        let effect = f.effect(&lease, &format!("create-{n}"));
        f.observe(
            "full",
            &effect,
            &format!("created-{n}"),
            EffectState::Confirmed,
        );
    }
    assert_eq!(
        f.store
            .prepare_transition_effect(
                &f.keys[0],
                &lease,
                "overflow",
                "fixture",
                "overflow",
                IdempotencyMode::None,
                None,
            )
            .unwrap_err()
            .code,
        "capacity-exceeded"
    );
    f.cancel("cancel-full", "full");
    let cleanup = f
        .store
        .claim_transition_cleanup(&f.keys[0], "full", 60)
        .unwrap()
        .unwrap();
    for n in 0..32 {
        let id = format!("compensate-{n}");
        f.store
            .prepare_transition_effect(
                &f.keys[0],
                &cleanup,
                &id,
                "fixture",
                &id,
                IdempotencyMode::StatusQuery,
                Some(&format!("create-{n}")),
            )
            .unwrap();
        let effect = f
            .store
            .begin_transition_effect(&f.keys[0], &cleanup, &id)
            .unwrap();
        f.observe(
            "full",
            &effect,
            &format!("compensated-{n}"),
            EffectState::Confirmed,
        );
    }
    assert_eq!(f.get("full").effects.len(), 64);
    assert_eq!(
        f.store
            .prepare_transition_effect(
                &f.keys[0],
                &cleanup,
                "over-compensation",
                "fixture",
                "over-compensation",
                IdempotencyMode::StatusQuery,
                Some("create-0"),
            )
            .unwrap_err()
            .code,
        "capacity-exceeded"
    );
    let evidence = f
        .store
        .record_transition_evidence(
            &f.keys[0],
            "full",
            "full-cleanup-proof",
            EvidenceSource::ProviderStatus,
            ReconcileOutcome::Cancelled,
            None,
        )
        .unwrap();
    f.store
        .execute_operation_action(
            &f.keys[0],
            OperationAction::Reconcile {
                operation_id: "settle-full".into(),
                target_operation_id: "full".into(),
                outcome: ReconcileOutcome::Cancelled,
                evidence_id: evidence,
            },
        )
        .unwrap();
    assert_eq!(f.get("full").state, OperationState::Cancelled);
    assert!(f
        .get("full")
        .effects
        .iter()
        .all(|effect| effect.state == EffectState::Confirmed));
}
#[test]
fn local_decision_and_completion_are_atomic_and_cancel_after_commit_cannot_undo_it() {
    let f = Fixture::new();
    f.admit("one");
    let lease = f.claim("one");
    f.store
        .advance_transition(&f.keys[0], &lease, OperationState::Ready)
        .unwrap();
    f.store
        .advance_transition(&f.keys[0], &lease, OperationState::Committing)
        .unwrap();
    let result = f.commit(&lease).unwrap();
    assert_eq!(result.state, OperationState::Completed);
    assert_eq!(result.result.as_ref().unwrap()["session_revision"], 1);
    let cancel = f.cancel("late-cancel", "one");
    assert_eq!(
        cancel.result.as_ref().unwrap()["disposition"],
        "already_terminal"
    );
    assert_eq!(f.get("one").state, OperationState::Completed);
    assert_eq!(f.commit(&lease).unwrap_err().code, "forbidden");
}
#[test]
fn administration_child_get_retry_and_claim_preserve_live_transition_worker() {
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let f = Fixture::new();
    let request = f.request("live-worker");
    f.store
        .admit_transition(&f.keys[0], request.clone())
        .unwrap();
    let lease = f.claim("live-worker");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "transitions::tests::crash_worker",
            "--ignored",
            "--nocapture",
        ])
        .env("CONNECT_LEDGER_FIXTURE_DB", f.store.path())
        .env("CONNECT_LEDGER_FIXTURE_KEY", &f.keys[0])
        .env(
            "CONNECT_LEDGER_FIXTURE_REQUEST",
            serde_json::to_string(&request).unwrap(),
        )
        .env("CONNECT_LEDGER_CRASH_STAGE", "administration")
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("administration child timed out");
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(f.get("live-worker").state, OperationState::Preparing);
    f.store
        .renew_transition_lease(&f.keys[0], &lease, 60)
        .unwrap();
    f.store
        .advance_transition(&f.keys[0], &lease, OperationState::Ready)
        .unwrap();
}
#[test]
#[ignore = "subprocess fixture, explicitly invoked by crash and administration parent tests"]
fn crash_worker() {
    use std::io::Write;
    let path = std::path::PathBuf::from(std::env::var_os("CONNECT_LEDGER_FIXTURE_DB").unwrap());
    let stage = std::env::var("CONNECT_LEDGER_CRASH_STAGE").unwrap();
    let store = if stage == "administration" {
        Store::open_administration(&path, Limits::default()).unwrap()
    } else {
        Store::open(&path, Limits::default()).unwrap()
    };
    let key = std::env::var("CONNECT_LEDGER_FIXTURE_KEY").unwrap();
    let request: TransitionRequest =
        serde_json::from_str(&std::env::var("CONNECT_LEDGER_FIXTURE_REQUEST").unwrap()).unwrap();
    if stage == "administration" {
        let row = store
            .execute_operation_action(
                &key,
                OperationAction::Get {
                    operation_id: request.operation_id.clone(),
                },
            )
            .unwrap();
        assert_eq!(row.state, OperationState::Preparing);
        assert_eq!(
            store.admit_transition(&key, request.clone()).unwrap().state,
            OperationState::Preparing
        );
        assert!(store
            .claim_transition(&key, &request.operation_id, 60)
            .unwrap()
            .is_none());
        return;
    }
    // A fresh trusted fixture adapter re-observes its local rows. These are not
    // copied network handles and are never a real provider/media qualification.
    {
        let c = store.connection().unwrap();
        c.execute("UPDATE connections SET observed_runtime_id=?1,state=CASE WHEN id='conn-carol' THEN 'ready' ELSE 'active' END",[store.runtime_epoch()]).unwrap();
    }
    store.admit_transition(&key, request.clone()).unwrap();
    let (lease, _) = store
        .claim_transition(&key, &request.operation_id, 60)
        .unwrap()
        .unwrap();
    if stage.starts_with("decision-cancel:") {
        store
            .execute_operation_action(
                &key,
                OperationAction::Cancel {
                    operation_id: "cancel-crash".into(),
                    target_operation_id: request.operation_id.clone(),
                    reason: None,
                },
            )
            .unwrap();
        panic!("cancel crash boundary was not reached");
    }
    store
        .prepare_transition_effect(
            &key,
            &lease,
            "effect-one",
            "fixture",
            "correlation-one",
            IdempotencyMode::None,
            None,
        )
        .unwrap();
    let effect = store
        .begin_transition_effect(&key, &lease, "effect-one")
        .unwrap();
    let mut counter = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(std::env::var_os("CONNECT_LEDGER_FIXTURE_COUNTER").unwrap())
        .unwrap();
    counter.write_all(b"submitted\n").unwrap();
    counter.sync_all().unwrap();
    fault("external:after-submit", &request.operation_id);
    if stage.starts_with("cleanup-decision:") {
        let record = store
            .execute_operation_action(
                &key,
                OperationAction::Get {
                    operation_id: request.operation_id.clone(),
                },
            )
            .unwrap();
        store
            .settle_abandoned_transition(
                &key,
                &record.record_id,
                "cleanup-crash",
                "absence-probe",
                EvidenceSource::ProviderStatus,
                &[CleanupSettlement::Absent {
                    effect_id: "effect-one".into(),
                    provider: "fixture".into(),
                    submission_generation: effect.submission_generation,
                }],
                ReconcileOutcome::Cancelled,
            )
            .unwrap();
        panic!("cleanup crash boundary was not reached");
    }
    let decision = stage.starts_with("evidence:") || stage.starts_with("decision-reconcile:");
    store
        .observe_transition_effect(
            &key,
            &request.operation_id,
            "effect-one",
            "fixture",
            "confirmed",
            effect.submission_generation,
            if decision {
                EffectState::Failed
            } else {
                EffectState::Confirmed
            },
            Some("fixture-ref"),
        )
        .unwrap();
    if decision {
        store
            .advance_transition(&key, &lease, OperationState::Unknown)
            .unwrap();
        let evidence = store
            .record_transition_evidence(
                &key,
                &request.operation_id,
                "absence-proof",
                EvidenceSource::ProviderStatus,
                ReconcileOutcome::Cancelled,
                None,
            )
            .unwrap();
        store
            .execute_operation_action(
                &key,
                OperationAction::Reconcile {
                    operation_id: "reconcile-crash".into(),
                    target_operation_id: request.operation_id.clone(),
                    outcome: ReconcileOutcome::Cancelled,
                    evidence_id: evidence,
                },
            )
            .unwrap();
        panic!("evidence/reconciliation crash boundary was not reached");
    }
    store
        .advance_transition(&key, &lease, OperationState::Ready)
        .unwrap();
    store
        .advance_transition(&key, &lease, OperationState::Committing)
        .unwrap();
    let apply = |c: &Connection, r: &TransitionRequest| {
        c.execute(
            "UPDATE sessions SET topology_revision=topology_revision+1 WHERE id=?1",
            [&r.session_id],
        )
        .map_err(|_| Error::storage())?;
        c.execute(
            "UPDATE connections SET state='active' WHERE id='conn-carol'",
            [],
        )
        .map_err(|_| Error::storage())?;
        c.execute("UPDATE session_participants SET role='speaker' WHERE session_id=?1 AND participant_id=(SELECT participant_id FROM connections WHERE id='conn-carol')",[&r.session_id]).map_err(|_|Error::storage())?;
        Ok(TransitionCompletion {
            retained_connection_ids: vec!["conn-bob".into()],
            admitted_connection_ids: vec!["conn-carol".into()],
            retired_connection_ids: vec![],
            session_revision: 1,
            conversation_revision: None,
        })
    };
    if std::env::var("CONNECT_LEDGER_CRASH_STAGE")
        .unwrap()
        .starts_with("reconcile-local:")
    {
        store
            .advance_transition(&key, &lease, OperationState::Unknown)
            .unwrap();
        let completion = TransitionCompletion {
            retained_connection_ids: vec!["conn-bob".into()],
            admitted_connection_ids: vec!["conn-carol".into()],
            retired_connection_ids: vec![],
            session_revision: 1,
            conversation_revision: None,
        };
        let evidence = store
            .record_transition_evidence(
                &key,
                &request.operation_id,
                "verified-graph",
                EvidenceSource::LocalState,
                ReconcileOutcome::Completed,
                Some(completion),
            )
            .unwrap();
        store
            .reconcile_transition_commit(
                &key,
                &request.operation_id,
                "reconcile-crash",
                &evidence,
                apply,
            )
            .unwrap();
    } else {
        store.commit_transition(&key, &lease, apply).unwrap();
    }
    panic!("selected crash boundary was not reached")
}
fn crashed_fixture(stage: &str) -> (Fixture, TransitionRequest, std::path::PathBuf) {
    let mut f = Fixture::new();
    let request = f.request("crash-operation");
    // The controller is administrative; only the child owns the simulated runtime.
    // Retaining a real parent runtime would correctly refuse the child's adoption.
    let admin = Store::open_administration(f.store.path(), Limits::default()).unwrap();
    f.store = Arc::new(admin);
    let counter = f.dir.path().join("external-calls.txt");
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "transitions::tests::crash_worker",
            "--nocapture",
        ])
        .env(
            "CONNECT_LEDGER_FIXTURE_DB",
            f.dir.path().join("state.sqlite3"),
        )
        .env("CONNECT_LEDGER_FIXTURE_KEY", &f.keys[0])
        .env(
            "CONNECT_LEDGER_FIXTURE_REQUEST",
            serde_json::to_string(&request).unwrap(),
        )
        .env("CONNECT_LEDGER_FIXTURE_COUNTER", &counter)
        .env("CONNECT_LEDGER_CRASH_OPERATION", &request.operation_id)
        .env("CONNECT_LEDGER_CRASH_STAGE", stage)
        .output()
        .unwrap();
    assert_eq!(
        status.status.code(),
        Some(86),
        "{stage}: {}",
        String::from_utf8_lossy(&status.stdout)
    );
    (f, request, counter)
}
#[test]
fn process_crashes_cover_durable_boundaries_without_duplicate_callbacks_or_partial_local_commit() {
    let stages = [
        "admit:before-commit",
        "admit:after-commit",
        "claim:before-commit",
        "claim:after-commit",
        "effect-prepare:before-commit",
        "effect-prepare:after-commit",
        "effect-submit:before-commit",
        "effect-submit:after-commit",
        "external:after-submit",
        "effect-observe:before-commit",
        "effect-observe:after-commit",
        "advance-ready:before-commit",
        "advance-ready:after-commit",
        "advance-committing:before-commit",
        "advance-committing:after-commit",
        "local-commit:before-commit",
        "local-commit:after-commit",
        "reconcile-local:before-commit",
        "reconcile-local:after-commit",
    ];
    for stage in stages {
        let (f, request, counter) = crashed_fixture(stage);
        let restored = Store::open(&f.dir.path().join("state.sqlite3"), Limits::default()).unwrap();
        let c = restored.connection().unwrap();
        let count: u64 = c
            .query_row(
                "SELECT COUNT(*) FROM operations WHERE operation_id='crash-operation'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let revision: u64 = c
            .query_row(
                "SELECT topology_revision FROM sessions WHERE id=?1",
                [&f.sid],
                |r| r.get(0),
            )
            .unwrap();
        let role: String = c
            .query_row(
                "SELECT role FROM session_participants WHERE session_id=?1 AND participant_id=?2",
                params![f.sid, f.parts[2]],
                |r| r.get(0),
            )
            .unwrap();
        drop(c);
        if stage == "admit:before-commit" {
            assert_eq!(count, 0);
            assert_eq!(revision, 0);
            continue;
        }
        assert_eq!(count, 1, "{stage}");
        let calls = std::fs::read_to_string(&counter)
            .unwrap_or_default()
            .lines()
            .count();
        let should_call = stages_position(stage) >= stages_position("external:after-submit");
        assert_eq!(calls, usize::from(should_call), "{stage}");
        let record = restored
            .execute_operation_action(
                &f.keys[0],
                OperationAction::Get {
                    operation_id: request.operation_id.clone(),
                },
            )
            .unwrap();
        assert_eq!(
            restored
                .admit_transition(&f.keys[0], request.clone())
                .unwrap()
                .record_id,
            record.record_id,
            "same business ID must replay {stage}"
        );
        if ["local-commit:after-commit", "reconcile-local:after-commit"].contains(&stage) {
            assert_eq!(record.state, OperationState::Completed);
            assert_eq!(revision, 1);
            assert_eq!(role, "speaker");
        } else {
            assert_eq!(revision, 0, "{stage}");
            assert_eq!(role, "participant");
            assert_ne!(record.state, OperationState::Completed);
        }
        if stages_position(stage) >= stages_position("effect-submit:after-commit")
            && !["local-commit:after-commit", "reconcile-local:after-commit"].contains(&stage)
        {
            assert!(
                matches!(
                    record.state,
                    OperationState::Unknown | OperationState::Interrupted
                ),
                "{stage}: {:?}",
                record.state
            );
            assert!(restored
                .claim_transition(&f.keys[0], &request.operation_id, 60)
                .unwrap()
                .is_none());
            assert_eq!(
                restored
                    .admit_transition(&f.keys[0], f.request("competing"))
                    .unwrap_err()
                    .code,
                "forbidden"
            );
        }
    }
    fn stages_position(stage: &str) -> usize {
        [
            "admit:before-commit",
            "admit:after-commit",
            "claim:before-commit",
            "claim:after-commit",
            "effect-prepare:before-commit",
            "effect-prepare:after-commit",
            "effect-submit:before-commit",
            "effect-submit:after-commit",
            "external:after-submit",
            "effect-observe:before-commit",
            "effect-observe:after-commit",
            "advance-ready:before-commit",
            "advance-ready:after-commit",
            "advance-committing:before-commit",
            "advance-committing:after-commit",
            "local-commit:before-commit",
            "local-commit:after-commit",
            "reconcile-local:before-commit",
            "reconcile-local:after-commit",
        ]
        .iter()
        .position(|v| *v == stage)
        .unwrap()
    }
}
#[test]
fn process_crashes_keep_cancellation_evidence_reconciliation_and_cleanup_atomic() {
    let decision_stages = [
        "decision-cancel:before-commit",
        "decision-cancel:after-commit",
        "evidence:before-commit",
        "evidence:after-commit",
        "decision-reconcile:before-commit",
        "decision-reconcile:after-commit",
        "cleanup-decision:before-commit",
        "cleanup-decision:after-commit",
    ];
    for stage in decision_stages {
        let (f, request, counter) = crashed_fixture(stage);
        let store = Store::open(&f.dir.path().join("state.sqlite3"), Limits::default()).unwrap();
        let record = store
            .execute_operation_action(
                &f.keys[0],
                OperationAction::Get {
                    operation_id: request.operation_id.clone(),
                },
            )
            .unwrap();
        let terminal = stage.ends_with(":after-commit") && !stage.starts_with("evidence:");
        assert_eq!(
            record.state,
            if terminal {
                OperationState::Cancelled
            } else if stage.starts_with("decision-cancel:") {
                OperationState::Interrupted
            } else {
                OperationState::Unknown
            },
            "{stage}"
        );
        let c = store.connection().unwrap();
        let revision: u64 = c
            .query_row(
                "SELECT topology_revision FROM sessions WHERE id=?1",
                [&f.sid],
                |r| r.get(0),
            )
            .unwrap();
        let role: String = c
            .query_row(
                "SELECT role FROM session_participants WHERE session_id=?1 AND participant_id=?2",
                params![f.sid, f.parts[2]],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!((revision, role.as_str()), (0, "participant"), "{stage}");
        let decisions: u32 = c
            .query_row(
                "SELECT COUNT(*) FROM operation_decisions WHERE target_record_id=?1",
                [&record.record_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(decisions, u32::from(terminal), "{stage}");
        if stage.starts_with("evidence:") {
            let proofs: u32 = c
                .query_row(
                    "SELECT COUNT(*) FROM operation_evidence WHERE operation_record_id=?1",
                    [&record.record_id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                proofs,
                u32::from(stage.ends_with(":after-commit")),
                "{stage}"
            );
        }
        drop(c);
        let calls = || {
            std::fs::read_to_string(&counter)
                .unwrap_or_default()
                .lines()
                .count()
        };
        let expected_calls = usize::from(!stage.starts_with("decision-cancel:"));
        assert_eq!(calls(), expected_calls, "{stage}");
        let decision = if stage.starts_with("decision-cancel:") {
            store
                .execute_operation_action(
                    &f.keys[0],
                    OperationAction::Cancel {
                        operation_id: "cancel-crash".into(),
                        target_operation_id: request.operation_id.clone(),
                        reason: None,
                    },
                )
                .unwrap()
        } else if stage.starts_with("cleanup-decision:") {
            store
                .settle_abandoned_transition(
                    &f.keys[0],
                    &record.record_id,
                    "cleanup-crash",
                    "absence-probe",
                    EvidenceSource::ProviderStatus,
                    &[CleanupSettlement::Absent {
                        effect_id: "effect-one".into(),
                        provider: "fixture".into(),
                        submission_generation: record.effects[0].submission_generation,
                    }],
                    ReconcileOutcome::Cancelled,
                )
                .unwrap()
        } else {
            let proof = store
                .record_transition_evidence(
                    &f.keys[0],
                    &request.operation_id,
                    "absence-proof",
                    EvidenceSource::ProviderStatus,
                    ReconcileOutcome::Cancelled,
                    None,
                )
                .unwrap();
            let same = store
                .record_transition_evidence(
                    &f.keys[0],
                    &request.operation_id,
                    "absence-proof",
                    EvidenceSource::ProviderStatus,
                    ReconcileOutcome::Cancelled,
                    None,
                )
                .unwrap();
            assert_eq!(proof, same, "same evidence ID after crash: {stage}");
            if stage.starts_with("evidence:") {
                assert!(store
                    .claim_transition(&f.keys[0], &request.operation_id, 60)
                    .unwrap()
                    .is_none());
                assert_eq!(calls(), expected_calls);
                continue;
            }
            store
                .execute_operation_action(
                    &f.keys[0],
                    OperationAction::Reconcile {
                        operation_id: "reconcile-crash".into(),
                        target_operation_id: request.operation_id.clone(),
                        outcome: ReconcileOutcome::Cancelled,
                        evidence_id: proof,
                    },
                )
                .unwrap()
        };
        assert_eq!(decision.state, OperationState::Completed);
        assert_eq!(
            store
                .execute_operation_action(
                    &f.keys[0],
                    OperationAction::Get {
                        operation_id: decision.operation_id.clone(),
                    }
                )
                .unwrap()
                .record_id,
            decision.record_id
        );
        assert_eq!(
            store.admit_transition(&f.keys[0], request).unwrap().state,
            OperationState::Cancelled
        );
        assert_eq!(
            calls(),
            expected_calls,
            "recovery cannot submit another callback: {stage}"
        );
        let c = store.connection().unwrap();
        let count: u32 = c
            .query_row(
                "SELECT COUNT(*) FROM operation_decisions WHERE target_record_id=?1",
                [&record.record_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "{stage}");
    }
}
#[test]
fn stale_lease_and_membership_readmission_cannot_restore_old_execution_authority() {
    let f = Fixture::new();
    f.admit("one");
    let old = f.claim("one");
    {
        let c = f.store.connection().unwrap();
        c.execute(
            "UPDATE operations SET lease_expires_at=?1 WHERE operation_id='one'",
            [crate::now() - 1],
        )
        .unwrap();
    }
    let new = f.claim("one");
    assert_ne!(old.generation, new.generation);
    assert_eq!(
        f.store
            .advance_transition(&f.keys[0], &old, OperationState::Ready)
            .unwrap_err()
            .code,
        "forbidden"
    );
    {
        let c = f.store.connection().unwrap();
        c.execute(
            "UPDATE participants SET active=0 WHERE participant_id=?1",
            [&f.parts[2]],
        )
        .unwrap();
        c.execute("UPDATE participants SET active=1,membership_epoch=membership_epoch+1 WHERE participant_id=?1",[&f.parts[2]]).unwrap();
    }
    assert_eq!(
        f.store
            .advance_transition(&f.keys[0], &new, OperationState::Ready)
            .unwrap_err()
            .code,
        "forbidden"
    );
    // Cancellation can clean up the owner's old intent without readmitting its target.
    f.cancel("cancel-old", "one");
    assert_eq!(f.get("one").state, OperationState::Cancelled);
}
#[test]
fn expired_unclaimed_work_settles_without_dispatch_and_tombstones_never_reuse_ids() {
    let f = Fixture::new();
    let mut request = f.request("one");
    // Leave admission headroom when the full suite performs concurrent SQL
    // and crash recovery. Then wait for that same immutable deadline, rather
    // than racing the next wall-clock second before admission can commit.
    request.expires_at = crate::now() + 10;
    f.store
        .admit_transition(&f.keys[0], request.clone())
        .unwrap();
    let wait_until = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while crate::now() < request.expires_at {
        assert!(
            std::time::Instant::now() < wait_until,
            "bounded expiry wait elapsed"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(f.get("one").state, OperationState::Failed);
    assert!(f
        .store
        .claim_transition(&f.keys[0], "one", 60)
        .unwrap()
        .is_none());
    {
        let c = f.store.connection().unwrap();
        c.execute("UPDATE conversations SET closed=1 WHERE id=?1", [&f.cid])
            .unwrap();
        c.execute(
            "UPDATE conversations SET closed_at=?2 WHERE id=?1",
            params![f.cid, crate::now() - 31 * 86400],
        )
        .unwrap();
        c.execute(
            "UPDATE operations SET updated_at=?1 WHERE operation_id='one'",
            [crate::now() - 31 * 86400],
        )
        .unwrap();
    }
    assert_eq!(f.store.purge_transition_results(10).unwrap(), 1);
    assert_eq!(
        f.store
            .admit_transition(&f.keys[0], request.clone())
            .unwrap_err()
            .code,
        "gone"
    );
    let mut changed = request;
    changed.expected_revision = 1;
    assert_eq!(
        f.store
            .admit_transition(&f.keys[0], changed)
            .unwrap_err()
            .code,
        "conflict"
    );
}
#[test]
fn completion_storage_failure_rolls_back_local_role_change_and_same_worker_recovers() {
    let f = Fixture::new();
    f.admit("one");
    let lease = f.claim("one");
    f.store
        .advance_transition(&f.keys[0], &lease, OperationState::Ready)
        .unwrap();
    f.store
        .advance_transition(&f.keys[0], &lease, OperationState::Committing)
        .unwrap();
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER fail_ledger_commit BEFORE UPDATE OF state ON operations WHEN NEW.state='completed' BEGIN SELECT RAISE(ABORT,'injected completion failure'); END;").unwrap();
    assert_eq!(f.commit(&lease).unwrap_err().code, "storage-unavailable");
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT topology_revision FROM sessions WHERE id=?1",
                [&f.sid],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(f.get("one").state, OperationState::Committing);
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_ledger_commit;")
        .unwrap();
    assert_eq!(f.commit(&lease).unwrap().state, OperationState::Completed);
}
#[test]
fn shared_capacity_keeps_room_for_a_terminal_cancel_decision_and_its_causal_event() {
    let f = Fixture::new();
    let admission = f.admit("one");
    let c = f.store.connection().unwrap();
    let consumed: u64 = c
        .query_row(
            "SELECT (SELECT COUNT(*) FROM operations)+(SELECT COUNT(*) FROM task_requests)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let events: u64 = c
        .query_row(
            "SELECT (SELECT COUNT(*) FROM journal)+(SELECT COUNT(*) FROM task_events)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    drop(c);
    let tiny = Store::open(
        &f.dir.path().join("state.sqlite3"),
        Limits {
            operation_receipts: consumed + 1,
            journal_events: events + 1,
            ..Limits::default()
        },
    )
    .unwrap();
    assert_eq!(
        tiny.admit_transition(&f.keys[0], f.request("cannot-fit"))
            .unwrap_err()
            .code,
        "capacity-exceeded"
    );
    let cancelled = tiny
        .execute_operation_action(
            &f.keys[0],
            OperationAction::Cancel {
                operation_id: "cancel-one".into(),
                target_operation_id: "one".into(),
                reason: None,
            },
        )
        .unwrap();
    assert_eq!(cancelled.state, OperationState::Completed);
    assert_eq!(
        cancelled.result.as_ref().unwrap()["state_at_decision"],
        "cancelled"
    );
    assert_eq!(cancelled.causal_event_ids.len(), 1);
    assert_ne!(cancelled.record_id, admission.record_id);
}
#[test]
fn legacy_provisioning_and_membership_status_reuses_current_source_replay_policy() {
    use connect_protocol::{membership::*, provisioning::*};
    let f = Fixture::new();
    f.store
        .execute_provisioning(
            &f.keys[0],
            ProvisioningCommand::Identity {
                operation_id: "provisioned".into(),
                alias: "new-identity".into(),
                kind: IdentityKind::Human,
                display_name: None,
            },
        )
        .unwrap();
    let status = f.get("provisioned");
    assert_eq!(status.state, OperationState::Completed);
    assert!(status.result.is_some());
    let revision = f
        .store
        .membership_snapshot(&f.keys[0], &f.cid)
        .unwrap()
        .revision;
    f.store
        .execute_membership(
            &f.keys[0],
            MembershipRequest {
                operation_id: "role-change".into(),
                conversation_id: f.cid.clone(),
                expected_revision: revision,
                change: MembershipAction::UpdateRole {
                    participant_id: f.parts[2].clone(),
                    role: "reviewer".into(),
                },
            },
        )
        .unwrap();
    assert_eq!(f.get("role-change").state, OperationState::Completed);
    let denied = f
        .store
        .issue(
            &f.ids[0],
            Scope {
                operations: vec!["operation.get".into()],
                recipients: vec!["*".into()],
            },
            crate::now() + 600,
        )
        .unwrap();
    for id in ["provisioned", "role-change"] {
        assert_eq!(
            f.store
                .execute_operation_action(
                    &denied.id,
                    OperationAction::Get {
                        operation_id: id.into()
                    }
                )
                .unwrap_err()
                .code,
            "forbidden"
        );
    }
}
#[test]
fn revoked_owner_does_not_make_uncertainty_unrepairable_or_grant_a_new_operator_live_commit() {
    let f = Fixture::new();
    let record = f.admit("one");
    let lease = f.claim("one");
    let effect = f.effect(&lease, "creation");
    f.observe("one", &effect, "created", EffectState::Confirmed);
    f.store
        .advance_transition(&f.keys[0], &lease, OperationState::Unknown)
        .unwrap();
    let current = f
        .store
        .authenticate(
            &f.store
                .issue(&f.ids[1], Scope::operator(), crate::now() + 600)
                .unwrap()
                .token,
        )
        .unwrap();
    f.store
        .execute_provisioning(
            &current.credential_id,
            connect_protocol::provisioning::ProvisioningCommand::RevokeIdentity {
                operation_id: "revoke-owner".into(),
                identity_id: f.ids[0].clone(),
                expected_revision: 0,
            },
        )
        .unwrap();
    assert_eq!(
        f.store
            .execute_operation_action(
                &current.credential_id,
                OperationAction::Get {
                    operation_id: "one".into()
                }
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        f.store
            .inspect_transition_for_cleanup(&current.credential_id, &record.record_id)
            .unwrap()
            .state,
        OperationState::Unknown
    );
    let narrow = f
        .store
        .issue(
            &f.ids[1],
            Scope {
                operations: vec!["operation.cleanup".into(), "session_get".into()],
                recipients: vec![f.ids[0].clone()],
            },
            crate::now() + 600,
        )
        .unwrap();
    assert_eq!(
        f.store
            .inspect_transition_for_cleanup(&narrow.id, &record.record_id)
            .unwrap_err()
            .code,
        "forbidden"
    );
    let settlement = CleanupSettlement::Compensated {
        effect_id: effect.id.clone(),
        provider: "fixture".into(),
        submission_generation: effect.submission_generation,
    };
    assert_eq!(
        f.store
            .settle_abandoned_transition(
                &current.credential_id,
                &record.record_id,
                "repair",
                "probe",
                EvidenceSource::OperatorAttestation,
                &[settlement.clone()],
                ReconcileOutcome::Cancelled
            )
            .unwrap_err()
            .code,
        "invalid-request"
    );
    assert_eq!(
        f.store
            .settle_abandoned_transition(
                &current.credential_id,
                &record.record_id,
                "repair",
                "probe",
                EvidenceSource::ProviderStatus,
                &[settlement.clone()],
                ReconcileOutcome::Completed
            )
            .unwrap_err()
            .code,
        "invalid-request"
    );
    let result = f
        .store
        .settle_abandoned_transition(
            &current.credential_id,
            &record.record_id,
            "repair",
            "probe",
            EvidenceSource::ProviderStatus,
            &[settlement.clone()],
            ReconcileOutcome::Cancelled,
        )
        .unwrap();
    assert_eq!(result.command_type, "operation.cleanup");
    assert_eq!(
        result.result.as_ref().unwrap()["state_at_decision"],
        "cancelled"
    );
    assert_eq!(result.causal_event_ids.len(), 1);
    assert_eq!(
        f.store
            .settle_abandoned_transition(
                &current.credential_id,
                &record.record_id,
                "repair",
                "probe",
                EvidenceSource::ProviderStatus,
                &[settlement],
                ReconcileOutcome::Cancelled
            )
            .unwrap(),
        result
    );
    let inspected = f
        .store
        .inspect_transition_for_cleanup(&current.credential_id, &record.record_id)
        .unwrap();
    assert_eq!(inspected.state, OperationState::Cancelled);
    assert_eq!(inspected.effects[0].attempt, 0);
    assert!(inspected
        .effects
        .iter()
        .all(|e| e.state == EffectState::Confirmed));
    let c = f.store.connection().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT owner_id FROM operations WHERE record_id=?1",
            [&record.record_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        f.ids[0]
    );
    assert_eq!(
        c.query_row(
            "SELECT topology_revision FROM sessions WHERE id=?1",
            [&f.sid],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
}
#[test]
fn authority_is_rechecked_after_commit_callback_and_invalid_local_scope_rolls_back() {
    let f = Fixture::new();
    f.admit("one");
    let lease = f.claim("one");
    f.store
        .advance_transition(&f.keys[0], &lease, OperationState::Committing)
        .unwrap();
    let error = f
        .store
        .commit_transition(&f.keys[0], &lease, |c, r| {
            c.execute(
                "UPDATE sessions SET topology_revision=1 WHERE id=?1",
                [&r.session_id],
            )
            .map_err(|_| Error::storage())?;
            c.execute(
                "UPDATE connections SET state='active' WHERE id='conn-carol'",
                [],
            )
            .map_err(|_| Error::storage())?;
            c.execute(
                "UPDATE participants SET active=0 WHERE participant_id=?1",
                [&f.parts[2]],
            )
            .map_err(|_| Error::storage())?;
            Ok(f.completion())
        })
        .unwrap_err();
    assert_eq!(error.code, "forbidden");
    let c = f.store.connection().unwrap();
    assert!(c
        .query_row(
            "SELECT active FROM participants WHERE participant_id=?1",
            [&f.parts[2]],
            |r| r.get::<_, bool>(0)
        )
        .unwrap());
    assert_eq!(
        c.query_row(
            "SELECT topology_revision FROM sessions WHERE id=?1",
            [&f.sid],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
}
#[test]
fn credential_expiry_while_waiting_for_writer_does_not_admit_intent_or_a_receipt() {
    let f = Fixture::new();
    let expiry = crate::now() + 1;
    let short = f.store.issue(&f.ids[0], Scope::operator(), expiry).unwrap();
    let c = rusqlite::Connection::open(f.dir.path().join("state.sqlite3")).unwrap();
    c.execute_batch("BEGIN IMMEDIATE;").unwrap();
    let store = f.store.clone();
    let request = f.request("expires-under-lock");
    let key = short.id.clone();
    let worker = std::thread::spawn(move || store.admit_transition(&key, request));
    std::thread::sleep(std::time::Duration::from_millis(1100));
    c.execute_batch("COMMIT;").unwrap();
    assert_eq!(worker.join().unwrap().unwrap_err().code, "unauthorized");
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM operations WHERE operation_id='expires-under-lock'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
}
#[test]
fn one_principal_business_id_cannot_execute_in_both_core_receipt_families() {
    let f = Fixture::new();
    f.store
        .execute(
            &f.keys[0],
            &Operation::Send {
                request_id: "message-key".into(),
                conversation_id: f.cid.clone(),
                message: connect_protocol::messages::MessageDraft::text_all("existing mutation"),
            },
        )
        .unwrap();
    assert_eq!(
        f.store
            .admit_transition(&f.keys[0], f.request("message-key"))
            .unwrap_err()
            .code,
        "conflict"
    );
    f.admit("transition-key");
    assert_eq!(
        f.store
            .execute(
                &f.keys[0],
                &Operation::Send {
                    request_id: "transition-key".into(),
                    conversation_id: f.cid.clone(),
                    message: connect_protocol::messages::MessageDraft::text_all("must not execute")
                }
            )
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(
        f.store
            .create_identity_as(
                &f.store
                    .authenticate(
                        &f.store
                            .issue(&f.ids[0], Scope::operator(), crate::now() + 600)
                            .unwrap()
                            .token
                    )
                    .unwrap(),
                "transition-key",
                "must-not-create"
            )
            .unwrap_err()
            .code,
        "conflict"
    );
    let c = f.store.connection().unwrap();
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM messages", [], |r| r.get::<_, u64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM operations WHERE operation_id='message-key'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
}
#[test]
fn full_journal_cannot_keep_an_old_worker_authorized_and_explicit_repair_uses_terminal_room() {
    let mut f = Fixture::new();
    let record = f.admit("one");
    let old = f.claim("one");
    let events: u64 = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT (SELECT COUNT(*) FROM journal)+(SELECT COUNT(*) FROM task_events)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let limits = Limits {
        journal_events: events + 1,
        ..Limits::default()
    };
    let tiny = Store::open(&f.dir.path().join("state.sqlite3"), limits.clone()).unwrap();
    assert_eq!(tiny.fence_transition_workers(1000).unwrap(), 1);
    assert_eq!(
        tiny.recover_transition_work(1000).unwrap_err().code,
        "capacity-exceeded"
    );
    assert_eq!(
        f.store
            .advance_transition(&f.keys[0], &old, OperationState::Ready)
            .unwrap_err()
            .code,
        "forbidden"
    );
    drop(tiny);
    assert_eq!(
        Store::open(&f.dir.path().join("state.sqlite3"), limits.clone())
            .err()
            .unwrap()
            .code,
        "capacity-exceeded"
    );
    let admin = Store::open_administration(f.store.path(), Limits::default()).unwrap();
    f.store = Arc::new(admin); // The simulated runtime is stopped before offline repair.
    let repair = Store::open_for_recovery(&f.dir.path().join("state.sqlite3"), limits).unwrap();
    let decision = repair
        .settle_abandoned_transition(
            &f.keys[0],
            &record.record_id,
            "repair-full-journal",
            "local-unsubmitted-proof",
            EvidenceSource::LocalState,
            &[],
            ReconcileOutcome::Cancelled,
        )
        .unwrap();
    assert_eq!(
        decision.result.as_ref().unwrap()["state_at_decision"],
        "cancelled"
    );
    assert_eq!(decision.causal_event_ids.len(), 1);
    assert_eq!(
        f.store
            .advance_transition(&f.keys[0], &old, OperationState::Committing)
            .unwrap_err()
            .code,
        "forbidden"
    );
}
#[test]
fn conversation_role_revision_race_prevents_an_incompatible_handoff_commit() {
    use connect_protocol::membership::*;
    let f = Fixture::new();
    let revision = f
        .store
        .membership_snapshot(&f.keys[0], &f.cid)
        .unwrap()
        .revision;
    let mut request = f.request("handoff");
    request.connection_id = Some("conn-alice".into());
    request.intent = TransitionIntent::Handoff {
        source_participant: f.parts[0].clone(),
        target_participant: f.parts[2].clone(),
        retained_connid: "conn-bob".into(),
        candidate_connid: Some("conn-carol".into()),
        outgoing: OutgoingDisposition::Leave,
        role_scope: RoleScope::Conversation,
        expected_conversation_revision: Some(revision),
    };
    f.store.admit_transition(&f.keys[0], request).unwrap();
    let lease = f.claim("handoff");
    f.store
        .execute_membership(
            &f.keys[0],
            MembershipRequest {
                operation_id: "competing-role".into(),
                conversation_id: f.cid.clone(),
                expected_revision: revision,
                change: MembershipAction::UpdateRole {
                    participant_id: f.parts[2].clone(),
                    role: "supervisor".into(),
                },
            },
        )
        .unwrap();
    assert_eq!(
        f.store
            .advance_transition(&f.keys[0], &lease, OperationState::Committing)
            .unwrap_err()
            .code,
        "conflict"
    );
    let c = f.store.connection().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT role FROM participants WHERE participant_id=?1",
            [&f.parts[2]],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "supervisor"
    );
    assert_eq!(
        c.query_row(
            "SELECT topology_revision FROM sessions WHERE id=?1",
            [&f.sid],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
}
#[test]
fn verified_uncertain_graph_commit_can_reconcile_local_decision_and_receipt_atomically() {
    let f = Fixture::new();
    f.admit("one");
    let lease = f.claim("one");
    let effect = f.effect(&lease, "graph-commit");
    f.observe("one", &effect, "graph-confirmed", EffectState::Confirmed);
    f.store
        .advance_transition(&f.keys[0], &lease, OperationState::Committing)
        .unwrap();
    f.store
        .advance_transition(&f.keys[0], &lease, OperationState::Unknown)
        .unwrap();
    let proof = f
        .store
        .record_transition_evidence(
            &f.keys[0],
            "one",
            "verified-local-graph",
            EvidenceSource::LocalState,
            ReconcileOutcome::Completed,
            Some(f.completion()),
        )
        .unwrap();
    // A public receipt cannot merely assert completion while local role state is old.
    assert_eq!(
        f.store
            .execute_operation_action(
                &f.keys[0],
                OperationAction::Reconcile {
                    operation_id: "reconcile".into(),
                    target_operation_id: "one".into(),
                    outcome: ReconcileOutcome::Completed,
                    evidence_id: proof.clone()
                }
            )
            .unwrap_err()
            .code,
        "conflict"
    );
    let result = f
        .store
        .reconcile_transition_commit(&f.keys[0], "one", "reconcile", &proof, |c, r| {
            c.execute(
                "UPDATE sessions SET topology_revision=1 WHERE id=?1",
                [&r.session_id],
            )
            .map_err(|_| Error::storage())?;
            c.execute(
                "UPDATE connections SET state='active' WHERE id='conn-carol'",
                [],
            )
            .map_err(|_| Error::storage())?;
            Ok(f.completion())
        })
        .unwrap();
    assert_eq!(
        result.result.as_ref().unwrap()["state_at_decision"],
        "completed"
    );
    assert_eq!(f.get("one").state, OperationState::Completed);
    assert_eq!(
        f.store
            .reconcile_transition_commit(&f.keys[0], "one", "reconcile", &proof, |_, _| panic!(
                "replay must not commit again"
            ))
            .unwrap(),
        result
    );
}

#[test]
fn protected_graph_commit_and_reconciliation_wait_for_current_delivery() {
    for reconcile in [false, true] {
        let f = Fixture::new();
        f.admit("one");
        let lease = f.claim("one");
        f.store
            .advance_transition(&f.keys[0], &lease, OperationState::Ready)
            .unwrap();
        f.store
            .advance_transition(&f.keys[0], &lease, OperationState::Committing)
            .unwrap();
        let proof = if reconcile {
            f.store
                .advance_transition(&f.keys[0], &lease, OperationState::Unknown)
                .unwrap();
            Some(
                f.store
                    .record_transition_evidence(
                        &f.keys[0],
                        "one",
                        "local-proof",
                        EvidenceSource::LocalState,
                        ReconcileOutcome::Completed,
                        Some(f.completion()),
                    )
                    .unwrap(),
            )
        } else {
            None
        };
        let read = f
            .store
            .read_authority(
                &f.keys[0],
                Some(&f.cid),
                &crate::AuthorityWait::new(std::time::Duration::from_secs(1)),
            )
            .unwrap();
        let before = Store::inspect_snapshot(f.store.path()).unwrap();
        std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                if let Some(proof) = proof.as_ref() {
                    f.store.reconcile_transition_commit(
                        &f.keys[0],
                        "one",
                        "reconcile",
                        proof,
                        |c, r| {
                            c.execute(
                                "UPDATE sessions SET topology_revision=1 WHERE id=?1",
                                [&r.session_id],
                            )
                            .map_err(|_| Error::storage())?;
                            c.execute(
                                "UPDATE connections SET state='active' WHERE id='conn-carol'",
                                [],
                            )
                            .map_err(|_| Error::storage())?;
                            Ok(f.completion())
                        },
                    )
                } else {
                    f.commit(&lease)
                }
            });
            crate::authority_fences::test_wait_for_writer(
                &f.store,
                &crate::AuthorityScope::identity(
                    f.ids.iter().min().unwrap(),
                    crate::AuthorityMode::Write,
                )
                .unwrap(),
            );
            assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
            drop(read);
            assert_eq!(
                worker.join().unwrap().unwrap().state,
                OperationState::Completed
            );
        });
        assert_eq!(f.get("one").result.unwrap()["session_revision"], 1);
    }
}
