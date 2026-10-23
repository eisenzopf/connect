//! Controlled resource captures only; actual signed negotiation is a server test.
use super::*;
use super::{allocation::prepared_target, custody::cancel};
use connect_protocol::transitions::{EffectState, OperationState};
use std::sync::Arc;

fn fixture() -> (
    Fixture,
    String,
    TransitionCommand,
    Arc<crate::NativeReplacementAllocated>,
    crate::PeerAuthorityRequest,
) {
    let (f, key, command, prepared, mut request) = prepared_target();
    let proof = f.admit(&request);
    let (attempt, read) = f
        .store
        .begin_native_replacement_allocation_with_peer(&prepared, &request.credentials[0], &proof)
        .unwrap();
    let expected = attempt.expected_allocation().clone();
    let captured = Arc::new(attempt.allocated(expected).unwrap());
    drop(read);
    drop(proof);
    f.store
        .record_native_replacement_allocated(&captured)
        .unwrap();
    let crate::PeerCommand::NativeConnectionOffer(intent) = &mut request.command else {
        panic!()
    };
    intent.offer["replacement_preparation"]["profile"] = json!("connect-replacement-media-v1");
    (f, key, command, captured, request)
}
#[test]
fn replacement_start_has_separate_once_only_effect_and_fresh_provisional_generation_two_reads() {
    let (f, key, command, allocated, request) = fixture();
    let proof = f.admit(&request);
    let read = f
        .store
        .native_replacement_start_read_with_peer(&allocated, &request.credentials[0], &proof)
        .unwrap();
    assert!(read.current());
    assert_eq!(read.target().connection.generation, 2);
    assert_eq!(read.source().context().source_generation, 1);
    assert_eq!(read.target().connection.state, "preparing");
    assert!(read.target().publication_suspended);
    let (attempt, read) = f
        .store
        .begin_native_replacement_start_with_read(
            allocated.clone(),
            &request.credentials[0],
            &proof,
            read,
        )
        .unwrap();
    let repeat = f
        .store
        .native_replacement_start_read_with_peer(&allocated, &request.credentials[0], &proof)
        .unwrap();
    assert_eq!(
        f.store
            .begin_native_replacement_start_with_read(
                allocated,
                &request.credentials[0],
                &proof,
                repeat
            )
            .unwrap_err()
            .code,
        "reconciliation-required"
    );
    let expected = attempt.expected_resources().clone();
    assert_eq!(expected["media_ready"], false);
    assert_eq!(expected["prepared_muted"], true);
    let captured = Arc::new(attempt.started(expected).unwrap());
    drop(read);
    let record = f
        .store
        .record_native_replacement_started_with_peer(&captured, &proof)
        .unwrap();
    assert_eq!(record.state, OperationState::Preparing);
    assert!(record.result.is_none());
    assert_eq!(record.effects.len(), 3);
    assert!(record
        .effects
        .iter()
        .all(|e| e.state == EffectState::Confirmed && e.attempt == 1));
    assert!(f
        .store
        .replay_native_replacement_start_with_peer(&captured, &request.credentials[0], &proof)
        .unwrap()
        .result
        .is_none());
    let origin = f
        .store
        .native_replacement_media_origin(captured.clone())
        .unwrap();
    drop(proof);
    let media = f.store.try_native_replacement_media_read(&origin).unwrap();
    assert!(media.current());
    assert_eq!(media.target().connection.generation, 2);
    assert_eq!(f.count("connections"), 2);
    drop(media);
    let identity = f
        .store
        .revalidate(&request.credentials[0])
        .unwrap()
        .identity_id;
    drop(
        f.store
            .try_acquire_authority(&[crate::AuthorityScope::identity(
                &identity,
                crate::AuthorityMode::Write,
            )
            .unwrap()])
            .unwrap(),
    );
    let journal = f.count("journal");
    assert_eq!(
        f.store
            .record_native_replacement_started(&captured)
            .unwrap(),
        record
    );
    assert_eq!(f.count("journal"), journal);
    cancel(&f, &key, &command);
    let admin = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&admin, &request.credentials[0]).unwrap();
    assert!(f.store.try_native_replacement_media_read(&origin).is_err());
    let frozen = f
        .store
        .record_native_replacement_started(&captured)
        .unwrap();
    assert_eq!(frozen.state, OperationState::Unknown);
    assert!(frozen.result.is_none());
    let absent = Gone {
        runtime: captured_runtime(&f),
        generation: 2,
    };
    let stopped = f
        .store
        .record_native_replacement_started_candidate_stopped(&captured, &absent)
        .unwrap();
    assert_eq!(stopped.state, OperationState::Cancelled);
    assert_eq!(stopped.effects.len(), 6);
    assert_eq!(f.count("connections"), 2);
    let journal = f.count("journal");
    assert_eq!(
        f.store
            .record_native_replacement_started_candidate_stopped(&captured, &absent)
            .unwrap(),
        stopped
    );
    assert_eq!(f.count("journal"), journal);
}
fn captured_runtime(f: &Fixture) -> String {
    f.store
        .connection()
        .unwrap()
        .query_row(
            "SELECT candidate_runtime_id FROM native_replacements",
            [],
            |r| r.get(0),
        )
        .unwrap()
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
fn replacement_start_refuses_changed_parameters_and_unknown_never_repeats() {
    for changed in [false, true] {
        let (f, _, _, allocated, mut request) = fixture();
        if changed {
            let crate::PeerCommand::NativeConnectionOffer(intent) = &mut request.command else {
                panic!()
            };
            intent.offer["streams_offered"][0]["codec_preferences"] = json!(["pcmu"]);
        }
        let proof = f.admit(&request);
        if changed {
            assert!(f
                .store
                .native_replacement_start_read_with_peer(
                    &allocated,
                    &request.credentials[0],
                    &proof
                )
                .is_err());
            assert_eq!(f.count("native_replacement_steps"), 2);
        } else {
            let read = f
                .store
                .native_replacement_start_read_with_peer(
                    &allocated,
                    &request.credentials[0],
                    &proof,
                )
                .unwrap();
            let (attempt, read) = f
                .store
                .begin_native_replacement_start_with_read(
                    allocated.clone(),
                    &request.credentials[0],
                    &proof,
                    read,
                )
                .unwrap();
            drop(read);
            drop(proof);
            let uncertain = attempt.uncertain();
            f.store
                .native_replacement_start_uncertain(&uncertain)
                .unwrap();
            let proof = f.admit(&request);
            assert!(f
                .store
                .native_replacement_start_read_with_peer(
                    &allocated,
                    &request.credentials[0],
                    &proof
                )
                .is_err());
            assert_eq!(f.count("native_replacement_steps"), 3);
        }
        assert_eq!(f.count("connections"), 2);
    }
}
#[test]
fn replacement_start_rejects_ready_assertion_and_cleanup_after_a_possible_commit() {
    for changed in [false, true] {
        let (f, _, _, allocated, request) = fixture();
        let proof = f.admit(&request);
        let read = f
            .store
            .native_replacement_start_read_with_peer(&allocated, &request.credentials[0], &proof)
            .unwrap();
        let (attempt, read) = f
            .store
            .begin_native_replacement_start_with_read(
                allocated,
                &request.credentials[0],
                &proof,
                read,
            )
            .unwrap();
        drop(read);
        drop(proof);
        let mut observation = attempt.expected_resources().clone();
        if changed {
            observation["media_ready"] = json!(true);
            assert!(attempt.started(observation).is_err());
        } else {
            let captured = attempt.started(observation).unwrap();
            f.store
                .record_native_replacement_started(&captured)
                .unwrap();
            let c = f.store.connection().unwrap();
            let record: String = c
                .query_row(
                    "SELECT operation_record_id FROM native_replacements",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            c.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state) VALUES('possible-commit',?1,'native-replacement-commit','possible-commit','none','prepared')",[&record]).unwrap();
            c.execute("INSERT INTO native_replacement_steps(operation_record_id,phase,attempt_id,effect_id) VALUES(?1,'commit','possible-commit','possible-commit')",[&record]).unwrap();
            c.execute(
                "UPDATE native_replacement_steps SET started=1 WHERE phase='commit'",
                [],
            )
            .unwrap();
            drop(c);
            let absent = Gone {
                runtime: captured_runtime(&f),
                generation: 2,
            };
            assert_eq!(
                f.store
                    .record_native_replacement_started_candidate_stopped(&captured, &absent)
                    .unwrap_err()
                    .code,
                "reconciliation-required"
            );
        }
    }
}

#[test]
fn uncertain_native_resource_start_needs_exact_local_retirement_and_cannot_settle_commit() {
    for (cancelled, committing) in [(false, false), (true, false), (false, true)] {
        let (f, key, command, allocated, request) = fixture();
        let proof = f.admit(&request);
        let read = f
            .store
            .native_replacement_start_read_with_peer(&allocated, &request.credentials[0], &proof)
            .unwrap();
        let (attempt, read) = f
            .store
            .begin_native_replacement_start_with_read(
                allocated.clone(),
                &request.credentials[0],
                &proof,
                read,
            )
            .unwrap();
        let uncertain = attempt.uncertain();
        drop(read);
        if committing {
            let c = f.store.connection().unwrap();
            let record: String = c
                .query_row(
                    "SELECT operation_record_id FROM native_replacements",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            c.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state) VALUES('possible-commit',?1,'native-replacement-commit','possible-commit','none','prepared')",[&record]).unwrap();
            c.execute("INSERT INTO native_replacement_steps(operation_record_id,phase,attempt_id,effect_id) VALUES(?1,'commit','possible-commit','possible-commit')",[&record]).unwrap();
            c.execute(
                "UPDATE native_replacement_steps SET started=1 WHERE phase='commit'",
                [],
            )
            .unwrap();
        }
        // Reuse the held interval to capture historical possible IO, including
        // while an authority writer could be waiting behind the target command.
        f.store
            .native_replacement_start_uncertain_with_peer(&uncertain, &proof)
            .unwrap();
        drop(proof);
        let journal = f.count("journal");
        f.store
            .native_replacement_start_uncertain(&uncertain)
            .unwrap();
        assert_eq!(f.count("journal"), journal);
        assert!(f
            .store
            .record_native_replacement_uncertain_start_stopped(
                &uncertain,
                &Gone {
                    runtime: captured_runtime(&f),
                    generation: 1
                }
            )
            .is_err());
        assert!(f
            .store
            .record_native_replacement_uncertain_start_stopped(
                &uncertain,
                &Gone {
                    runtime: "another-native-incarnation".into(),
                    generation: 2
                }
            )
            .is_err());
        if cancelled {
            cancel(&f, &key, &command);
        }
        let admin = f.store.revalidate(&f.admin).unwrap();
        f.store.revoke(&admin, &request.credentials[0]).unwrap();
        let absent = Gone {
            runtime: captured_runtime(&f),
            generation: 2,
        };
        let result = f
            .store
            .record_native_replacement_uncertain_start_stopped(&uncertain, &absent);
        if committing {
            assert_eq!(result.unwrap_err().code, "reconciliation-required");
        } else {
            let result = result.unwrap();
            assert_eq!(
                result.state,
                if cancelled {
                    OperationState::Cancelled
                } else {
                    OperationState::Failed
                }
            );
            assert_eq!(result.effects.len(), 6);
            assert_eq!(
                result
                    .effects
                    .iter()
                    .filter(|e| e.state == EffectState::Unknown)
                    .count(),
                1
            );
            assert!(result.result.is_none());
            assert_eq!(f.count("connections"), 2);
            let journal = f.count("journal");
            assert_eq!(
                f.store
                    .record_native_replacement_uncertain_start_stopped(&uncertain, &absent)
                    .unwrap(),
                result
            );
            assert_eq!(f.count("journal"), journal);
        }
    }
}

fn resource_fixture() -> (
    Fixture,
    String,
    TransitionCommand,
    Arc<crate::NativeReplacementStarted>,
    Arc<crate::NativeReplacementMediaOrigin>,
) {
    let (f, key, command, allocated, request) = fixture();
    let proof = f.admit(&request);
    let read = f
        .store
        .native_replacement_start_read_with_peer(&allocated, &request.credentials[0], &proof)
        .unwrap();
    let (attempt, read) = f
        .store
        .begin_native_replacement_start_with_read(allocated, &request.credentials[0], &proof, read)
        .unwrap();
    let expected = attempt.expected_resources().clone();
    let started = Arc::new(attempt.started(expected).unwrap());
    drop(read);
    drop(proof);
    f.store.record_native_replacement_started(&started).unwrap();
    let origin = f
        .store
        .native_replacement_media_origin(started.clone())
        .unwrap();
    (f, key, command, started, origin)
}
#[test]
fn replacement_private_media_phases_are_ordered_once_only_and_capture_without_projection() {
    use crate::NativeReplacementMediaPhase as Phase;
    let (f, key, command, started, origin) = resource_fixture();
    let read = f.store.try_native_replacement_media_read(&origin).unwrap();
    let effects = f.count("operation_effects");
    assert_eq!(
        f.store
            .begin_native_replacement_media_phase(origin.clone(), Phase::Activate, &read)
            .unwrap_err()
            .code,
        "not-ready"
    );
    assert_eq!(f.count("operation_effects"), effects);
    assert!(f
        .store
        .try_native_replacement_scene_read(&origin, &[])
        .is_err());
    drop(read);
    let mut captures = Vec::new();
    for phase in [Phase::Attach, Phase::Activate, Phase::Readiness] {
        let read = f.store.try_native_replacement_media_read(&origin).unwrap();
        let attempt = f
            .store
            .begin_native_replacement_media_phase(origin.clone(), phase, &read)
            .unwrap();
        assert_eq!(
            f.store
                .begin_native_replacement_media_phase(origin.clone(), phase, &read)
                .unwrap_err()
                .code,
            "reconciliation-required"
        );
        let expected = attempt.expected().unwrap();
        assert_eq!(expected["generation"], 2);
        assert_eq!(expected["private"], true);
        assert_eq!(expected["media_ready"], phase == Phase::Readiness);
        let capture = attempt.applied(expected).unwrap();
        drop(read);
        let record = f
            .store
            .record_native_replacement_media_phase(&capture)
            .unwrap();
        assert_eq!(record.state, OperationState::Preparing);
        assert!(record.result.is_none());
        assert_eq!(f.count("connections"), 2);
        let journal = f.count("journal");
        assert_eq!(
            f.store
                .record_native_replacement_media_phase(&capture)
                .unwrap(),
            record
        );
        assert_eq!(f.count("journal"), journal);
        captures.push(capture);
    }
    let backup = f._dir.path().join("private-media-steps.sqlite3");
    let before = f.store.backup(&backup).unwrap();
    assert_eq!(Store::inspect_snapshot(&backup).unwrap(), before);
    let copy = rusqlite::Connection::open(&backup).unwrap();
    copy.execute_batch("DROP TRIGGER native_replacement_media_immutable; UPDATE native_replacement_media_steps SET observation_json='{\"isolated_backup_change\":true}' WHERE phase='attach'").unwrap();
    assert_ne!(
        Store::inspect_snapshot(&backup).unwrap().digest,
        before.digest
    );
    drop(copy);
    cancel(&f, &key, &command);
    let gone = Gone {
        runtime: captured_runtime(&f),
        generation: 2,
    };
    let result = f
        .store
        .record_native_replacement_started_candidate_stopped(&started, &gone)
        .unwrap();
    assert_eq!(result.state, OperationState::Cancelled);
    assert_eq!(result.effects.len(), 12);
    assert!(result
        .effects
        .iter()
        .all(|e| e.state == EffectState::Confirmed && e.attempt == 1));
    assert!(f.store.try_native_replacement_media_read(&origin).is_err());
    for capture in captures {
        assert_eq!(
            f.store
                .record_native_replacement_media_phase(&capture)
                .unwrap(),
            result
        );
    }
}
#[test]
fn uncertain_private_attach_does_not_reexecute_and_exact_native_retirement_settles_only_local_resources(
) {
    let (f, _, _, started, origin) = resource_fixture();
    let read = f.store.try_native_replacement_media_read(&origin).unwrap();
    let attempt = f
        .store
        .begin_native_replacement_media_phase(
            origin.clone(),
            crate::NativeReplacementMediaPhase::Attach,
            &read,
        )
        .unwrap();
    drop(read);
    let captured = attempt.uncertain();
    let unknown = f
        .store
        .record_native_replacement_media_phase(&captured)
        .unwrap();
    assert_eq!(unknown.state, OperationState::Unknown);
    assert!(f.store.try_native_replacement_media_read(&origin).is_err());
    let stopped = f
        .store
        .record_native_replacement_started_candidate_stopped(
            &started,
            &Gone {
                runtime: captured_runtime(&f),
                generation: 2,
            },
        )
        .unwrap();
    assert_eq!(stopped.state, OperationState::Failed);
    assert_eq!(stopped.effects.len(), 8);
    assert_eq!(
        stopped
            .effects
            .iter()
            .filter(|e| e.state == EffectState::Unknown)
            .count(),
        1
    );
    assert!(stopped.result.is_none());
    assert_eq!(f.count("connections"), 2);
}
#[test]
fn replacement_full_scene_read_uses_one_complete_interval_for_target_and_existing_routes() {
    let (f, _, _, prepared, mut request, sources) = super::allocation::prepared_target_scene();
    let proof = f.admit(&request);
    let (attempt, read) = f
        .store
        .begin_native_replacement_allocation_with_peer(&prepared, &request.credentials[0], &proof)
        .unwrap();
    let expected = attempt.expected_allocation().clone();
    let allocated = Arc::new(attempt.allocated(expected).unwrap());
    drop(read);
    drop(proof);
    f.store
        .record_native_replacement_allocated(&allocated)
        .unwrap();
    let crate::PeerCommand::NativeConnectionOffer(intent) = &mut request.command else {
        panic!()
    };
    intent.offer["replacement_preparation"]["profile"] = json!("connect-replacement-media-v1");
    let proof = f.admit(&request);
    let read = f
        .store
        .native_replacement_start_read_with_peer(&allocated, &request.credentials[0], &proof)
        .unwrap();
    let (attempt, read) = f
        .store
        .begin_native_replacement_start_with_read(allocated, &request.credentials[0], &proof, read)
        .unwrap();
    let expected = attempt.expected_resources().clone();
    let started = Arc::new(attempt.started(expected).unwrap());
    drop(read);
    drop(proof);
    f.store.record_native_replacement_started(&started).unwrap();
    let origin = f.store.native_replacement_media_origin(started).unwrap();
    let scene = f
        .store
        .try_native_replacement_scene_read(&origin, &sources)
        .unwrap();
    assert!(scene.current());
    assert!(scene
        .existing(sources[0].runtime_connection_id(), None)
        .is_some());
    let pair = scene
        .existing(
            sources[1].runtime_connection_id(),
            Some(sources[0].runtime_connection_id()),
        )
        .unwrap();
    assert!(pair.source().is_some());
    assert!(
        !pair.permits_subscriptions(),
        "Scene ownership must not add subscription permission"
    );
    drop(scene);
    assert!(f
        .store
        .try_native_replacement_scene_read(&origin, &sources[..1])
        .is_err());
}
