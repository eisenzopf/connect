//! Controlled Store movement captures only; no Core/media IO is claimed here.
use super::*;
use super::{allocation::prepared_target_scene_scoped, custody::cancel};
use crate::{NativeMediaOrigin, NativeReplacementMediaOrigin, NativeReplacementStarted};
use connect_protocol::transitions::{EffectState, OperationState};
use std::sync::Arc;
struct Movement {
    f: Fixture,
    key: String,
    command: TransitionCommand,
    started: Arc<NativeReplacementStarted>,
    origin: Arc<NativeReplacementMediaOrigin>,
    sources: Vec<Arc<NativeMediaOrigin>>,
    target_request: crate::PeerAuthorityRequest,
}
fn fixture(ready: bool) -> Movement {
    fixture_scoped(ready, &[])
}
fn fixture_scoped(ready: bool, extra_operations: &[&str]) -> Movement {
    fixture_prepared(ready, prepared_target_scene_scoped(extra_operations))
}

#[test]
fn pending_candidate_commit_transfers_exact_public_slot_and_old_cleanup_cannot_retire_successor() {
    let (f, key, command, prepared, request, sources, old) = super::candidate::prepared_scene();
    let m = fixture_prepared(true, (f, key, command, prepared, request, sources));
    let read =
        m.f.store
            .try_native_replacement_scene_read(&m.origin, &m.sources)
            .unwrap();
    let attempt =
        m.f.store
            .begin_native_replacement_commit(m.origin.clone(), &read)
            .unwrap();
    let expected = attempt.expected().unwrap();
    let observed = ready(&read, &expected);
    let captured = attempt.committed(expected, &observed).unwrap();
    drop(read);
    m.f.store
        .record_native_replacement_committed(&captured)
        .unwrap();
    let established =
        m.f.store
            .project_native_replacement_committed(&captured)
            .unwrap();
    assert_eq!(established.connection_id(), old.connection_id());
    assert_ne!(
        established.runtime_connection_id(),
        old.runtime_connection_id()
    );
    assert_eq!(established.generation(), 2);
    assert_eq!(m.f.count("connections"), 3);
    let snapshot = Store::inspect_snapshot(m.f.store.path()).unwrap();
    m.f.store
        .settle_native_offer_cleanup(
            &old,
            &super::candidate::Absent(old.runtime_connection_id().into(), old.generation()),
        )
        .unwrap();
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), snapshot);
    let repeated =
        m.f.store
            .project_native_replacement_committed(&captured)
            .unwrap();
    assert_eq!(
        repeated.runtime_connection_id(),
        established.runtime_connection_id()
    );
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), snapshot);
    let c = m.f.store.connection().unwrap();
    let exact:bool=c.query_row("SELECT state='ready' AND generation=2 AND replaces=?2 AND observed_runtime_id=?3 AND readiness_json IS NOT NULL FROM connections WHERE id=?1",params![old.connection_id(),m.sources[0].connection_id(),m.f.store.runtime_epoch()],|r|r.get(0)).unwrap();
    assert!(exact);
    let mapping: String = c
        .query_row(
            "SELECT runtime_connection_id FROM connection_offers WHERE connection_id=?1",
            [old.connection_id()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(mapping, established.runtime_connection_id());
}

#[test]
fn candidate_adoption_successor_recovery_never_reconstructs_old_offer_from_new_mapping() {
    let (f, key, command, prepared, request, sources, old) = super::candidate::prepared_scene();
    let m = fixture_prepared(true, (f, key, command, prepared, request, sources));
    let read =
        m.f.store
            .try_native_replacement_scene_read(&m.origin, &m.sources)
            .unwrap();
    let attempt =
        m.f.store
            .begin_native_replacement_commit(m.origin.clone(), &read)
            .unwrap();
    let expected = attempt.expected().unwrap();
    let observed = ready(&read, &expected);
    let captured = attempt.committed(expected, &observed).unwrap();
    drop(read);
    m.f.store
        .record_native_replacement_committed(&captured)
        .unwrap();
    let established =
        m.f.store
            .project_native_replacement_committed(&captured)
            .unwrap();
    let fresh_runtime = established.runtime_connection_id().to_owned();
    let path = m.f.store.path().to_owned();
    drop(m.f.store);
    let successor = Store::open(&path, connect_protocol::Limits::default()).unwrap();
    struct Recovery(std::sync::Mutex<Vec<String>>);
    impl crate::NativeOfferRecoveryObservation for Recovery {
        fn observe_retired(
            &self,
            id: &str,
            generation: u64,
        ) -> connect_protocol::Result<crate::NativeOfferObservation> {
            self.0.lock().unwrap().push(id.into());
            Ok(crate::NativeOfferObservation::Retired {
                runtime_connection_id: id.into(),
                generation,
            })
        }
    }
    let observations = Recovery(Default::default());
    successor
        .recover_native_offer_cleanup(&observations)
        .unwrap();
    let ids = observations.0.lock().unwrap();
    assert!(!ids.contains(&fresh_runtime));
    assert!(!ids.iter().any(|id| id == old.runtime_connection_id()));
}
fn fixture_prepared(
    ready: bool,
    scene: (
        Fixture,
        String,
        TransitionCommand,
        crate::NativeReplacementPrepared,
        crate::PeerAuthorityRequest,
        Vec<Arc<NativeMediaOrigin>>,
    ),
) -> Movement {
    let (f, key, command, prepared, mut request, sources) = scene;
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
    let origin = f
        .store
        .native_replacement_media_origin(started.clone())
        .unwrap();
    if ready {
        for phase in [
            crate::NativeReplacementMediaPhase::Attach,
            crate::NativeReplacementMediaPhase::Activate,
            crate::NativeReplacementMediaPhase::Readiness,
        ] {
            let read = f.store.try_native_replacement_media_read(&origin).unwrap();
            let attempt = f
                .store
                .begin_native_replacement_media_phase(origin.clone(), phase, &read)
                .unwrap();
            let expected = attempt.expected().unwrap();
            let capture = attempt.applied(expected).unwrap();
            drop(read);
            f.store
                .record_native_replacement_media_phase(&capture)
                .unwrap();
        }
    }
    Movement {
        f,
        key,
        command,
        started,
        origin,
        sources,
        target_request: request,
    }
}
fn ready(
    read: &crate::NativeReplacementSceneRead,
    expected: &serde_json::Value,
) -> crate::NativeReadyObservation {
    let t = read.media().target();
    crate::NativeReadyObservation {
        private_consultation: None,
        handoff_preparation: false,
        conversation_id: t.connection.conversation_id.clone(),
        session_id: t.connection.session_id.clone(),
        runtime_connection_id: t.runtime_connection_id.clone(),
        participant_id: t.connection.participant_id.clone(),
        membership_epoch: t.connection.membership_epoch,
        endpoint_id: t.connection.endpoint_id.clone(),
        device_id: t.device_id.clone(),
        generation: t.connection.generation,
        stream_revision: 3,
        participant_revision: expected["participant_revision"].as_u64().unwrap(),
        streams: t.offer["streams_offered"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["id"].as_str().unwrap().into())
            .collect(),
    }
}
#[test]
fn replacement_commit_requires_actual_private_readiness_and_complete_current_scene() {
    let m = fixture(false);
    let read =
        m.f.store
            .try_native_replacement_scene_read(&m.origin, &m.sources)
            .unwrap();
    let before = (m.f.count("operation_effects"), m.f.count("journal"));
    assert_eq!(
        m.f.store
            .begin_native_replacement_commit(m.origin.clone(), &read)
            .unwrap_err()
            .code,
        "not-ready"
    );
    assert_eq!(
        (m.f.count("operation_effects"), m.f.count("journal")),
        before
    );
    drop(read);
    assert!(m
        .f
        .store
        .try_native_replacement_scene_read(&m.origin, &m.sources[..1])
        .is_err());
    cancel(&m.f, &m.key, &m.command);
    assert!(m
        .f
        .store
        .try_native_replacement_scene_read(&m.origin, &m.sources)
        .is_err());
}
#[test]
fn replacement_known_commit_capture_survives_cancel_revocation_and_failed_metadata_write_without_reexecution(
) {
    let m = fixture(true);
    let read =
        m.f.store
            .try_native_replacement_scene_read(&m.origin, &m.sources)
            .unwrap();
    let attempt =
        m.f.store
            .begin_native_replacement_commit(m.origin.clone(), &read)
            .unwrap();
    assert_eq!(
        m.f.store
            .begin_native_replacement_commit(m.origin.clone(), &read)
            .unwrap_err()
            .code,
        "reconciliation-required"
    );
    let expected = attempt.expected().unwrap();
    assert_eq!(expected["generation"], 2);
    let observed = ready(&read, &expected);
    let capture = attempt.committed(expected, &observed).unwrap();
    drop(read);
    m.f.store.connection().unwrap().execute_batch("CREATE TRIGGER fixture_commit_failure BEFORE UPDATE ON native_replacement_steps WHEN NEW.phase='commit' BEGIN SELECT RAISE(ABORT,'isolated metadata failure'); END;").unwrap();
    assert_eq!(
        m.f.store
            .record_native_replacement_committed(&capture)
            .unwrap_err()
            .code,
        "storage-unavailable"
    );
    m.f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER fixture_commit_failure")
        .unwrap();
    cancel(&m.f, &m.key, &m.command);
    let admin = m.f.store.revalidate(&m.f.admin).unwrap();
    m.f.store.revoke(&admin, &m.key).unwrap();
    let record =
        m.f.store
            .record_native_replacement_committed(&capture)
            .unwrap();
    assert!(record.cancel_requested);
    assert_eq!(record.state, OperationState::Unknown);
    assert_eq!(record.effects.len(), 7);
    assert!(record
        .effects
        .iter()
        .all(|e| e.state == EffectState::Confirmed && e.attempt == 1));
    // A known engine result alone does not publish a new connection or a
    // successful public operation before independent lifetime projection.
    assert!(record.result.is_none());
    assert_eq!(m.f.count("connections"), 2);
    let before = (m.f.count("journal"), m.f.count("operation_effects"));
    assert_eq!(
        m.f.store
            .record_native_replacement_committed(&capture)
            .unwrap(),
        record
    );
    assert_eq!(
        (m.f.count("journal"), m.f.count("operation_effects")),
        before
    );
    assert!(m
        .f
        .store
        .try_native_replacement_scene_read(&m.origin, &m.sources)
        .is_err());
    assert_eq!(
        format!("{capture:?}"),
        "NativeReplacementCommitted(redacted)"
    );
}
#[test]
fn replacement_unknown_commit_cannot_be_cancelled_into_candidate_cleanup_or_worker_retry() {
    let m = fixture(true);
    let read =
        m.f.store
            .try_native_replacement_scene_read(&m.origin, &m.sources)
            .unwrap();
    let attempt =
        m.f.store
            .begin_native_replacement_commit(m.origin.clone(), &read)
            .unwrap();
    let runtime = read.media().target().runtime_connection_id.clone();
    let generation = read.media().target().connection.generation;
    drop(read);
    let capture = attempt.uncertain();
    let record =
        m.f.store
            .native_replacement_commit_uncertain(&capture)
            .unwrap();
    assert_eq!(record.state, OperationState::Unknown);
    assert_eq!(
        record
            .effects
            .iter()
            .filter(|e| e.state == EffectState::Unknown)
            .count(),
        1
    );
    let before = m.f.count("journal");
    assert_eq!(
        m.f.store
            .native_replacement_commit_uncertain(&capture)
            .unwrap(),
        record
    );
    assert_eq!(m.f.count("journal"), before);
    cancel(&m.f, &m.key, &m.command);
    struct Gone(String, u64);
    impl crate::NativeOfferRuntimeObservation for Gone {
        fn observe(&self) -> Result<crate::NativeOfferObservation> {
            Ok(crate::NativeOfferObservation::Retired {
                runtime_connection_id: self.0.clone(),
                generation: self.1,
            })
        }
    }
    assert_eq!(
        m.f.store
            .record_native_replacement_started_candidate_stopped(
                &m.started,
                &Gone(runtime, generation)
            )
            .unwrap_err()
            .code,
        "reconciliation-required"
    );
    assert!(m
        .f
        .store
        .claim_transition(&m.key, &m.command.operation_id, 60)
        .unwrap()
        .is_none());
    assert_eq!(m.f.count("connections"), 2);
    let backup = m.f._dir.path().join("commit-custody.sqlite3");
    let before = m.f.store.backup(&backup).unwrap();
    assert_eq!(Store::inspect_snapshot(&backup).unwrap(), before);
    let copy = rusqlite::Connection::open(&backup).unwrap();
    copy.execute_batch("DROP TRIGGER native_replacement_step_immutable;UPDATE native_replacement_steps SET observation_json='{}' WHERE phase='commit'").unwrap();
    assert_ne!(
        Store::inspect_snapshot(&backup).unwrap().digest,
        before.digest
    );
}
#[test]
fn replacement_commit_capture_refuses_wrong_incarnation_role_confirmation_and_unobserved_streams() {
    for changed in 0..16 {
        let m = fixture(true);
        let read =
            m.f.store
                .try_native_replacement_scene_read(&m.origin, &m.sources)
                .unwrap();
        let attempt =
            m.f.store
                .begin_native_replacement_commit(m.origin.clone(), &read)
                .unwrap();
        let mut expected = attempt.expected().unwrap();
        let mut observed = ready(&read, &expected);
        match changed {
            0 => expected["role"] = json!("observer"),
            1 => expected["confirmation"] = json!({"type":"answered-dtmf","digits":"1"}),
            2 => observed.generation = 1,
            3 => observed.runtime_connection_id = "another-incarnation".into(),
            4 => observed.endpoint_id = "another-endpoint".into(),
            5 => observed.device_id = "another-device".into(),
            6 => observed.participant_id = "another-participant".into(),
            7 => observed.membership_epoch += 1,
            8 => observed.participant_revision += 1,
            9 => observed.stream_revision = 2,
            10 => observed.streams.clear(),
            11 => observed.streams = vec!["not-negotiated".into()],
            12 => observed.streams.push(observed.streams[0].clone()),
            13 => observed.private_consultation = Some("another-private-admission".into()),
            14 => observed.handoff_preparation = true,
            15 => observed.session_id = "another-session".into(),
            _ => unreachable!(),
        }
        assert!(
            attempt.committed(expected, &observed).is_err(),
            "changed={changed}"
        );
        drop(read);
        assert_eq!(m.f.count("connections"), 2);
    }
}

#[test]
fn replacement_established_projection_keeps_target_generation_and_survives_old_key_revocation_and_retirement_race(
) {
    for old_cleanup_first in [false, true] {
        let m = fixture(true);
        let read =
            m.f.store
                .try_native_replacement_scene_read(&m.origin, &m.sources)
                .unwrap();
        let attempt =
            m.f.store
                .begin_native_replacement_commit(m.origin.clone(), &read)
                .unwrap();
        let expected = attempt.expected().unwrap();
        let observed = ready(&read, &expected);
        let capture = attempt.committed(expected, &observed).unwrap();
        drop(read);
        m.f.store
            .record_native_replacement_committed(&capture)
            .unwrap();
        if old_cleanup_first {
            let c = m.f.store.connection().unwrap();
            c.execute(
                "UPDATE native_connection_lifetimes SET retired=1 WHERE connection_id=?1",
                [m.sources[0].connection_id()],
            )
            .unwrap();
            c.execute("UPDATE connections SET state='retired',generation=generation+1,revision=revision+1,observed_runtime_id=NULL,observed_at=NULL,readiness_json=NULL WHERE id=?1",[m.sources[0].connection_id()]).unwrap();
        }
        m.f.store.connection().unwrap().execute_batch("CREATE TRIGGER fixture_projection_failure BEFORE INSERT ON native_replacement_lifetimes BEGIN SELECT RAISE(ABORT,'isolated projection failure'); END;").unwrap();
        assert!(m
            .f
            .store
            .project_native_replacement_committed(&capture)
            .is_err());
        assert_eq!(m.f.count("connections"), 2);
        assert_eq!(m.f.count("native_replacement_lifetimes"), 0);
        let topology: u64 =
            m.f.store
                .connection()
                .unwrap()
                .query_row(
                    "SELECT topology_revision FROM sessions WHERE id='session'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
        assert_eq!(topology, 3);
        m.f.store
            .connection()
            .unwrap()
            .execute_batch("DROP TRIGGER fixture_projection_failure")
            .unwrap();
        let origin =
            m.f.store
                .project_native_replacement_committed(&capture)
                .unwrap();
        assert_eq!(origin.generation(), 2);
        assert_eq!(origin.participant_id(), m.sources[0].participant_id());
        assert_eq!(origin.membership_epoch(), m.sources[0].membership_epoch());
        assert_eq!(
            origin.offer_record_id(),
            m.f.store
                .connection()
                .unwrap()
                .query_row(
                    "SELECT operation_record_id FROM native_replacements",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap()
        );
        assert_eq!(m.f.count("connections"), 3);
        assert_eq!(m.f.count("native_offer_operations"), 2);
        assert_eq!(m.f.count("native_replacement_lifetimes"), 1);
        let read = m.f.store.try_native_media_read(&origin, None).unwrap();
        assert!(read.current());
        assert_eq!(read.target().connection.generation, 2);
        assert_eq!(read.target().connection.state, "ready");
        assert!(!read.target().publication_suspended);
        assert!(read.target().offer.get("replacement_preparation").is_none());
        assert!(!read.permits_subscriptions());
        drop(read);
        assert!(m
            .f
            .store
            .try_native_media_read(&m.sources[0], None)
            .is_err());
        assert!(m.f.store.try_native_media_read(&m.sources[1], None).is_ok());
        let journal = m.f.count("journal");
        let again =
            m.f.store
                .project_native_replacement_committed(&capture)
                .unwrap();
        assert_eq!(
            again.runtime_connection_id(),
            origin.runtime_connection_id()
        );
        assert_eq!(m.f.count("journal"), journal);
        assert_eq!(
            m.f.store
                .record_native_replacement_committed(&capture)
                .unwrap()
                .state,
            OperationState::Completed
        );
        let admin = m.f.store.revalidate(&m.f.admin).unwrap();
        let operator = key(&m.f);
        m.f.store.revoke(&admin, &m.key).unwrap();
        m.f.store.revoke(&admin, &m.f.keys[0]).unwrap();
        assert!(m
            .f
            .store
            .try_native_media_read(&origin, None)
            .unwrap()
            .current());
        assert!(m.f.store.try_native_media_read(&m.sources[1], None).is_ok());
        // A later replacement plans generation three from the actual immutable
        // generation-two admission, even though the old source row is retired.
        let old_endpoint: String =
            m.f.store
                .connection()
                .unwrap()
                .query_row(
                    "SELECT endpoint_id FROM connections WHERE id=?1",
                    [m.sources[0].connection_id()],
                    |r| r.get(0),
                )
                .unwrap();
        let next_command = TransitionCommand {
            operation_id: "move-generation-three".into(),
            conversation_id: m.f.cid.clone(),
            session_id: "session".into(),
            connection_id: Some(origin.connection_id().into()),
            expected_revision: 4,
            intent: TransitionIntent::Replace {
                endpoint_id: Some(old_endpoint),
                candidate_connid: None,
                retained_connid: m.sources[1].connection_id().into(),
            },
            payload_extensions: Default::default(),
        };
        m.f.store
            .admit_transition_command(&operator, next_command.clone())
            .unwrap();
        let next_origin =
            m.f.store
                .native_replacement_origin(&operator, next_command)
                .unwrap();
        let next =
            m.f.store
                .try_native_replacement_preparation_read(
                    &next_origin,
                    &crate::NativeReplacementSourceOrigin::Native(origin.clone()),
                    &m.sources[1],
                )
                .unwrap();
        let next_plan =
            m.f.store
                .prepare_native_replacement(
                    next,
                    crate::ReplacementPreparationRequest {
                        candidate_connection_id: "replacement-third".into(),
                        participant_revision: 2,
                        timeout_seconds: 60,
                        telephone_policy: None,
                        confirmation: crate::ReplacementConfirmation::MediaOnly,
                    },
                )
                .unwrap()
                .unwrap()
                .plan()
                .unwrap();
        assert_eq!(next_plan["generation"], 3);
        assert_eq!(next_plan["source_generation"], 2);
        // The actual target credential remains authoritative after movement.
        let target: String =
            m.f.store
                .connection()
                .unwrap()
                .query_row(
                    "SELECT credential_id FROM native_replacement_lifetimes",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
        m.f.store.revoke(&admin, &target).unwrap();
        assert!(m.f.store.try_native_media_read(&origin, None).is_err());
        let c = m.f.store.connection().unwrap();
        assert!(c
            .execute("UPDATE native_replacement_lifetimes SET generation=3", [])
            .is_err());
        assert!(c
            .execute("DELETE FROM native_replacement_lifetimes", [])
            .is_err());
        let public: String = c
            .query_row(
                "SELECT connection_id FROM native_admission_sources WHERE operation_record_id=?1",
                [origin.offer_record_id()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(public, origin.connection_id());
    }
}

#[test]
fn replacement_established_stream_commands_use_candidate_parent_and_independent_target_permission()
{
    use connect_protocol::native_streams::{Command, Intent};
    for granted in [false, true] {
        let m = fixture_scoped(true, if granted { &["stream.subscribe"] } else { &[] });
        let read =
            m.f.store
                .try_native_replacement_scene_read(&m.origin, &m.sources)
                .unwrap();
        let attempt =
            m.f.store
                .begin_native_replacement_commit(m.origin.clone(), &read)
                .unwrap();
        let expected = attempt.expected().unwrap();
        let observed = ready(&read, &expected);
        let capture = attempt.committed(expected, &observed).unwrap();
        drop(read);
        m.f.store
            .record_native_replacement_committed(&capture)
            .unwrap();
        let origin =
            m.f.store
                .project_native_replacement_committed(&capture)
                .unwrap();
        let mut request = m.target_request.clone();
        request.command = crate::PeerCommand::NativeStream(Intent {
            operation_id: "select-after-movement".into(),
            conversation_id: m.f.cid.clone(),
            session_id: "session".into(),
            connection_id: origin.connection_id().into(),
            expected_revision: Some(3),
            by_participant: None,
            command: Command::Subscribe,
            payload: json!({"subscriptions":[{"from_participant":m.f.pids[1],"kinds":["audio"]}]}),
        });
        let admin = m.f.store.revalidate(&m.f.admin).unwrap();
        m.f.store.revoke(&admin, &m.key).unwrap();
        m.f.store.revoke(&admin, &m.f.keys[0]).unwrap();
        let admitted = m.f.store.admit_peer_authority(
            &request,
            &crate::AuthorityWait::new(std::time::Duration::from_secs(1)),
        );
        if !granted {
            assert!(admitted.is_err());
            continue;
        }
        let proof = admitted.unwrap();
        let projected =
            m.f.store
                .project_native_stream_with_peer(&request.credentials[0], &proof)
                .unwrap();
        assert_eq!(projected.parent_record_id, origin.offer_record_id());
        assert_eq!(
            projected.connection.connection.connection_id,
            origin.connection_id()
        );
        assert_eq!(projected.connection.connection.generation, 2);
        assert_eq!(projected.stream_revision, 3);
        let accepted =
            m.f.store
                .accept_native_stream_with_peer(&request.credentials[0], &proof)
                .unwrap();
        assert_eq!(
            accepted.connection_id.as_deref(),
            Some(origin.connection_id())
        );
        assert_eq!(accepted.state, OperationState::Accepted);
        assert_eq!(m.f.count("native_stream_operations"), 1);
        assert!(m
            .f
            .store
            .try_native_media_read(&origin, Some(&m.sources[1]))
            .unwrap()
            .permits_subscriptions());
    }
}

#[test]
fn replacement_scene_pairs_use_independent_candidate_and_existing_receiver_permissions() {
    for granted in [false, true] {
        let m = fixture_scoped(true, if granted { &["stream.subscribe"] } else { &[] });
        let read =
            m.f.store
                .try_native_replacement_scene_read(&m.origin, &m.sources)
                .unwrap();
        let candidate = &read.media().target().runtime_connection_id;
        let remote = m.sources[1].runtime_connection_id();
        let receive = read.existing(candidate, Some(remote)).unwrap();
        assert_eq!(receive.permits_subscriptions(), granted);
        assert_eq!(receive.target().runtime_connection_id, *candidate);
        assert_eq!(receive.source().unwrap().runtime_connection_id, remote);
        let other = read.existing(remote, Some(candidate)).unwrap();
        assert_eq!(other.source().unwrap().runtime_connection_id, *candidate);
        assert_eq!(
            other.permits_subscriptions(),
            read.existing(remote, None).unwrap().permits_subscriptions()
        );
        assert!(read
            .existing(candidate, Some(m.sources[0].runtime_connection_id()))
            .is_none());
        assert_eq!(m.f.count("connections"), 2);
    }
}
#[test]
fn replacement_actual_absence_cleanup_is_exact_historical_and_idempotent_after_target_revoke() {
    let m = fixture(true);
    let read =
        m.f.store
            .try_native_replacement_scene_read(&m.origin, &m.sources)
            .unwrap();
    let attempt =
        m.f.store
            .begin_native_replacement_commit(m.origin.clone(), &read)
            .unwrap();
    let expected = attempt.expected().unwrap();
    let observed = ready(&read, &expected);
    let capture = attempt.committed(expected, &observed).unwrap();
    drop(read);
    m.f.store
        .record_native_replacement_committed(&capture)
        .unwrap();
    let origin =
        m.f.store
            .project_native_replacement_committed(&capture)
            .unwrap();
    struct Gone(String, u64);
    impl crate::NativeOfferRuntimeObservation for Gone {
        fn observe(&self) -> Result<crate::NativeOfferObservation> {
            Ok(crate::NativeOfferObservation::Retired {
                runtime_connection_id: self.0.clone(),
                generation: self.1,
            })
        }
    }
    let before = m.f.count("journal");
    for wrong in [
        Gone("other-runtime".into(), 2),
        Gone(origin.runtime_connection_id().into(), 1),
    ] {
        assert!(m
            .f
            .store
            .settle_native_replacement_cleanup(&origin, &wrong)
            .is_err());
        assert_eq!(m.f.count("journal"), before);
    }
    let absent = Gone(origin.runtime_connection_id().into(), origin.generation());
    assert!(m
        .f
        .store
        .settle_native_replacement_cleanup(&m.sources[1], &absent)
        .is_err());
    let admin = m.f.store.revalidate(&m.f.admin).unwrap();
    m.f.store
        .revoke(&admin, &m.target_request.credentials[0])
        .unwrap();
    m.f.store
        .settle_native_replacement_cleanup(&origin, &absent)
        .unwrap();
    assert!(m.f.store.try_native_media_read(&origin, None).is_err());
    assert!(m
        .f
        .store
        .try_native_media_read(&m.sources[1], None)
        .unwrap()
        .current());
    let journal = m.f.count("journal");
    m.f.store
        .settle_native_replacement_cleanup(&origin, &absent)
        .unwrap();
    assert_eq!(m.f.count("journal"), journal);
    let remote_identity = m.f.store.revalidate(&m.f.keys[1]).unwrap().identity_id;
    let c = m.f.store.connection().unwrap();
    let state:(String,u64,bool)=c.query_row("SELECT c.state,c.generation,l.retired FROM connections c JOIN native_connection_lifetimes l ON l.connection_id=c.id WHERE c.id=?1",[origin.connection_id()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(state, ("retired".into(), 3, true));
    let count:u32=c.query_row("SELECT count(*) FROM journal WHERE kind='connection_retired' AND json_extract(data_json,'$.connection_id')=?1 AND recipient=?2",params![origin.connection_id(),remote_identity],|r|r.get(0)).unwrap();
    assert_eq!(count, 1);
    drop(c);
    assert_eq!(
        m.f.store
            .record_native_replacement_committed(&capture)
            .unwrap()
            .state,
        OperationState::Completed
    );
}

#[test]
fn replacement_audience_revisions_are_atomic_historical_and_keep_surviving_native_controls_current()
{
    use std::collections::BTreeMap;
    let m = fixture(false);
    let old = m.sources[0].runtime_connection_id().to_owned();
    let remote = m.sources[1].runtime_connection_id().to_owned();
    let mut candidate = String::new();
    for phase in [
        crate::NativeReplacementMediaPhase::Attach,
        crate::NativeReplacementMediaPhase::Activate,
        crate::NativeReplacementMediaPhase::Readiness,
    ] {
        let read =
            m.f.store
                .try_native_replacement_scene_read(&m.origin, &m.sources)
                .unwrap();
        candidate = read.media().target().runtime_connection_id.clone();
        let attempt =
            m.f.store
                .begin_native_replacement_media_phase(m.origin.clone(), phase, read.media())
                .unwrap();
        let expected = attempt.expected().unwrap();
        let captured = attempt.applied(expected).unwrap();
        let captured = if phase == crate::NativeReplacementMediaPhase::Activate {
            assert!(read
                .capture_activation_revisions(BTreeMap::from([(candidate.clone(), 3)]))
                .is_err());
            assert!(read
                .capture_activation_revisions(BTreeMap::from([
                    (old.clone(), 3),
                    (remote.clone(), 2),
                    (candidate.clone(), 3)
                ]))
                .is_err());
            let actual = read
                .capture_activation_revisions(BTreeMap::from([
                    (old.clone(), 2),
                    (remote.clone(), 2),
                    (candidate.clone(), 3),
                ]))
                .unwrap();
            captured.with_audience_revisions(actual).unwrap()
        } else {
            captured
        };
        drop(read);
        if phase == crate::NativeReplacementMediaPhase::Activate {
            m.f.store.connection().unwrap().execute_batch("CREATE TRIGGER fixture_audience_fail BEFORE INSERT ON native_replacement_audiences BEGIN SELECT RAISE(ABORT,'isolated audience capture failure'); END;").unwrap();
            assert!(m
                .f
                .store
                .record_native_replacement_media_phase(&captured)
                .is_err());
            let c = m.f.store.connection().unwrap();
            let before:u64=c.query_row("SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1",[m.sources[1].connection_id()],|r|r.get(0)).unwrap();
            assert_eq!(before, 1);
            c.execute_batch("DROP TRIGGER fixture_audience_fail")
                .unwrap();
            drop(c);
        }
        m.f.store
            .record_native_replacement_media_phase(&captured)
            .unwrap();
        let journal = m.f.count("journal");
        m.f.store
            .record_native_replacement_media_phase(&captured)
            .unwrap();
        assert_eq!(m.f.count("journal"), journal);
    }
    assert_eq!(m.f.count("native_replacement_audiences"), 1);
    let read =
        m.f.store
            .try_native_replacement_scene_read(&m.origin, &m.sources)
            .unwrap();
    let revisions = read
        .capture_commit_revisions(BTreeMap::from([(old, 3), (remote, 3), (candidate, 4)]))
        .unwrap();
    let attempt =
        m.f.store
            .begin_native_replacement_commit(m.origin.clone(), &read)
            .unwrap();
    let expected = attempt.expected().unwrap();
    let mut observed = ready(&read, &expected);
    observed.stream_revision = 4;
    let captured = attempt
        .committed(expected, &observed)
        .unwrap()
        .with_audience_revisions(revisions)
        .unwrap();
    drop(read);
    m.f.store
        .record_native_replacement_committed(&captured)
        .unwrap();
    let origin =
        m.f.store
            .project_native_replacement_committed(&captured)
            .unwrap();
    assert_eq!(m.f.count("native_replacement_audiences"), 2);
    let c = m.f.store.connection().unwrap();
    for (id, expected) in [
        (m.sources[1].connection_id(), 3),
        (origin.connection_id(), 4),
    ] {
        let revision:u64=c.query_row("SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1",[id],|r|r.get(0)).unwrap();
        assert_eq!(revision, expected);
    }
    assert!(c
        .execute(
            "UPDATE native_replacement_audiences SET capture_json='{}'",
            []
        )
        .is_err());
    assert!(c
        .execute("DELETE FROM native_replacement_audiences", [])
        .is_err());
    drop(c);
    let journal = m.f.count("journal");
    m.f.store
        .record_native_replacement_committed(&captured)
        .unwrap();
    assert_eq!(m.f.count("journal"), journal);
    let backup = m.f._dir.path().join("audience-revisions.sqlite3");
    let snapshot = m.f.store.backup(&backup).unwrap();
    let copy = rusqlite::Connection::open(&backup).unwrap();
    copy.execute_batch("DROP TRIGGER native_replacement_audience_immutable;UPDATE native_replacement_audiences SET capture_json='{}' WHERE phase='activate'").unwrap();
    assert_ne!(
        Store::inspect_snapshot(&backup).unwrap().digest,
        snapshot.digest
    );
}

#[test]
fn replacement_known_commit_projection_preserves_later_topology_without_replaying_physical_effect()
{
    let m = fixture(true);
    let read =
        m.f.store
            .try_native_replacement_scene_read(&m.origin, &m.sources)
            .unwrap();
    let attempt =
        m.f.store
            .begin_native_replacement_commit(m.origin.clone(), &read)
            .unwrap();
    let expected = attempt.expected().unwrap();
    let observed = ready(&read, &expected);
    let captured = attempt.committed(expected, &observed).unwrap();
    drop(read);
    m.f.store
        .record_native_replacement_committed(&captured)
        .unwrap();
    m.f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE sessions SET topology_revision=topology_revision+1 WHERE id='session'",
            [],
        )
        .unwrap();
    let origin =
        m.f.store
            .project_native_replacement_committed(&captured)
            .unwrap();
    let record =
        m.f.store
            .record_native_replacement_committed(&captured)
            .unwrap();
    assert_eq!(record.result.as_ref().unwrap()["session_revision"], 5);
    assert_eq!(record.state, OperationState::Completed);
    assert!(record.effects.iter().all(|e| e.attempt == 1));
    let journal = m.f.count("journal");
    m.f.store
        .project_native_replacement_committed(&captured)
        .unwrap();
    assert_eq!(m.f.count("journal"), journal);
    assert!(m
        .f
        .store
        .try_native_media_read(&origin, None)
        .unwrap()
        .current());
}
