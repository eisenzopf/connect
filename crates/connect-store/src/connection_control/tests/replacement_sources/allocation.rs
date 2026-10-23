//! Store phase tests use controlled captures; no actual target allocation/IO.
use super::custody::*;
use super::*;
use crate::{NativeReplacementPrepared, PeerAuthorityRequest};
use connect_protocol::transitions::{EffectState, OperationState};

pub(super) fn prepared_target() -> (
    Fixture,
    String,
    TransitionCommand,
    NativeReplacementPrepared,
    PeerAuthorityRequest,
) {
    let (f, key, command, receipt, request, _) = prepared_target_scene();
    (f, key, command, receipt, request)
}
pub(super) fn prepared_target_scene() -> (
    Fixture,
    String,
    TransitionCommand,
    NativeReplacementPrepared,
    PeerAuthorityRequest,
    Vec<std::sync::Arc<crate::NativeMediaOrigin>>,
) {
    prepared_target_scene_scoped(&[])
}
pub(super) fn prepared_target_scene_scoped(
    extra_operations: &[&str],
) -> (
    Fixture,
    String,
    TransitionCommand,
    NativeReplacementPrepared,
    PeerAuthorityRequest,
    Vec<std::sync::Arc<crate::NativeMediaOrigin>>,
) {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use ring::signature::{Ed25519KeyPair, KeyPair};
    let (f, key, command, origin, sources) = setup();
    let lease = prepare(&f, &origin, &sources);
    let plan = lease.plan().unwrap();
    let observation = plan["expected_preparation"].clone();
    let receipt = begin(&f, lease).prepared(observation).unwrap();
    f.store
        .record_native_replacement_prepared(&receipt)
        .unwrap();
    let identity = f.store.revalidate(&f.keys[0]).unwrap().identity_id;
    let mut target_scope = Scope::collaboration(vec!["*".into()]);
    target_scope
        .operations
        .extend(extra_operations.iter().map(|s| (*s).to_owned()));
    let target = f
        .store
        .issue(&identity, target_scope, crate::now() + 600)
        .unwrap();
    let bytes = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    let pair = Ed25519KeyPair::from_pkcs8(bytes.as_ref()).unwrap();
    f.store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::SigningKey {
                operation_id: "replacement-target-key".into(),
                identity_id: identity,
                credential_id: target.id.clone(),
                device_id: "device-replacement".into(),
                key_id: "replacement-key".into(),
                algorithm: SigningAlgorithm::EdDSA,
                public_key: URL_SAFE_NO_PAD.encode(pair.public_key().as_ref()),
                not_before: crate::now() - 1,
                expires_at: crate::now() + 600,
                expected_revision: None,
            },
        )
        .unwrap();
    let mut intent = f.request(0, "replacement-target").intent;
    intent.operation_id = command.operation_id.clone();
    intent.connection_id = plan["candidate_connection_id"].as_str().unwrap().into();
    intent.offer["replacement_preparation"] = json!({"profile":"connect-replacement-preparation-v1","operation_record_id":origin.record_id()});
    let request = PeerAuthorityRequest {
        credentials: vec![target.id],
        device: crate::PeerDeviceClaim {
            id: "device-replacement".into(),
            kind: "desktop".into(),
            platform: "isolated-store-fixture".into(),
        },
        key_id: "replacement-key".into(),
        algorithm: "EdDSA".into(),
        command: crate::PeerCommand::NativeConnectionOffer(intent),
    };
    (f, key, command, receipt, request, sources)
}

#[test]
fn replacement_allocation_begins_once_under_independent_target_and_original_grants_without_io() {
    let (f, key, command, receipt, request) = prepared_target();
    let proof = f.admit(&request);
    let target = request.credentials.last().unwrap();
    let (attempt, read) = f
        .store
        .begin_native_replacement_allocation_with_peer(&receipt, target, &proof)
        .unwrap();
    assert!(read.current());
    assert_ne!(target, &key);
    assert_eq!(read.target().actor.participant_id, f.pids[0]);
    assert_eq!(read.source().context().source_generation, 1);
    assert_eq!(
        read.target_intent().unwrap().connection_id,
        "replacement-next"
    );
    assert_eq!(f.count("connections"), 2);
    assert_eq!(f.count("native_replacement_steps"), 2);
    assert_eq!(
        f.store
            .begin_native_replacement_allocation_with_peer(&receipt, target, &proof)
            .unwrap_err()
            .code,
        "reconciliation-required"
    );
    let expected = attempt.expected_allocation().clone();
    let captured = attempt.allocated(expected).unwrap();
    drop(read);
    drop(proof);
    assert!(f
        .store
        .check_native_replacement_allocated_target(&captured)
        .is_ok());
    let actor = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&actor, target).unwrap();
    assert!(f
        .store
        .check_native_replacement_allocated_target(&captured)
        .is_err());
    cancel(&f, &key, &command);
    let record = f
        .store
        .record_native_replacement_allocated(&captured)
        .unwrap();
    assert_eq!(record.state, OperationState::Unknown);
    assert!(record.result.is_none());
    assert_eq!(record.effects.len(), 2);
    assert!(record
        .effects
        .iter()
        .all(|e| e.state == EffectState::Confirmed && e.attempt == 1));
    let journal = f.count("journal");
    assert_eq!(
        f.store
            .record_native_replacement_allocated(&captured)
            .unwrap(),
        record
    );
    assert_eq!(f.count("journal"), journal);
    assert_eq!(
        f.store
            .record_native_replacement_preparation_stopped(&receipt)
            .unwrap_err()
            .code,
        "reconciliation-required"
    );
    let expected = serde_json::from_str::<Value>(
        &f.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT observation_json FROM native_replacement_steps WHERE phase='allocate'",
                [],
                |r| r.get::<_, String>(0),
            )
            .unwrap(),
    )
    .unwrap();
    let mut absent = Gone {
        runtime: expected["candidate"].as_str().unwrap().into(),
        generation: 1,
    };
    assert!(f
        .store
        .record_native_replacement_allocation_stopped(&captured, &absent)
        .is_err());
    absent.generation = 2;
    let stopped = f
        .store
        .record_native_replacement_allocation_stopped(&captured, &absent)
        .unwrap();
    assert_eq!(stopped.state, OperationState::Cancelled);
    assert_eq!(stopped.effects.len(), 4);
    assert!(stopped.result.is_none());
    let journal = f.count("journal");
    assert_eq!(
        f.store
            .record_native_replacement_allocation_stopped(&captured, &absent)
            .unwrap(),
        stopped
    );
    assert_eq!(f.count("journal"), journal);
    let foreign = Store::open_administration(f.store.path(), Limits::default()).unwrap();
    assert!(foreign
        .record_native_replacement_allocated(&captured)
        .is_err());
}

#[test]
fn replacement_allocation_rejects_forged_target_binding_and_does_not_start_an_effect() {
    for change in [
        "root",
        "operation",
        "candidate",
        "session",
        "participant",
        "marker-extra",
        "profile",
        "private",
        "handoff",
        "direction",
        "foreign-key",
    ] {
        let (f, _, _, receipt, mut request) = prepared_target();
        let crate::PeerCommand::NativeConnectionOffer(intent) = &mut request.command else {
            panic!()
        };
        match change {
            "operation" => intent.operation_id = "another-operation".into(),
            "root" => {
                intent.offer["replacement_preparation"]["operation_record_id"] = json!("foreign")
            }
            "candidate" => intent.connection_id = "foreign-candidate".into(),
            "session" => intent.session_id = "foreign-session".into(),
            "participant" => intent.by_participant = Some(f.pids[1].clone()),
            "marker-extra" => {
                intent.offer["replacement_preparation"]["asserted_ready"] = json!(true)
            }
            "profile" => intent.offer["replacement_preparation"]["profile"] = json!("unknown"),
            "private" => intent.offer["private_consultation"] = json!({}),
            "handoff" => intent.offer["handoff_preparation"] = json!({}),
            "direction" => intent.offer["streams_offered"][0]["direction"] = json!("sendonly"),
            "foreign-key" => {
                request.credentials = vec![f.keys[0].clone()];
                request.device.id = "device-0".into();
                request.key_id = "native-key-0".into();
            }
            _ => unreachable!(),
        }
        if let Ok(proof) = f.store.admit_peer_authority(
            &request,
            &crate::AuthorityWait::new(std::time::Duration::from_secs(1)),
        ) {
            assert!(
                f.store
                    .begin_native_replacement_allocation_with_peer(
                        &receipt,
                        request.credentials.last().unwrap(),
                        &proof
                    )
                    .is_err(),
                "{change}"
            );
        }
        assert_eq!(f.count("native_replacement_steps"), 1, "{change}");
        assert_eq!(f.count("connections"), 2, "{change}");
    }
}

#[test]
fn replacement_allocation_unknown_cannot_reexecute_or_borrow_preparation_cleanup() {
    let (f, _, _, receipt, request) = prepared_target();
    let proof = f.admit(&request);
    let (attempt, read) = f
        .store
        .begin_native_replacement_allocation_with_peer(&receipt, &request.credentials[0], &proof)
        .unwrap();
    drop(read);
    drop(proof);
    f.store
        .native_replacement_allocation_uncertain(attempt)
        .unwrap();
    let proof = f.admit(&request);
    assert!(f
        .store
        .begin_native_replacement_allocation_with_peer(&receipt, &request.credentials[0], &proof)
        .is_err());
    drop(proof);
    assert_eq!(
        f.store
            .record_native_replacement_preparation_stopped(&receipt)
            .unwrap_err()
            .code,
        "reconciliation-required"
    );
    let (state,effect):(String,String)=f.store.connection().unwrap().query_row("SELECT o.state,e.state FROM native_replacement_steps s JOIN operations o ON o.record_id=s.operation_record_id JOIN operation_effects e ON e.id=s.effect_id WHERE s.phase='allocate'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!((state.as_str(), effect.as_str()), ("unknown", "unknown"));
    assert_eq!(f.count("connections"), 2);
}

struct Gone {
    runtime: String,
    generation: u64,
}
impl crate::NativeOfferRuntimeObservation for Gone {
    fn observe(&self) -> Result<crate::NativeOfferObservation> {
        Ok(crate::NativeOfferObservation::Retired {
            runtime_connection_id: self.runtime.clone(),
            generation: self.generation,
        })
    }
}
#[test]
fn replacement_allocation_cannot_upgrade_a_binding_only_fence_or_borrow_revoked_source() {
    for revoked in [false, true] {
        let (f, _, _, receipt, request) = prepared_target();
        if revoked {
            let admin = f.store.revalidate(&f.admin).unwrap();
            f.store.revoke(&admin, &f.keys[0]).unwrap();
            let proof = f.admit(&request);
            assert!(f
                .store
                .begin_native_replacement_allocation_with_peer(
                    &receipt,
                    &request.credentials[0],
                    &proof
                )
                .is_err());
        } else {
            let mut binding = request.clone();
            binding.command = crate::PeerCommand::Binding;
            let mut proof = f.admit(&binding);
            // Internal negative fixture: even a forged command field cannot
            // turn an Identity-only held fence into the full source interval.
            proof.request.command = request.command.clone();
            assert!(f
                .store
                .begin_native_replacement_allocation_with_peer(
                    &receipt,
                    &request.credentials[0],
                    &proof
                )
                .is_err());
        }
        assert_eq!(f.count("native_replacement_steps"), 1);
        assert_eq!(f.count("connections"), 2);
    }
}

#[test]
fn replacement_allocation_planning_contention_has_no_effect_and_begin_remains_once_only() {
    let (f, _, _, receipt, request) = prepared_target();
    let proof = f.admit(&request);
    let c = f.store.connection().unwrap();
    for _ in 0..3 {
        let now = std::time::Instant::now();
        assert_eq!(
            f.store
                .native_replacement_allocation_read_with_peer(
                    &receipt,
                    &request.credentials[0],
                    &proof
                )
                .unwrap_err()
                .code,
            "authority-busy"
        );
        assert!(now.elapsed() < std::time::Duration::from_millis(100));
    }
    drop(c);
    assert_eq!(f.count("native_replacement_steps"), 1);
    let read = f
        .store
        .native_replacement_allocation_read_with_peer(&receipt, &request.credentials[0], &proof)
        .unwrap();
    let (attempt, read) = f
        .store
        .begin_native_replacement_allocation_with_read(
            &receipt,
            &request.credentials[0],
            &proof,
            read,
        )
        .unwrap();
    drop(read);
    drop(proof);
    f.store
        .native_replacement_allocation_uncertain(attempt)
        .unwrap();
    let proof = f.admit(&request);
    assert!(f
        .store
        .native_replacement_allocation_read_with_peer(&receipt, &request.credentials[0], &proof)
        .is_err());
    assert_eq!(f.count("native_replacement_steps"), 2);
    assert_eq!(f.count("connections"), 2);
}
