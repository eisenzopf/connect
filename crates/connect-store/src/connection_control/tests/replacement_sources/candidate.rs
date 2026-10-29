//! Actual Store admission/retirement APIs with controlled absence observations.
//! No Core ticket, socket or physical media qualification is inferred here.
use super::*;
use crate::{
    NativeMediaOrigin, NativeOfferLease, NativeOfferObservation, NativeOfferRuntimeObservation,
    NativeReplacementCandidate, NativeReplacementOrigin, NativeReplacementPrepared,
    PeerAuthorityGuard, PeerAuthorityRequest,
};
use std::sync::Arc;

pub(super) struct Pending {
    pub f: Fixture,
    pub key: String,
    pub command: TransitionCommand,
    pub origin: NativeReplacementOrigin,
    pub sources: Vec<Arc<NativeMediaOrigin>>,
    pub request: PeerAuthorityRequest,
    pub proof: Option<PeerAuthorityGuard>,
    pub original: NativeOfferLease,
}
pub(super) struct Absent(pub String, pub u64);
impl NativeOfferRuntimeObservation for Absent {
    fn observe(&self) -> connect_protocol::Result<NativeOfferObservation> {
        Ok(NativeOfferObservation::Retired {
            runtime_connection_id: self.0.clone(),
            generation: self.1,
        })
    }
}
pub(super) fn pending() -> Pending {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use ring::signature::{Ed25519KeyPair, KeyPair};
    let (f, key, target, mut command, sources) = fixture_with_sources();
    let identity = f.store.revalidate(&f.keys[0]).unwrap().identity_id;
    let target_key = f
        .store
        .issue(
            &identity,
            Scope::collaboration(vec!["*".into()]),
            crate::now() + 600,
        )
        .unwrap();
    let bytes = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    let pair = Ed25519KeyPair::from_pkcs8(bytes.as_ref()).unwrap();
    f.store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::SigningKey {
                operation_id: "candidate-target-key".into(),
                identity_id: identity,
                credential_id: target_key.id.clone(),
                device_id: "device-replacement".into(),
                key_id: "candidate-key".into(),
                algorithm: SigningAlgorithm::EdDSA,
                public_key: URL_SAFE_NO_PAD.encode(pair.public_key().as_ref()),
                not_before: crate::now() - 1,
                expires_at: crate::now() + 600,
                expected_revision: None,
            },
        )
        .unwrap();
    let mut intent = f.request(0, "candidate").intent;
    intent.connection_id = "replacement-next".into();
    let request = PeerAuthorityRequest {
        credentials: vec![target_key.id],
        device: crate::PeerDeviceClaim {
            id: "device-replacement".into(),
            kind: "desktop".into(),
            platform: "isolated-store-fixture".into(),
        },
        key_id: "candidate-key".into(),
        algorithm: "EdDSA".into(),
        command: crate::PeerCommand::NativeConnectionOffer(intent),
    };
    let proof = f.admit(&request);
    f.store
        .prepare_native_offer_with_peer(
            &request.credentials[0],
            &proof,
            Some(EndpointTransport::Quic),
        )
        .unwrap();
    let (original, _) = f
        .store
        .begin_native_offer_with_peer(&request.credentials[0], &proof)
        .unwrap()
        .unwrap();
    command.intent = TransitionIntent::Replace {
        endpoint_id: None,
        candidate_connid: Some("replacement-next".into()),
        retained_connid: sources[1].connection_id().into(),
    };
    // The original peer still holds its invocation reader. Admission must not
    // wait for that reader to finish an unanswered parameter callback.
    f.store
        .admit_transition_command(&key, command.clone())
        .unwrap();
    let origin = f
        .store
        .native_replacement_origin(&key, command.clone())
        .unwrap();
    assert_eq!(
        f.store
            .native_replacement_context(&origin)
            .unwrap()
            .target_endpoint_id,
        target
    );
    Pending {
        f,
        key,
        command,
        origin,
        sources,
        request,
        proof: Some(proof),
        original,
    }
}
impl Pending {
    pub fn claim(&self) -> NativeReplacementCandidate {
        let read = self
            .f
            .store
            .try_native_replacement_preparation_read(
                &self.origin,
                &crate::NativeReplacementSourceOrigin::Native(self.sources[0].clone()),
                &self.sources[1],
            )
            .unwrap();
        self.f
            .store
            .claim_native_replacement_candidate(read)
            .unwrap()
    }
    pub fn absent(&self) -> Absent {
        Absent(
            self.original.runtime_connection_id().into(),
            self.original.generation(),
        )
    }
    pub fn settle_original(&mut self) {
        drop(self.proof.take());
        self.f
            .store
            .settle_native_offer_cleanup(&self.original, &self.absent())
            .unwrap();
    }
}
pub(super) fn prepared_scene() -> (
    Fixture,
    String,
    TransitionCommand,
    NativeReplacementPrepared,
    PeerAuthorityRequest,
    Vec<Arc<NativeMediaOrigin>>,
    NativeOfferLease,
) {
    let mut p = pending();
    let claim = p.claim();
    let attempt =
        p.f.store
            .begin_native_candidate_retirement(&claim)
            .unwrap()
            .unwrap();
    p.settle_original();
    p.f.store
        .complete_native_candidate_retirement(&attempt, &p.absent())
        .unwrap();
    let lease = custody::prepare(&p.f, &p.origin, &p.sources);
    let plan = lease.plan().unwrap();
    assert_ne!(
        plan["candidate_runtime_id"],
        p.original.runtime_connection_id()
    );
    let receipt = custody::begin(&p.f, lease)
        .prepared(plan["expected_preparation"].clone())
        .unwrap();
    p.f.store
        .record_native_replacement_prepared(&receipt)
        .unwrap();
    let crate::PeerCommand::NativeConnectionOffer(intent) = &mut p.request.command else {
        panic!()
    };
    intent.operation_id = p.command.operation_id.clone();
    intent.offer["replacement_preparation"] = json!({"profile":"connect-replacement-preparation-v1","operation_record_id":p.origin.record_id()});
    (
        p.f, p.key, p.command, receipt, p.request, p.sources, p.original,
    )
}
#[test]
fn pending_candidate_claim_preserves_original_receipt_and_requires_attributed_retirement_before_fresh_ticket(
) {
    let mut p = pending();
    let claim = p.claim();
    assert_eq!(
        claim.runtime_connection_id(),
        p.original.runtime_connection_id()
    );
    assert_eq!(claim.generation(), 1);
    let snapshot = Store::inspect_snapshot(p.f.store.path()).unwrap();
    let again = p.claim();
    assert_eq!(again.runtime_connection_id(), claim.runtime_connection_id());
    assert_eq!(Store::inspect_snapshot(p.f.store.path()).unwrap(), snapshot);
    assert!(p
        .f
        .store
        .claim_transition(&p.key, &p.command.operation_id, 30)
        .unwrap()
        .is_none());
    let attempt =
        p.f.store
            .begin_native_candidate_retirement(&claim)
            .unwrap()
            .unwrap();
    let snapshot = Store::inspect_snapshot(p.f.store.path()).unwrap();
    assert!(p
        .f
        .store
        .complete_native_candidate_retirement(&attempt, &Absent("foreign-core".into(), 1))
        .is_err());
    assert!(p
        .f
        .store
        .complete_native_candidate_retirement(&attempt, &p.absent())
        .is_err());
    assert_eq!(Store::inspect_snapshot(p.f.store.path()).unwrap(), snapshot);
    p.settle_original();
    p.f.store
        .complete_native_candidate_retirement(&attempt, &p.absent())
        .unwrap();
    let snapshot = Store::inspect_snapshot(p.f.store.path()).unwrap();
    p.f.store
        .complete_native_candidate_retirement(&attempt, &p.absent())
        .unwrap();
    assert_eq!(Store::inspect_snapshot(p.f.store.path()).unwrap(), snapshot);
    let lease = custody::prepare(&p.f, &p.origin, &p.sources);
    let plan = lease.plan().unwrap();
    assert_eq!(plan["candidate_connection_id"], "replacement-next");
    assert_ne!(
        plan["candidate_runtime_id"],
        p.original.runtime_connection_id()
    );
    assert_eq!(plan["generation"], 2);
    let c = p.f.store.connection().unwrap();
    let still_not_ready:bool=c.query_row("SELECT state='retired' AND generation=2 AND observed_runtime_id IS NULL AND readiness_json IS NULL FROM connections WHERE id='replacement-next'",[],|r|r.get(0)).unwrap();
    assert!(still_not_ready);
}
#[test]
fn known_candidate_retirement_metadata_survives_revocation_without_granting_fresh_preparation() {
    let mut p = pending();
    let claim = p.claim();
    let attempt =
        p.f.store
            .begin_native_candidate_retirement(&claim)
            .unwrap()
            .unwrap();
    p.settle_original();
    let admin = p.f.store.revalidate(&p.f.admin).unwrap();
    p.f.store.revoke(&admin, &p.key).unwrap();
    p.f.store
        .complete_native_candidate_retirement(&attempt, &p.absent())
        .unwrap();
    assert!(p
        .f
        .store
        .try_native_replacement_preparation_read(
            &p.origin,
            &crate::NativeReplacementSourceOrigin::Native(p.sources[0].clone()),
            &p.sources[1]
        )
        .is_err());
    assert_eq!(p.f.count("native_replacements"), 0);
    assert!(p
        .f
        .store
        .fail_native_replacement_preflight(&p.key, p.command.clone(), "forbidden")
        .unwrap());
    let failed =
        p.f.store
            .execute_operation_action(
                &p.f.admin,
                OperationAction::Get {
                    operation_id: p.command.operation_id.clone(),
                },
            )
            .unwrap();
    assert_eq!(
        failed.state,
        connect_protocol::transitions::OperationState::Failed
    );
    assert!(failed.result.is_none());
}
#[test]
fn candidate_retirement_sql_contention_preserves_original_attempt_and_allows_metadata_only_retry() {
    let mut p = pending();
    let claim = p.claim();
    let attempt =
        p.f.store
            .begin_native_candidate_retirement(&claim)
            .unwrap()
            .unwrap();
    p.settle_original();
    let writer = rusqlite::Connection::open(p.f.store.path()).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    let started = std::time::Instant::now();
    assert_eq!(
        p.f.store
            .complete_native_candidate_retirement(&attempt, &p.absent())
            .unwrap_err()
            .code,
        "authority-busy"
    );
    assert!(started.elapsed() < std::time::Duration::from_millis(500));
    writer.execute_batch("ROLLBACK").unwrap();
    p.f.store
        .complete_native_candidate_retirement(&attempt, &p.absent())
        .unwrap();
    assert!(p
        .f
        .store
        .begin_native_candidate_retirement(&claim)
        .unwrap()
        .is_none());
}

#[test]
fn fresh_candidate_reservation_never_holds_authority_writers_waiting_for_sql() {
    let mut p = pending();
    let claim = p.claim();
    let attempt =
        p.f.store
            .begin_native_candidate_retirement(&claim)
            .unwrap()
            .unwrap();
    p.settle_original();
    p.f.store
        .complete_native_candidate_retirement(&attempt, &p.absent())
        .unwrap();
    let before = Store::inspect_snapshot(p.f.store.path()).unwrap();
    let writer = rusqlite::Connection::open(p.f.store.path()).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    let read =
        p.f.store
            .try_native_replacement_preparation_read(
                &p.origin,
                &crate::NativeReplacementSourceOrigin::Native(p.sources[0].clone()),
                &p.sources[1],
            )
            .unwrap();
    let started = std::time::Instant::now();
    assert_eq!(
        p.f.store
            .prepare_native_replacement(read, custody::request())
            .unwrap_err()
            .code,
        "authority-busy"
    );
    assert!(started.elapsed() < std::time::Duration::from_millis(500));
    // The failed SQL admission returned every authority writer immediately.
    let identity = p.f.store.revalidate(&p.key).unwrap().identity_id;
    let grant =
        p.f.store
            .try_acquire_authority(&[crate::AuthorityScope::identity(
                &identity,
                crate::AuthorityMode::Read,
            )
            .unwrap()])
            .unwrap();
    drop(grant);
    writer.execute_batch("ROLLBACK").unwrap();
    assert_eq!(Store::inspect_snapshot(p.f.store.path()).unwrap(), before);
    let lease = custody::prepare(&p.f, &p.origin, &p.sources);
    assert_ne!(
        lease.plan().unwrap()["candidate_runtime_id"],
        p.original.runtime_connection_id()
    );
}

#[test]
fn ordinary_ready_binding_cannot_be_claimed_as_private_replacement_preparation() {
    let mut p = pending();
    drop(p.proof.take());
    // Controlled ordinary readiness, as in the shared Store source fixture.
    // It supplies no Core ticket and must never be accepted as a private one.
    {
        let mut c = p.f.store.connection().unwrap();
        let tx = c.transaction().unwrap();
        tx.execute(
            "UPDATE operation_effects SET state='confirmed' WHERE id=?1",
            [p.original.effect_id()],
        )
        .unwrap();
        tx.execute(
            "UPDATE operations SET state='completed',revision=revision+1 WHERE record_id=?1",
            [p.original.record_id()],
        )
        .unwrap();
        tx.execute("UPDATE connections SET state='ready',revision=1,observed_runtime_id=?2,readiness_json=?3 WHERE id=?1",params![p.original.connection_id(),p.f.store.runtime_epoch(),json!({"streams":["audio"],"stream_revision":1,"participant_revision":1}).to_string()]).unwrap();
        tx.execute("INSERT INTO native_connection_lifetimes(operation_record_id,connection_id,runtime_epoch,generation) VALUES(?1,?2,?3,1)",params![p.original.record_id(),p.original.connection_id(),p.f.store.runtime_epoch()]).unwrap();
        tx.commit().unwrap();
    }
    let read =
        p.f.store
            .try_native_replacement_preparation_read(
                &p.origin,
                &crate::NativeReplacementSourceOrigin::Native(p.sources[0].clone()),
                &p.sources[1],
            )
            .unwrap();
    let before = Store::inspect_snapshot(p.f.store.path()).unwrap();
    assert_eq!(
        p.f.store
            .claim_native_replacement_candidate(read)
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(Store::inspect_snapshot(p.f.store.path()).unwrap(), before);
    assert_eq!(p.f.count("native_replacement_candidates"), 0);
    assert_eq!(p.f.count("native_replacements"), 0);
}

#[test]
fn candidate_claim_crash_recovery_settles_only_original_absence_and_never_prepares_a_ticket() {
    for begun in [false, true] {
        let mut p = pending();
        let claim = p.claim();
        let _attempt = begun.then(|| {
            p.f.store
                .begin_native_candidate_retirement(&claim)
                .unwrap()
                .unwrap()
        });
        drop(p.proof.take());
        let path = p.f.store.path().to_owned();
        drop(p.f.store);
        let successor = Store::open(&path, connect_protocol::Limits::default()).unwrap();
        struct Recovery;
        impl crate::NativeOfferRecoveryObservation for Recovery {
            fn observe_retired(
                &self,
                id: &str,
                generation: u64,
            ) -> connect_protocol::Result<NativeOfferObservation> {
                Ok(NativeOfferObservation::Retired {
                    runtime_connection_id: id.into(),
                    generation,
                })
            }
        }
        successor.recover_native_offer_cleanup(&Recovery).unwrap();
        assert_eq!(
            successor
                .recover_native_candidate_claims(&Recovery)
                .unwrap(),
            1
        );
        assert_eq!(
            successor
                .recover_native_candidate_claims(&Recovery)
                .unwrap(),
            0
        );
        let record = successor
            .execute_operation_action(
                &p.key,
                OperationAction::Get {
                    operation_id: p.command.operation_id.clone(),
                },
            )
            .unwrap();
        assert_eq!(
            record.state,
            connect_protocol::transitions::OperationState::Failed
        );
        assert!(record.result.is_none());
        assert_eq!(record.effects.len(), 1);
        assert_eq!(record.effects[0].attempt, u64::from(begun));
        assert_eq!(record.effects[0].submission_generation, u64::from(begun));
        assert_eq!(
            record.effects[0].state,
            if begun {
                connect_protocol::transitions::EffectState::Confirmed
            } else {
                connect_protocol::transitions::EffectState::Cancelled
            }
        );
        let c = successor.connection().unwrap();
        assert_eq!(
            c.query_row("SELECT COUNT(*) FROM native_replacements", [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn candidate_retirement_cancel_finishes_only_after_known_original_cleanup_without_a_ticket() {
    let mut p = pending();
    let claim = p.claim();
    let attempt =
        p.f.store
            .begin_native_candidate_retirement(&claim)
            .unwrap()
            .unwrap();
    custody::cancel(&p.f, &p.key, &p.command);
    let current =
        p.f.store
            .execute_operation_action(
                &p.key,
                OperationAction::Get {
                    operation_id: p.command.operation_id.clone(),
                },
            )
            .unwrap();
    assert_eq!(
        current.state,
        connect_protocol::transitions::OperationState::Unknown
    );
    p.settle_original();
    p.f.store
        .complete_native_candidate_retirement(&attempt, &p.absent())
        .unwrap();
    let current =
        p.f.store
            .execute_operation_action(
                &p.key,
                OperationAction::Get {
                    operation_id: p.command.operation_id.clone(),
                },
            )
            .unwrap();
    assert_eq!(
        current.state,
        connect_protocol::transitions::OperationState::Cancelled
    );
    assert!(current.result.is_none());
    assert_eq!(p.f.count("native_replacements"), 0);
}

#[test]
fn candidate_retirement_uncertainty_cannot_accept_operator_asserted_replacement_completion() {
    let p = pending();
    let claim = p.claim();
    let attempt =
        p.f.store
            .begin_native_candidate_retirement(&claim)
            .unwrap()
            .unwrap();
    p.f.store
        .native_candidate_retirement_uncertain(&attempt)
        .unwrap();
    let before = Store::inspect_snapshot(p.f.store.path()).unwrap();
    let error =
        p.f.store
            .execute_operation_action(
                &p.f.admin,
                OperationAction::Reconcile {
                    operation_id: "asserted-candidate-completion".into(),
                    target_operation_id: p.command.operation_id.clone(),
                    outcome: connect_protocol::transitions::ReconcileOutcome::Completed,
                    evidence_id: "operator-assertion".into(),
                },
            )
            .unwrap_err();
    assert_eq!(error.code, "reconciliation-required");
    assert_eq!(Store::inspect_snapshot(p.f.store.path()).unwrap(), before);
    assert_eq!(p.f.count("native_replacements"), 0);
}
