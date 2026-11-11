//! Controlled resource captures only; actual signed negotiation is a server test.
use super::*;
use super::{allocation::prepared_target_scene_scoped, custody::cancel};
use connect_protocol::transitions::{EffectState, OperationState};
use std::sync::Arc;
mod initial_read;
mod mixed_authority;
mod receiver_epoch;
mod scene_snapshot;

fn fixture() -> (
    Fixture,
    String,
    TransitionCommand,
    Arc<crate::NativeReplacementAllocated>,
    crate::PeerAuthorityRequest,
) {
    let (f, key, command, captured, request, _) = fixture_scene(&[]);
    (f, key, command, captured, request)
}
fn fixture_scene(
    extra_operations: &[&str],
) -> (
    Fixture,
    String,
    TransitionCommand,
    Arc<crate::NativeReplacementAllocated>,
    crate::PeerAuthorityRequest,
    Vec<Arc<crate::NativeMediaOrigin>>,
) {
    let (f, key, command, prepared, mut request, sources) =
        prepared_target_scene_scoped(extra_operations);
    let proof = f.admit(&request);
    let (attempt, read) = f
        .store
        .begin_native_replacement_allocation_with_peer(&prepared, &request.credentials[0], &proof)
        .unwrap();
    let expected = attempt.expected_allocation().clone();
    let captured = Arc::new(attempt.allocated(expected).unwrap());
    drop(read);
    // The exact positive immutable capture uses the original admitted Read.
    // Unknown and real mutations acquire their separate actual Writer.
    assert!(f.store.authority_covers(
        Store::peer_scopes(&proof),
        &[crate::AuthorityScope::conversation(
            &command.conversation_id,
            crate::AuthorityMode::Read
        )
        .unwrap(),]
    ));
    f.store
        .record_native_replacement_allocated_with_peer(&captured, &proof)
        .unwrap();
    drop(proof);
    let crate::PeerCommand::NativeConnectionOffer(intent) = &mut request.command else {
        panic!()
    };
    intent.offer["replacement_preparation"]["profile"] = json!("connect-replacement-media-v1");
    (f, key, command, captured, request, sources)
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
    let (attempt, read, _) = f
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
    // An unrelated command/status read cannot starve actual decoder input or
    // output. Provisional packet authority uses its own current SQL observer.
    let command_sql = f.store.connection().unwrap();
    let media = f.store.try_native_replacement_media_read(&origin).unwrap();
    assert!(media.current());
    assert_eq!(media.target().connection.generation, 2);
    drop(command_sql);
    assert_eq!(f.count("connections"), 2);
    drop(media);
    let observation = f.store.packet_connections.observe().unwrap();
    let observers: Vec<_> = (0..crate::packet_observation::OBSERVERS)
        .map(|_| f.store.try_packet_connection(&observation).unwrap())
        .collect();
    assert_eq!(
        f.store
            .try_native_replacement_media_read(&origin)
            .err()
            .unwrap()
            .code,
        "authority-busy"
    );
    drop(observers);
    drop(f.store.try_native_replacement_media_read(&origin).unwrap());
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
            let (attempt, read, _) = f
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
        let (attempt, read, _) = f
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
            let c = f.change_fixture();
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
        let proof = f
            .store
            .admit_peer_authority_mode(
                &request,
                &crate::AuthorityWait::new(std::time::Duration::from_secs(2)),
                true,
            )
            .unwrap();
        let read = f
            .store
            .native_replacement_start_read_with_peer(&allocated, &request.credentials[0], &proof)
            .unwrap();
        let (attempt, read, _) = f
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
            // The original signed mutation proof is already a real scene
            // Writer. Reuse it instead of re-entering the same lock footprint.
            let c = f
                .store
                .connection_with_authority(Store::peer_scopes(&proof))
                .unwrap();
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
    let (f, key, command, started, origin, _) = resource_fixture_scene();
    (f, key, command, started, origin)
}
fn resource_fixture_scene() -> (
    Fixture,
    String,
    TransitionCommand,
    Arc<crate::NativeReplacementStarted>,
    Arc<crate::NativeReplacementMediaOrigin>,
    Vec<Arc<crate::NativeMediaOrigin>>,
) {
    resource_fixture_scene_scoped(&[])
}
fn resource_fixture_scene_scoped(
    extra_operations: &[&str],
) -> (
    Fixture,
    String,
    TransitionCommand,
    Arc<crate::NativeReplacementStarted>,
    Arc<crate::NativeReplacementMediaOrigin>,
    Vec<Arc<crate::NativeMediaOrigin>>,
) {
    let (f, key, command, allocated, request, sources) = fixture_scene(extra_operations);
    let proof = f.admit(&request);
    let read = f
        .store
        .native_replacement_start_read_with_peer(&allocated, &request.credentials[0], &proof)
        .unwrap();
    let (attempt, read, _) = f
        .store
        .begin_native_replacement_start_with_read(allocated, &request.credentials[0], &proof, read)
        .unwrap();
    let expected = attempt.expected_resources().clone();
    let started = Arc::new(attempt.started(expected).unwrap());
    drop(read);
    f.store
        .record_native_replacement_started_with_peer(&started, &proof)
        .unwrap();
    drop(proof);
    let origin = f
        .store
        .native_replacement_media_origin(started.clone())
        .unwrap();
    (f, key, command, started, origin, sources)
}

#[test]
fn replacement_media_origin_busy_reread_preserves_started_capture_and_refuses_revocation() {
    let (f, _, command, started, original) = resource_fixture();
    let expected = {
        let read = f
            .store
            .try_native_replacement_media_read(&original)
            .unwrap();
        assert!(read.current());
        assert_eq!(read.target().connection.generation, 2);
        read.target().clone()
    };
    let target_credential: String = f.store.connection().unwrap().query_row(
        "SELECT json_extract(e.observation_json,'$.credential_id') FROM operations o JOIN native_replacement_steps s ON s.operation_record_id=o.record_id AND s.phase='start' JOIN operation_effects e ON e.id=s.effect_id WHERE o.operation_id=?1 AND o.command_type='connection.replace'",
        [&command.operation_id], |row| row.get(0),
    ).unwrap();
    let counts = || {
        (
            f.count("operations"),
            f.count("operation_effects"),
            f.count("native_replacements"),
            f.count("native_replacement_steps"),
            f.count("native_replacement_media_steps"),
        )
    };
    let before_counts = counts();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();

    // Real primary SQL exclusion, not a synthetic busy result. Acquiring this
    // immutable origin repeats no start/allocation effect and grants no Core IO.
    let command_sql = f.store.connection().unwrap();
    assert_eq!(
        f.store
            .native_replacement_media_origin(started.clone())
            .unwrap_err()
            .code,
        "authority-busy"
    );
    drop(command_sql);
    let recovered = f
        .store
        .native_replacement_media_origin(started.clone())
        .unwrap();
    let read = f
        .store
        .try_native_replacement_media_read(&recovered)
        .unwrap();
    assert!(read.current());
    assert_eq!(read.target().connection, expected.connection);
    assert_eq!(
        read.target().runtime_connection_id,
        expected.runtime_connection_id
    );
    drop(read);
    assert_eq!(counts(), before_counts);
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);

    // A separately live installation cannot lend this Store a started capture.
    let (foreign, _, _, foreign_started, _) = resource_fixture();
    let refused = f
        .store
        .native_replacement_media_origin(foreign_started)
        .unwrap_err();
    assert!(
        matches!(refused.code.as_str(), "forbidden" | "unauthorized"),
        "{refused:?}"
    );
    drop(foreign);
    assert_eq!(counts(), before_counts);
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);

    let admin = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&admin, &target_credential).unwrap();
    let revoked = Store::inspect_snapshot(f.store.path()).unwrap();
    let refused = f
        .store
        .native_replacement_media_origin(started)
        .unwrap_err();
    assert!(
        matches!(refused.code.as_str(), "forbidden" | "unauthorized"),
        "{refused:?}"
    );
    assert!(f
        .store
        .try_native_replacement_media_read(&recovered)
        .is_err());
    assert_eq!(counts(), before_counts);
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), revoked);
}

#[test]
fn replacement_private_media_phases_are_ordered_once_only_and_capture_without_projection() {
    use crate::NativeReplacementMediaPhase as Phase;
    let (f, key, command, started, origin, sources) = resource_fixture_scene();
    let effects = f.count("operation_effects");
    let packet_read = f.store.try_native_replacement_media_read(&origin).unwrap();
    assert_eq!(
        f.store
            .begin_native_replacement_media_phase(origin.clone(), Phase::Attach, &packet_read,)
            .unwrap_err()
            .code,
        "forbidden",
        "a packet reader must not start a durable phase"
    );
    drop(packet_read);
    assert_eq!(f.count("operation_effects"), effects);
    assert!(f
        .store
        .try_native_replacement_scene_read(&origin, &[])
        .is_err());
    let read = f
        .store
        .try_native_replacement_scene_write(&origin, &sources)
        .unwrap();
    assert_eq!(
        f.store
            .begin_native_replacement_media_phase(origin.clone(), Phase::Activate, read.media(),)
            .unwrap_err()
            .code,
        "not-ready",
        "a real writer must still obey original phase ordering"
    );
    drop(read);
    assert_eq!(f.count("operation_effects"), effects);
    let mut captures = Vec::new();
    for phase in [Phase::Attach, Phase::Activate, Phase::Readiness] {
        let read = f
            .store
            .try_native_replacement_scene_write(&origin, &sources)
            .unwrap();
        let attempt = f
            .store
            .begin_native_replacement_media_phase(origin.clone(), phase, read.media())
            .unwrap();
        assert_eq!(
            f.store
                .begin_native_replacement_media_phase(origin.clone(), phase, read.media())
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
    // This is a deliberately corrupted, isolated backup: ordinary raw writes
    // must still fail. Remove only this table's UPDATE guard and its immutable
    // trigger on the copy so inspection can detect an actual content change.
    // Never install a permissive authority UDF or alter the live Store.
    let original_triggers: Vec<String> = copy.prepare(
        "SELECT sql FROM sqlite_schema WHERE type='trigger' AND name IN ('native_replacement_media_immutable','connect_writer_native_replacement_media_steps_update') ORDER BY name"
    ).unwrap().query_map([], |row| row.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    assert_eq!(original_triggers.len(), 2);
    assert!(copy.execute_batch("UPDATE native_replacement_media_steps SET observation_json='{\"isolated_backup_change\":true}' WHERE phase='attach'").is_err());
    copy.execute_batch("DROP TRIGGER native_replacement_media_immutable; DROP TRIGGER connect_writer_native_replacement_media_steps_update; UPDATE native_replacement_media_steps SET observation_json='{\"isolated_backup_change\":true}' WHERE phase='attach'").unwrap();
    for sql in &original_triggers {
        copy.execute_batch(sql).unwrap();
    }
    let restored_triggers: Vec<String> = copy.prepare(
        "SELECT sql FROM sqlite_schema WHERE type='trigger' AND name IN ('native_replacement_media_immutable','connect_writer_native_replacement_media_steps_update') ORDER BY name"
    ).unwrap().query_map([], |row| row.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    assert_eq!(restored_triggers, original_triggers);
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
    let (f, _, _, started, origin, sources) = resource_fixture_scene();
    let read = f
        .store
        .try_native_replacement_scene_write(&origin, &sources)
        .unwrap();
    let attempt = f
        .store
        .begin_native_replacement_media_phase(
            origin.clone(),
            crate::NativeReplacementMediaPhase::Attach,
            read.media(),
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
    let (attempt, read, _) = f
        .store
        .begin_native_replacement_start_with_read(allocated, &request.credentials[0], &proof, read)
        .unwrap();
    let expected = attempt.expected_resources().clone();
    let started = Arc::new(attempt.started(expected).unwrap());
    drop(read);
    drop(proof);
    f.store.record_native_replacement_started(&started).unwrap();
    let origin = f.store.native_replacement_media_origin(started).unwrap();
    let command_sql = f.store.connection().unwrap();
    let scene = f
        .store
        .try_native_replacement_scene_read(&origin, &sources)
        .unwrap();
    drop(command_sql);
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
    let observation = f.store.packet_connections.observe().unwrap();
    let observers: Vec<_> = (0..crate::packet_observation::OBSERVERS)
        .map(|_| f.store.try_packet_connection(&observation).unwrap())
        .collect();
    assert_eq!(
        f.store
            .try_native_replacement_scene_read(&origin, &sources)
            .err()
            .unwrap()
            .code,
        "authority-busy"
    );
    drop(observers);
    drop(
        f.store
            .try_native_replacement_scene_read(&origin, &sources)
            .unwrap(),
    );
    assert!(f
        .store
        .try_native_replacement_scene_read(&origin, &sources[..1])
        .is_err());
}

#[test]
fn replacement_begin_returns_exact_start_origin_without_an_authority_or_sql_alias() {
    let (f, _, command, allocated, request) = fixture();
    let proof = f.admit(&request);
    let read = f
        .store
        .native_replacement_start_read_with_peer(&allocated, &request.credentials[0], &proof)
        .unwrap();
    let expires = read.expires_at();
    let (attempt, read, origin) = f
        .store
        .begin_native_replacement_start_with_read(allocated, &request.credentials[0], &proof, read)
        .unwrap();
    let original_expiry = origin.expires_at();
    drop(read);
    drop(proof);
    // Keeping this returned provenance alive must neither pin its old writer
    // nor introduce a hidden reader. Independent writer admission still works.
    let writer = f.controlled_conversation_writer(0);
    let sql = f.store.connection_with_authority(&writer).unwrap();
    let expected_record: String = sql
        .query_row(
            "SELECT record_id FROM operations WHERE operation_id=?1",
            [&command.operation_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(origin.operation_record_id(), expected_record);
    assert!(original_expiry <= expires);
    assert_eq!(origin.expires_at(), original_expiry);
    drop(sql);
    drop(writer);
    let fresh = f
        .store
        .resume_native_replacement_start_authority(
            &origin,
            &crate::AuthorityWait::new(std::time::Duration::from_secs(1)),
        )
        .unwrap();
    let current = f
        .store
        .native_replacement_start_read_under(&origin, &fresh)
        .unwrap();
    assert!(current.current());
    drop(current);
    drop(fresh);
    f.store
        .native_replacement_start_uncertain(&attempt.uncertain())
        .unwrap();
    assert_eq!(
        f.store
            .execute_operation_action(
                &f.admin,
                OperationAction::Get {
                    operation_id: command.operation_id
                },
            )
            .unwrap()
            .effects
            .len(),
        3
    );
}

#[test]
fn signed_start_capture_and_replay_do_not_park_writer_on_primary_sql() {
    let (f, _, _, allocated, request) = fixture();
    let proof = f.admit(&request);
    let read = f
        .store
        .native_replacement_start_read_with_peer(&allocated, &request.credentials[0], &proof)
        .unwrap();
    let (attempt, read, _) = f
        .store
        .begin_native_replacement_start_with_read(allocated, &request.credentials[0], &proof, read)
        .unwrap();
    let expected = attempt.expected_resources().clone();
    let captured = attempt.started(expected).unwrap();
    drop(read);
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let (record_immediate, blocked_record) = super::allocation::call_while_primary_held(&f, || {
        f.store
            .record_native_replacement_started_with_peer(&captured, &proof)
    });
    assert!(
        record_immediate,
        "start capture parked its live writer on SQL"
    );
    assert_eq!(blocked_record.unwrap_err().code, "authority-busy");
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    let record = f
        .store
        .record_native_replacement_started_with_peer(&captured, &proof)
        .unwrap();
    let stable = Store::inspect_snapshot(f.store.path()).unwrap();
    let (replay_immediate, blocked_replay) = super::allocation::call_while_primary_held(&f, || {
        f.store.replay_native_replacement_start_with_peer(
            &captured,
            &request.credentials[0],
            &proof,
        )
    });
    assert!(
        replay_immediate,
        "start metadata replay parked its live writer on SQL"
    );
    assert_eq!(blocked_replay.unwrap_err().code, "authority-busy");
    let replay = f
        .store
        .replay_native_replacement_start_with_peer(&captured, &request.credentials[0], &proof)
        .unwrap();
    assert_eq!(replay.record_id, record.record_id);
    assert!(replay.result.is_none());
    assert!(replay.effects.is_empty());
    assert_eq!(
        f.store
            .record_native_replacement_started_with_peer(&captured, &proof)
            .unwrap(),
        record
    );
    assert_eq!(f.count("native_replacement_steps"), 3);
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), stable);
}

#[test]
fn replacement_start_sql_not_acquired_releases_writer_then_begins_same_original_once() {
    let (f, _, command, allocated, request) = fixture();
    let proof = f.admit(&request);
    let read = f
        .store
        .native_replacement_start_read_with_peer(&allocated, &request.credentials[0], &proof)
        .unwrap();
    let original_until = read.expires_at();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let sql = f.store.connection().unwrap();
    let deferred = f
        .store
        .try_begin_native_replacement_start_with_read(
            allocated.clone(),
            &request.credentials[0],
            &proof,
            read,
        )
        .unwrap();
    assert!(
        deferred.is_none(),
        "typed deferral only before primary SQL admission"
    );
    assert_eq!(
        sql.query_row(
            "SELECT count(*) FROM native_replacement_steps WHERE phase='start'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    drop(proof); // Same caller releases every original/fresh domain alias.
    let other_writer = f
        .store
        .try_acquire_authority(&[crate::AuthorityScope::conversation(
            &command.conversation_id,
            crate::AuthorityMode::Write,
        )
        .unwrap()])
        .expect("actual independent writer progresses while primary SQL is still held");
    drop(other_writer);
    drop(sql);
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    let current = f.admit(&request);
    let mut read = f
        .store
        .native_replacement_start_read_with_peer(&allocated, &request.credentials[0], &current)
        .unwrap();
    read.limit(original_until).unwrap();
    let (attempt, read, origin) = f
        .store
        .try_begin_native_replacement_start_with_read(
            allocated.clone(),
            &request.credentials[0],
            &current,
            read,
        )
        .unwrap()
        .expect("same original begins after SQL drain");
    assert!(read.expires_at() <= original_until && origin.expires_at() <= original_until);
    assert_eq!(
        origin.operation_record_id(),
        f.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT record_id FROM operations WHERE operation_id=?1",
                [&command.operation_id],
                |r| r.get::<_, String>(0),
            )
            .unwrap()
    );
    drop(read);
    let repeat = f
        .store
        .native_replacement_start_read_with_peer(&allocated, &request.credentials[0], &current)
        .unwrap();
    assert_eq!(
        f.store
            .try_begin_native_replacement_start_with_read(
                allocated,
                &request.credentials[0],
                &current,
                repeat,
            )
            .unwrap_err()
            .code,
        "reconciliation-required"
    );
    assert_eq!(f.count("native_replacement_steps"), 3);
    drop(current);
    f.store
        .native_replacement_start_uncertain(&attempt.uncertain())
        .unwrap();
    let row = f
        .store
        .execute_operation_action(
            &f.admin,
            OperationAction::Get {
                operation_id: command.operation_id,
            },
        )
        .unwrap();
    assert_eq!(row.effects.len(), 3);
    let start = row
        .effects
        .iter()
        .filter(|effect| effect.provider == "native-replacement-start")
        .collect::<Vec<_>>();
    assert_eq!(start.len(), 1);
    assert_eq!(start[0].attempt, 1);
    assert_eq!(start[0].submission_generation, 1);
}

#[test]
fn replacement_start_typed_deferral_never_accepts_expired_or_wrong_original_authority() {
    let (f, _, _, allocated, request) = fixture();
    let proof = f.admit(&request);
    let mut read = f
        .store
        .native_replacement_start_read_with_peer(&allocated, &request.credentials[0], &proof)
        .unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(read.limit(crate::now() - 1).unwrap_err().code, "forbidden");
    assert_eq!(
        f.store
            .try_begin_native_replacement_start_with_read(
                allocated.clone(),
                &request.credentials[0],
                &proof,
                read,
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    let read = f
        .store
        .native_replacement_start_read_with_peer(&allocated, &request.credentials[0], &proof)
        .unwrap();
    let mut unrelated = request.clone();
    unrelated.command = crate::PeerCommand::Binding;
    let foreign = f.admit(&unrelated);
    assert_eq!(
        f.store
            .try_begin_native_replacement_start_with_read(
                allocated,
                &request.credentials[0],
                &foreign,
                read,
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    drop(foreign);
    drop(proof);
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
}

#[test]
fn replacement_start_after_sql_admission_error_is_never_a_typed_prebegin_deferral() {
    let (f, _, _, allocated, request) = fixture();
    let proof = f
        .store
        .admit_peer_authority_mode(
            &request,
            &crate::AuthorityWait::new(std::time::Duration::from_secs(2)),
            true,
        )
        .unwrap();
    let read = f
        .store
        .native_replacement_start_read_with_peer(&allocated, &request.credentials[0], &proof)
        .unwrap();
    {
        let sql = f
            .store
            .connection_with_authority(Store::peer_scopes(&proof))
            .unwrap();
        sql.execute_batch("CREATE TRIGGER isolated_start_reservation_failure BEFORE INSERT ON native_replacement_steps WHEN NEW.phase='start' BEGIN SELECT RAISE(ABORT,'isolated original start transaction failure'); END;").unwrap();
    }
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let refused = f.store.try_begin_native_replacement_start_with_read(
        allocated,
        &request.credentials[0],
        &proof,
        read,
    );
    assert_eq!(
        refused.unwrap_err().code,
        "storage-unavailable",
        "post-admission failure must not look like retryable None"
    );
    assert_eq!(f.count("native_replacement_steps"), 2);
    assert_eq!(
        Store::inspect_snapshot(f.store.path()).unwrap(),
        before,
        "original failed transaction rolls back its partial effect"
    );
}

#[test]
fn positive_start_capture_preserves_original_queued_epoch_and_replay() {
    let (f, _, _, allocated, request, sources) = fixture_scene(&[]);
    let proof = f.admit(&request);
    let read = f
        .store
        .native_replacement_start_read_with_peer(&allocated, &request.credentials[0], &proof)
        .unwrap();
    let (attempt, read, _) = f
        .store
        .begin_native_replacement_start_with_read(allocated, &request.credentials[0], &proof, read)
        .unwrap();
    let expected = attempt.expected_resources().clone();
    let capture = attempt.started(expected).unwrap();
    drop(read);
    drop(proof);
    let store = Arc::new(f.store);
    super::allocation::positive_resource_preserves_old_source(
        &store,
        &sources[0],
        "native-replacement-start",
        || store.record_native_replacement_started(&capture),
    );
}
