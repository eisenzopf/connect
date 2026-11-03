//! Controlled preparation custody only. Observations here are fixture facts,
//! never qualification of actual Core tickets, allocated media or phone calls.
use super::*;
use crate::{
    NativeReplacementOrigin, NativeReplacementPreparationAttempt,
    NativeReplacementPreparationLease, ReplacementConfirmation, ReplacementPreparationRequest,
};
use connect_protocol::transitions::{EffectState, OperationState, ReconcileOutcome};

pub(super) fn request() -> ReplacementPreparationRequest {
    ReplacementPreparationRequest {
        candidate_connection_id: "replacement-next".into(),
        participant_revision: 1,
        timeout_seconds: 60,
        telephone_policy: None,
        confirmation: ReplacementConfirmation::MediaOnly,
    }
}
pub(super) fn setup() -> (
    Fixture,
    String,
    TransitionCommand,
    NativeReplacementOrigin,
    Vec<std::sync::Arc<crate::NativeMediaOrigin>>,
) {
    setup_with_capabilities(vec!["audio".into()])
}
pub(super) fn setup_with_capabilities(
    capabilities: Vec<String>,
) -> (
    Fixture,
    String,
    TransitionCommand,
    NativeReplacementOrigin,
    Vec<std::sync::Arc<crate::NativeMediaOrigin>>,
) {
    let (f, key, _, command, sources) = fixture_with_source_capabilities(capabilities);
    f.store
        .admit_transition_command(&key, command.clone())
        .unwrap();
    let origin = f
        .store
        .native_replacement_origin(&key, command.clone())
        .unwrap();
    (f, key, command, origin, sources)
}
pub(super) fn prepare(
    f: &Fixture,
    origin: &NativeReplacementOrigin,
    sources: &[std::sync::Arc<crate::NativeMediaOrigin>],
) -> NativeReplacementPreparationLease {
    let read = f
        .store
        .try_native_replacement_preparation_read(
            origin,
            &crate::NativeReplacementSourceOrigin::Native(sources[0].clone()),
            &sources[1],
        )
        .unwrap();
    f.store
        .prepare_native_replacement(read, request())
        .unwrap()
        .unwrap()
}
pub(super) fn begin(
    f: &Fixture,
    lease: NativeReplacementPreparationLease,
) -> NativeReplacementPreparationAttempt {
    let read = f
        .store
        .native_replacement_preparation_attempt_read(&lease)
        .unwrap();
    let (attempt, read) = f
        .store
        .begin_native_replacement_preparation(&lease, read)
        .unwrap();
    drop(read);
    attempt
}
fn status(
    f: &Fixture,
    key: &str,
    command: &TransitionCommand,
) -> connect_protocol::transitions::OperationRecord {
    f.store
        .execute_operation_action(
            key,
            OperationAction::Get {
                operation_id: command.operation_id.clone(),
            },
        )
        .unwrap()
}
pub(super) fn cancel(f: &Fixture, key: &str, command: &TransitionCommand) {
    f.store
        .execute_operation_action(
            key,
            OperationAction::Cancel {
                operation_id: "cancel-replacement".into(),
                target_operation_id: command.operation_id.clone(),
                reason: None,
            },
        )
        .unwrap();
}
#[test]
fn reservation_queues_writer_outside_sql_and_finishes_after_the_original_reader_releases() {
    let (f, key, command, origin, sources) = setup();
    let read = f
        .store
        .try_native_replacement_preparation_read(
            &origin,
            &crate::NativeReplacementSourceOrigin::Native(sources[0].clone()),
            &sources[1],
        )
        .unwrap();
    let identity = f.store.revalidate(&key).unwrap().identity_id;
    let scope = crate::AuthorityScope::identity(&identity, crate::AuthorityMode::Read).unwrap();
    let held = f.store.try_acquire_authority(&[scope.clone()]).unwrap();
    std::thread::scope(|threads| {
        let store = &f.store;
        let worker = threads.spawn(move || store.prepare_native_replacement(read, request()));
        let until = std::time::Instant::now() + std::time::Duration::from_secs(1);
        loop {
            match f.store.try_acquire_authority(&[scope.clone()]) {
                Err(error) if error.code == "authority-busy" => break,
                Ok(guard) => drop(guard),
                Err(error) => panic!("unexpected packet admission refusal: {error:?}"),
            }
            assert!(
                std::time::Instant::now() < until,
                "reservation never queued writer admission"
            );
            std::thread::yield_now();
        }
        assert!(!worker.is_finished(), "writer bypassed the original reader");
        // A file wait must never retain the shared SQL mutex. Original status
        // reads remain available and cannot observe an effect before admission.
        let before: (String, i64) = f.store.try_connection().unwrap().query_row(
            "SELECT state,(SELECT COUNT(*) FROM operation_effects WHERE operation_record_id=operations.record_id) FROM operations WHERE record_id=?1",
            [origin.record_id()], |row| Ok((row.get(0)?,row.get(1)?)),
        ).unwrap();
        assert_eq!(before, ("accepted".into(), 0));
        drop(held);
        let lease = worker.join().unwrap().unwrap().unwrap();
        assert_eq!(lease.plan().unwrap()["generation"], 2);
    });
    assert_eq!(f.count("native_replacements"), 1);
    assert_eq!(status(&f, &key, &command).effects.len(), 1);
}

#[test]
fn endpoint_reservation_refuses_contended_sql_or_authority_without_custody_or_effect() {
    let (f, key, _, origin, sources) = setup();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let read = || {
        f.store
            .try_native_replacement_preparation_read(
                &origin,
                &crate::NativeReplacementSourceOrigin::Native(sources[0].clone()),
                &sources[1],
            )
            .unwrap()
    };
    let writer = rusqlite::Connection::open(f.store.path()).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    let input = read();
    let started = std::time::Instant::now();
    assert_eq!(
        f.store
            .prepare_native_replacement(input, request())
            .unwrap_err()
            .code,
        "authority-busy"
    );
    assert!(started.elapsed() < std::time::Duration::from_millis(500));
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
    let input = read();
    let started = std::time::Instant::now();
    assert_eq!(
        f.store
            .prepare_native_replacement(input, request())
            .unwrap_err()
            .code,
        "authority-busy"
    );
    assert!(started.elapsed() < std::time::Duration::from_millis(500));
    drop(f.store.try_connection().unwrap());
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    drop(reader);
    let lease = prepare(&f, &origin, &sources);
    assert_eq!(lease.plan().unwrap()["generation"], 2);
    assert_eq!(f.count("native_replacements"), 1);
}

#[test]
fn replacement_reservation_is_exact_idempotent_and_does_not_allocate_or_claim_readiness() {
    let (f, key, command, origin, sources) = setup();
    let connections = f.count("connections");
    let first = prepare(&f, &origin, &sources);
    let duplicate = prepare(&f, &origin, &sources);
    assert_eq!(first.plan().unwrap(), duplicate.plan().unwrap());
    assert_eq!(first.expires_at(), duplicate.expires_at());
    assert_eq!(f.count("native_replacements"), 1);
    assert_eq!(f.count("native_replacement_steps"), 1);
    assert_eq!(f.count("connections"), connections);
    let record = status(&f, &key, &command);
    assert_eq!(record.state, OperationState::Preparing);
    assert!(record.result.is_none());
    assert_eq!(record.effects.len(), 1);
    assert_eq!(record.effects[0].state, EffectState::Prepared);
    assert_eq!(record.effects[0].attempt, 0);
    assert_eq!(first.plan().unwrap()["generation"], 2);
    assert_eq!(first.plan().unwrap()["source_role"], "participant");
    let mut changed = request();
    changed.confirmation = ReplacementConfirmation::AnsweredDtmf {
        digits: "42#".into(),
    };
    let read = f
        .store
        .try_native_replacement_preparation_read(
            &origin,
            &crate::NativeReplacementSourceOrigin::Native(sources[0].clone()),
            &sources[1],
        )
        .unwrap();
    assert!(f.store.prepare_native_replacement(read, changed).is_err());
    let mut changed = request();
    changed.timeout_seconds = 30;
    let read = f
        .store
        .try_native_replacement_preparation_read(
            &origin,
            &crate::NativeReplacementSourceOrigin::Native(sources[0].clone()),
            &sources[1],
        )
        .unwrap();
    assert!(f.store.prepare_native_replacement(read, changed).is_err());
    assert!(f
        .store
        .claim_transition(&key, &command.operation_id, 30)
        .unwrap()
        .is_none());
    let cleanup = f
        .store
        .claim_transition_cleanup(&f.admin, &command.operation_id, 30);
    assert!(matches!(cleanup, Ok(None)) || matches!(cleanup,Err(ref e) if e.code=="conflict"));
    let identity = f.store.revalidate(&key).unwrap().identity_id;
    drop(
        f.store
            .try_acquire_authority(&[crate::AuthorityScope::identity(
                &identity,
                crate::AuthorityMode::Write,
            )
            .unwrap()])
            .unwrap(),
    );
}
#[test]
fn replacement_preparation_begins_once_and_unknown_never_authorizes_another_attempt() {
    let (f, key, command, origin, sources) = setup();
    let lease = prepare(&f, &origin, &sources);
    let duplicate = prepare(&f, &origin, &sources);
    let read = f
        .store
        .native_replacement_preparation_attempt_read(&duplicate)
        .unwrap();
    let attempt = begin(&f, lease);
    assert!(f
        .store
        .begin_native_replacement_preparation(&duplicate, read)
        .is_err());
    f.store
        .native_replacement_preparation_uncertain(attempt)
        .unwrap();
    let record = status(&f, &key, &command);
    assert_eq!(record.state, OperationState::Unknown);
    assert_eq!(record.effects[0].state, EffectState::Unknown);
    assert_eq!(record.effects[0].attempt, 1);
    assert!(record.result.is_none());
    assert!(f
        .store
        .native_replacement_origin(&key, command.clone())
        .is_err());
    assert!(f
        .store
        .claim_transition(&key, &command.operation_id, 30)
        .unwrap()
        .is_none());
    let refused = f
        .store
        .execute_operation_action(
            &f.admin,
            OperationAction::Reconcile {
                operation_id: "cannot-assert-replacement".into(),
                target_operation_id: command.operation_id,
                outcome: ReconcileOutcome::Completed,
                evidence_id: "caller-ready-claim".into(),
            },
        )
        .unwrap_err();
    assert_eq!(refused.code, "reconciliation-required");
}
#[test]
fn replacement_preparation_cancel_before_begin_preserves_original_and_tombstones_candidate() {
    let (f, key, command, origin, sources) = setup();
    let lease = prepare(&f, &origin, &sources);
    cancel(&f, &key, &command);
    let record = status(&f, &key, &command);
    assert_eq!(record.state, OperationState::Cancelled);
    assert_eq!(record.effects[0].state, EffectState::Cancelled);
    assert_eq!(record.effects[0].attempt, 0);
    assert!(f
        .store
        .native_replacement_preparation_attempt_read(&lease)
        .is_err());
    let mut new = command.clone();
    new.operation_id = "new-operation-cannot-reuse-candidate".into();
    f.store.admit_transition_command(&key, new.clone()).unwrap();
    let origin = f.store.native_replacement_origin(&key, new).unwrap();
    let read = f
        .store
        .try_native_replacement_preparation_read(
            &origin,
            &crate::NativeReplacementSourceOrigin::Native(sources[0].clone()),
            &sources[1],
        )
        .unwrap();
    assert!(f.store.prepare_native_replacement(read, request()).is_err());
    assert_eq!(f.count("native_replacements"), 1);
    let state: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT state FROM connections WHERE id=?1",
            [sources[0].connection_id()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "ready");
}
#[test]
fn replacement_preparation_captured_fact_survives_cancel_and_revoke_without_media_or_role_claim() {
    let (f, key, command, origin, sources) = setup();
    let lease = prepare(&f, &origin, &sources);
    let expected = lease.plan().unwrap()["expected_preparation"].clone();
    let attempt = begin(&f, lease);
    let receipt = attempt.prepared(expected).unwrap();
    cancel(&f, &key, &command);
    let admin = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&admin, &key).unwrap();
    let record = f
        .store
        .record_native_replacement_prepared(&receipt)
        .unwrap();
    assert_eq!(record.state, OperationState::Unknown);
    assert_eq!(record.effects[0].state, EffectState::Confirmed);
    assert!(record.result.is_none());
    let journal = f.count("journal");
    assert_eq!(
        f.store
            .record_native_replacement_prepared(&receipt)
            .unwrap(),
        record
    );
    assert_eq!(f.count("journal"), journal);
    let fact:String=f.store.connection().unwrap().query_row("SELECT data_json FROM operation_observations WHERE operation_record_id=?1 AND source='native-replacement-prepare'",[origin.record_id()],|r|r.get(0)).unwrap();
    let fact: serde_json::Value = serde_json::from_str(&fact).unwrap();
    assert_eq!(fact["media_ready"], false);
    assert_eq!(fact["native_replacement_prepared"], true);
    assert!(fact["candidate"].is_null());
    assert_eq!(f.count("connections"), 2);
    let foreign = Store::open_administration(f.store.path(), Limits::default()).unwrap();
    assert!(foreign
        .record_native_replacement_prepared(&receipt)
        .is_err());
}
#[test]
fn replacement_preparation_refuses_forged_ticket_fields_and_ready_claims() {
    for changed in [
        "operation_id",
        "session",
        "participant",
        "membership_epoch",
        "source",
        "retained",
        "candidate",
        "endpoint",
        "device",
        "generation",
        "participant_revision",
        "ready",
    ] {
        let (f, _, _, origin, sources) = setup();
        let lease = prepare(&f, &origin, &sources);
        let mut expected = lease.plan().unwrap()["expected_preparation"].clone();
        let attempt = begin(&f, lease);
        expected[changed] = if matches!(
            changed,
            "generation" | "participant_revision" | "membership_epoch"
        ) {
            json!(99)
        } else if changed == "ready" {
            json!(true)
        } else {
            json!("forged")
        };
        assert!(attempt.prepared(expected).is_err(), "{changed}");
        assert_eq!(f.count("operation_observations"), 0);
    }
}
#[test]
fn replacement_preparation_current_policy_loss_prevents_begin_and_capture_is_not_execution() {
    for changed in ["source-key", "endpoint", "role", "candidate"] {
        let (f, _, _, origin, sources) = setup();
        let lease = prepare(&f, &origin, &sources);
        match changed {
            "source-key" => {
                let admin = f.store.revalidate(&f.admin).unwrap();
                f.store.revoke(&admin, &f.keys[0]).unwrap();
            }
            "endpoint" => {
                f.store
                    .connection()
                    .unwrap()
                    .execute(
                        "UPDATE endpoints SET verification_state='revoked' WHERE id=?1",
                        [lease.plan().unwrap()["expected_preparation"]["endpoint"]
                            .as_str()
                            .unwrap()],
                    )
                    .unwrap();
            }
            "role" => {
                f.store.connection().unwrap().execute("UPDATE session_participants SET role='observer' WHERE session_id='session' AND participant_id=?1",[&f.pids[0]]).unwrap();
            }
            "candidate" => {
                assert!(f
                    .store
                    .connection()
                    .unwrap()
                    .execute(
                        "UPDATE native_replacements SET candidate_connection_id='different'",
                        []
                    )
                    .is_err());
                continue;
            }
            _ => unreachable!(),
        }
        // Revalidate the complete prepared plan as well as original source IDs.
        let result = f
            .store
            .native_replacement_preparation_attempt_read(&lease)
            .and_then(|read| f.store.begin_native_replacement_preparation(&lease, read));
        assert!(result.is_err(), "{changed}");
    }
}
#[test]
fn replacement_restart_fences_unstarted_started_and_captured_preparation_without_reexecution() {
    for phase in ["reserved", "started", "captured"] {
        let (f, key, command, origin, sources) = setup();
        let lease = prepare(&f, &origin, &sources);
        if phase != "reserved" {
            let expected = lease.plan().unwrap()["expected_preparation"].clone();
            let attempt = begin(&f, lease);
            if phase == "captured" {
                f.store
                    .record_native_replacement_prepared(&attempt.prepared(expected).unwrap())
                    .unwrap();
            }
        }
        let path = f.store.path().to_path_buf();
        let Fixture { _dir, store, .. } = f;
        drop(store);
        let successor = Store::open(&path, Limits::default()).unwrap();
        let record = successor
            .execute_operation_action(
                &key,
                OperationAction::Get {
                    operation_id: command.operation_id,
                },
            )
            .unwrap();
        assert_eq!(
            record.state,
            if phase == "reserved" {
                OperationState::Failed
            } else {
                OperationState::Unknown
            }
        );
        assert_eq!(record.effects[0].attempt, u64::from(phase != "reserved"));
        if phase == "captured" {
            assert_eq!(record.effects[0].state, EffectState::Confirmed);
            assert!(!record.causal_event_ids.is_empty());
        }
        assert!(successor.native_replacement_context(&origin).is_err());
        assert_eq!(
            successor
                .connection()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM native_replacements", [], |r| r
                    .get::<_, u64>(0))
                .unwrap(),
            1
        );
        drop(sources);
        drop(successor);
        drop(_dir);
    }
}
#[test]
fn replacement_backup_digest_covers_reservation_and_step_state() {
    let (f, _, _, origin, sources) = setup();
    let lease = prepare(&f, &origin, &sources);
    let a = f._dir.path().join("reserved.sqlite3");
    let first = f.store.backup(&a).unwrap();
    assert_eq!(Store::inspect_snapshot(&a).unwrap(), first);
    // In an isolated copy, change only the new native tables. Operation/effect
    // changes must not be the reason their contents influence the digest.
    let isolated = f._dir.path().join("isolated-native-tables.sqlite3");
    std::fs::copy(&a, &isolated).unwrap();
    let copy = rusqlite::Connection::open(&isolated).unwrap();
    copy.execute("UPDATE native_replacement_steps SET started=1", [])
        .unwrap();
    let step_only = Store::inspect_snapshot(&isolated).unwrap();
    assert_ne!(first.digest, step_only.digest);
    // Deliberate corruption in this disposable backup copy, never the Store:
    // immutable-reservation rows must also be covered by inspection.
    copy.execute_batch("DROP TRIGGER native_replacement_immutable; UPDATE native_replacements SET candidate_runtime_id='corrupted-backup-runtime'").unwrap();
    assert_ne!(
        step_only.digest,
        Store::inspect_snapshot(&isolated).unwrap().digest
    );
    drop(copy);
    let attempt = begin(&f, lease);
    let b = f._dir.path().join("started.sqlite3");
    let second = f.store.backup(&b).unwrap();
    assert_ne!(first.digest, second.digest);
    f.store
        .native_replacement_preparation_uncertain(attempt)
        .unwrap();
    let c = rusqlite::Connection::open(&a).unwrap();
    assert_eq!(
        c.query_row("SELECT started FROM native_replacement_steps", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn replacement_telephone_reservation_requires_explicit_valid_configured_digits_without_dialing() {
    let (f, _, _, mut command, sources) = fixture_with_sources();
    let identity = f.store.revalidate(&f.keys[0]).unwrap().identity_id;
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope.operations.push("connection.replace".into());
    let key = f
        .store
        .issue(&identity, scope, crate::now() + 600)
        .unwrap()
        .id;
    let ProvisioningOutcome::Endpoint { endpoint_id, .. } = f
        .store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::Endpoint {
                operation_id: "provision-replacement-telephone".into(),
                identity_id: identity,
                device: DeviceSpec {
                    id: "replacement-telephone-device".into(),
                    kind: "telephone".into(),
                    platform: "isolated-no-io-fixture".into(),
                },
                transport: EndpointTransport::Telephone,
                address: "+15555550199".into(),
                capabilities: vec!["audio".into()],
                secret_ref: Some("fixture-telephone-route".into()),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    f.store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::VerifyEndpoint {
                operation_id: "verify-replacement-telephone".into(),
                endpoint_id: endpoint_id.clone(),
                expected_revision: 0,
                evidence_ref: "controlled-no-io-fixture".into(),
            },
        )
        .unwrap();
    if let TransitionIntent::Replace {
        endpoint_id: target,
        ..
    } = &mut command.intent
    {
        *target = Some(endpoint_id);
    }
    f.store
        .admit_transition_command(&key, command.clone())
        .unwrap();
    let origin = f
        .store
        .native_replacement_origin(&key, command.clone())
        .unwrap();
    let read = f
        .store
        .try_native_replacement_preparation_read(
            &origin,
            &crate::NativeReplacementSourceOrigin::Native(sources[0].clone()),
            &sources[1],
        )
        .unwrap();
    assert!(f.store.prepare_native_replacement(read, request()).is_err());
    for digits in ["", "12345678901234567", "invalid"] {
        let mut r = request();
        r.confirmation = ReplacementConfirmation::AnsweredDtmf {
            digits: digits.into(),
        };
        let read = f
            .store
            .try_native_replacement_preparation_read(
                &origin,
                &crate::NativeReplacementSourceOrigin::Native(sources[0].clone()),
                &sources[1],
            )
            .unwrap();
        assert!(f.store.prepare_native_replacement(read, r).is_err());
    }
    assert_eq!(status(&f, &key, &command).state, OperationState::Accepted);
    assert_eq!(f.count("native_replacements"), 0);
    let r = crate::TelephoneReplacementPolicy {
        route_reference: "fixture-telephone-route".into(),
        sip_authority: "carrier.example".into(),
        confirmation_digits: "7#".into(),
        timeout_seconds: 60,
    }
    .request("replacement-next".into(), 1);
    let read = f
        .store
        .try_native_replacement_preparation_read(
            &origin,
            &crate::NativeReplacementSourceOrigin::Native(sources[0].clone()),
            &sources[1],
        )
        .unwrap();
    let lease = f
        .store
        .prepare_native_replacement(read, r)
        .unwrap()
        .unwrap();
    assert_eq!(
        lease.plan().unwrap()["confirmation"],
        json!({"type":"answered-dtmf","digits":"7#"})
    );
    assert_eq!(f.count("connections"), 2);
    assert_eq!(f.count("sip_legs"), 0);
    assert!(status(&f, &key, &command).result.is_none());
}

#[test]
fn replacement_preflight_retry_requires_the_exact_unreserved_original_and_is_read_only() {
    let (f, key, command, origin, sources) = setup();
    let before = status(&f, &key, &command);
    let journal = f.count("journal");
    assert!(f
        .store
        .native_replacement_preflight_unstarted(&key, command.clone(), origin.record_id())
        .unwrap());
    assert_eq!(status(&f, &key, &command), before);
    assert_eq!(f.count("journal"), journal);
    let mut changed = command.clone();
    changed.expected_revision += 1;
    assert!(f
        .store
        .native_replacement_preflight_unstarted(&key, changed, origin.record_id())
        .is_err());
    assert!(f
        .store
        .native_replacement_preflight_unstarted(&key, command.clone(), "other-record")
        .is_err());
    let _lease = prepare(&f, &origin, &sources);
    assert!(!f
        .store
        .native_replacement_preflight_unstarted(&key, command.clone(), origin.record_id())
        .unwrap());
    assert_eq!(status(&f, &key, &command).state, OperationState::Preparing);
}

#[test]
fn replacement_preflight_retry_refuses_cancelled_or_revoked_original_authority() {
    for cancelled in [false, true] {
        let (f, key, command, origin, _) = setup();
        if cancelled {
            cancel(&f, &key, &command);
        } else {
            let admin = f.store.revalidate(&f.admin).unwrap();
            f.store.revoke(&admin, &key).unwrap();
        }
        assert!(f
            .store
            .native_replacement_preflight_unstarted(&key, command, origin.record_id())
            .is_err());
        assert_eq!(f.count("native_replacements"), 0);
    }
}

#[test]
fn replacement_preflight_refusal_is_exact_and_cannot_close_a_reserved_attempt() {
    let (f, key, command, origin, sources) = setup();
    let mut changed = command.clone();
    changed.expected_revision += 1;
    assert!(f
        .store
        .fail_native_replacement_preflight(&key, changed, "forbidden")
        .is_err());
    assert!(f
        .store
        .fail_native_replacement_preflight(&key, command.clone(), "unrecognized")
        .is_err());
    assert_eq!(status(&f, &key, &command).state, OperationState::Accepted);
    let lease = prepare(&f, &origin, &sources);
    assert!(!f
        .store
        .fail_native_replacement_preflight(&key, command.clone(), "forbidden")
        .unwrap());
    assert_eq!(status(&f, &key, &command).state, OperationState::Preparing);
    drop(lease);

    let (f, key, command, _, _) = setup();
    assert!(f
        .store
        .fail_native_replacement_preflight(&key, command.clone(), "unsupported-media-profile")
        .unwrap());
    let record = status(&f, &key, &command);
    assert_eq!(record.state, OperationState::Failed);
    assert!(record.effects.is_empty());
    assert!(record.result.is_none());
    let journal = f.count("journal");
    assert!(!f
        .store
        .fail_native_replacement_preflight(&key, command.clone(), "forbidden")
        .unwrap());
    assert_eq!(f.count("journal"), journal);
    assert_eq!(
        f.store.admit_transition_command(&key, command).unwrap(),
        record
    );
    assert_eq!(f.count("native_replacements"), 0);
}

#[test]
fn replacement_captured_local_stop_is_idempotent_after_cancel_or_revocation() {
    for cancelled in [false, true] {
        let (f, key, command, origin, sources) = setup();
        let lease = prepare(&f, &origin, &sources);
        let observation = lease.plan().unwrap()["expected_preparation"].clone();
        let receipt = begin(&f, lease).prepared(observation).unwrap();
        assert_eq!(
            f.store
                .record_native_replacement_preparation_stopped(&receipt)
                .unwrap_err()
                .code,
            "reconciliation-required"
        );
        if cancelled {
            cancel(&f, &key, &command);
        }
        let admin = f.store.revalidate(&f.admin).unwrap();
        f.store.revoke(&admin, &key).unwrap();
        f.store
            .record_native_replacement_prepared(&receipt)
            .unwrap();
        let foreign = Store::open_administration(f.store.path(), Limits::default()).unwrap();
        assert!(foreign
            .record_native_replacement_preparation_stopped(&receipt)
            .is_err());
        let stopped = f
            .store
            .record_native_replacement_preparation_stopped(&receipt)
            .unwrap();
        assert_eq!(
            stopped.state,
            if cancelled {
                OperationState::Cancelled
            } else {
                OperationState::Failed
            }
        );
        assert!(stopped.result.is_none());
        assert_eq!(stopped.effects.len(), 2);
        assert!(stopped
            .effects
            .iter()
            .all(|e| e.state == EffectState::Confirmed && e.attempt == 1));
        let cleanup = stopped
            .effects
            .iter()
            .find(|e| e.compensates_effect_id.is_some())
            .unwrap();
        let original = stopped
            .effects
            .iter()
            .find(|e| e.compensates_effect_id.is_none())
            .unwrap();
        assert_eq!(
            cleanup.compensates_effect_id.as_deref(),
            Some(original.id.as_str())
        );
        let journal = f.count("journal");
        assert_eq!(
            f.store
                .record_native_replacement_preparation_stopped(&receipt)
                .unwrap(),
            stopped
        );
        assert_eq!(f.count("journal"), journal);
        assert_eq!(f.count("connections"), 2);
    }
}

#[test]
fn replacement_local_stop_cannot_assert_cleanup_after_target_allocation_started() {
    let (f, _, _, origin, sources) = setup();
    let lease = prepare(&f, &origin, &sources);
    let observation = lease.plan().unwrap()["expected_preparation"].clone();
    let receipt = begin(&f, lease).prepared(observation).unwrap();
    f.store
        .record_native_replacement_prepared(&receipt)
        .unwrap();
    // Represent a later durable, possibly external step. The preparation-only
    // receipt must never be sufficient to assert that this IO was undone.
    let c = f.store.connection().unwrap();
    c.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,attempt,submission_generation) VALUES('allocation-effect',?1,'native-replacement-allocate','allocation-effect','none','prepared',0,0)", [origin.record_id()]).unwrap();
    c.execute("INSERT INTO native_replacement_steps(operation_record_id,phase,attempt_id,effect_id) VALUES(?1,'allocate','allocation-attempt','allocation-effect')", [origin.record_id()]).unwrap();
    c.execute("UPDATE native_replacement_steps SET started=1 WHERE operation_record_id=?1 AND phase='allocate'", [origin.record_id()]).unwrap();
    drop(c);
    assert_eq!(
        f.store
            .record_native_replacement_preparation_stopped(&receipt)
            .unwrap_err()
            .code,
        "reconciliation-required"
    );
    let effects: u64 = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM operation_effects WHERE operation_record_id=?1",
            [origin.record_id()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(effects, 2);
}

#[test]
fn replacement_reserved_candidate_refuses_ordinary_native_preparation_before_and_after_cancel() {
    let (f, key, command, origin, sources) = setup();
    let lease = prepare(&f, &origin, &sources);
    let plan = lease.plan().unwrap();
    let target = plan["expected_preparation"]["endpoint"].as_str().unwrap();
    let mut native = f.request(0, "ordinary-native").intent;
    native.connection_id = request().candidate_connection_id;
    native.operation_id = "cannot-adopt-reserved-replacement".into();
    native.expected_revision = Some(0);
    let reserved =
        connect_protocol::connection_control::PrepareRequest::new(native.clone(), target.into());
    let result = f.store.execute(
        &key,
        &connect_protocol::Operation::ConnectionPrepare {
            request: reserved.clone(),
        },
    );
    assert_eq!(result.unwrap_err().code, "conflict");
    cancel(&f, &key, &command);
    let result = f.store.execute(
        &key,
        &connect_protocol::Operation::ConnectionPrepare { request: reserved },
    );
    assert_eq!(result.unwrap_err().code, "conflict");
    assert_eq!(f.count("connections"), 2);
    assert_eq!(f.count("native_replacements"), 1);
}

#[test]
fn replacement_reservation_keeps_writer_admission_until_original_sql_is_available() {
    let (f, key, _, origin, sources) = setup();
    let identity = f.store.revalidate(&key).unwrap().identity_id;
    let scope = crate::AuthorityScope::identity(&identity, crate::AuthorityMode::Read).unwrap();
    let blocker = f
        .store
        .try_acquire_authority(std::slice::from_ref(&scope))
        .unwrap();
    let read = f
        .store
        .try_native_replacement_preparation_read(
            &origin,
            &crate::NativeReplacementSourceOrigin::Native(sources[0].clone()),
            &sources[1],
        )
        .unwrap();
    std::thread::scope(|threads| {
        let store = &f.store;
        let reservation = threads.spawn(move || store.prepare_native_replacement(read, request()));
        let until = std::time::Instant::now() + std::time::Duration::from_millis(200);
        loop {
            assert!(
                std::time::Instant::now() < until,
                "original reservation never requested writer admission"
            );
            match f.store.try_acquire_authority(std::slice::from_ref(&scope)) {
                Ok(reader) => drop(reader),
                Err(error) => {
                    assert_eq!(error.code, "authority-busy");
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        // Let this original acquire its writer interval while another metadata
        // unit briefly owns SQL. Releasing that interval and restarting the
        // whole preflight would let continuous source readers overtake it.
        let sql = f.store.connection().unwrap();
        drop(blocker);
        std::thread::sleep(std::time::Duration::from_millis(20));
        drop(sql);
        let _lease = reservation
            .join()
            .unwrap()
            .expect("reservation abandoned admitted authority when SQL was briefly busy")
            .expect("original reservation was not created");
    });
    let c = f.store.connection().unwrap();
    let (steps, effects): (u64, u64) = c
        .query_row(
            "SELECT (SELECT COUNT(*) FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='prepare' AND started=0),(SELECT COUNT(*) FROM operation_effects WHERE operation_record_id=?1 AND provider='native-replacement-prepare' AND attempt=0)",
            [origin.record_id()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((steps, effects), (1, 1));
}

#[test]
fn reservation_metadata_can_coexist_with_live_packet_authority() {
    let (f, key, command, origin, sources) = setup();
    let live_packet = f.store.try_native_media_read(&sources[0], None).unwrap();
    let input = f
        .store
        .try_native_replacement_preparation_read(
            &origin,
            &crate::NativeReplacementSourceOrigin::Native(sources[0].clone()),
            &sources[1],
        )
        .unwrap();
    let connections = f.count("connections");
    // This reservation changes only original operation metadata. It must not
    // require retiring an otherwise current packet reader to make progress.
    let lease = f
        .store
        .prepare_native_replacement(input, request())
        .unwrap()
        .unwrap();
    assert!(live_packet.current());
    assert!(f
        .store
        .try_native_media_read(&sources[0], None)
        .unwrap()
        .current());
    assert!(f
        .store
        .try_native_media_read(&sources[1], None)
        .unwrap()
        .current());
    let duplicate = prepare(&f, &origin, &sources);
    assert_eq!(duplicate.plan().unwrap(), lease.plan().unwrap());
    assert_eq!(duplicate.expires_at(), lease.expires_at());
    assert_eq!(f.count("connections"), connections);
    assert_eq!(f.count("native_replacements"), 1);
    let record = status(&f, &key, &command);
    assert_eq!(record.state, OperationState::Preparing);
    assert_eq!(record.effects.len(), 1);
    assert_eq!(record.effects[0].state, EffectState::Prepared);
    assert_eq!(record.effects[0].attempt, 0);
}
