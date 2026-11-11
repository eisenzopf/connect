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
fn reservation_metadata_respects_a_real_queued_revocation() {
    let (f, key, _, origin, sources) = setup();
    let effects_before = f.count("operation_effects");
    let input = f
        .store
        .try_native_replacement_preparation_read(
            &origin,
            &crate::NativeReplacementSourceOrigin::Native(sources[0].clone()),
            &sources[1],
        )
        .unwrap();
    let identity = f.store.revalidate(&key).unwrap().identity_id;
    let scope = crate::AuthorityScope::identity(&identity, crate::AuthorityMode::Read).unwrap();
    let blocker = f
        .store
        .try_acquire_authority(std::slice::from_ref(&scope))
        .unwrap();
    let admin = f.store.revalidate(&f.admin).unwrap();
    std::thread::scope(|threads| {
        let store = &f.store;
        let credential = key.clone();
        let writer = threads.spawn(move || store.revoke(&admin, &credential));
        let until = std::time::Instant::now() + std::time::Duration::from_secs(1);
        loop {
            match f.store.try_acquire_authority(std::slice::from_ref(&scope)) {
                Err(error) if error.code == "authority-busy" => break,
                Ok(reader) => drop(reader),
                Err(error) => panic!("unexpected packet admission refusal: {error:?}"),
            }
            assert!(std::time::Instant::now() < until, "revocation never queued");
            std::thread::yield_now();
        }
        // Fresh reservation admission cannot retain/bypass its input readers
        // once an actual authorization writer has queued. No effect can begin.
        assert_eq!(
            f.store
                .prepare_native_replacement(input, request())
                .unwrap_err()
                .code,
            "authority-busy"
        );
        assert_eq!(f.count("native_replacements"), 0);
        assert_eq!(f.count("operation_effects"), effects_before);
        drop(f.store.try_connection().unwrap());
        drop(blocker);
        writer.join().unwrap().unwrap();
    });
    assert!(f.store.revalidate(&key).is_err());
    assert!(f
        .store
        .try_native_replacement_preparation_read(
            &origin,
            &crate::NativeReplacementSourceOrigin::Native(sources[0].clone()),
            &sources[1],
        )
        .is_err());
    assert_eq!(f.count("native_replacements"), 0);
}

#[test]
fn endpoint_reservation_refuses_contended_sql_but_accepts_current_readers() {
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
    let lease = f
        .store
        .prepare_native_replacement(read(), request())
        .unwrap()
        .unwrap();
    assert_eq!(lease.plan().unwrap()["generation"], 2);
    assert_eq!(f.count("native_replacements"), 1);
    drop(reader);
    let repeated = prepare(&f, &origin, &sources);
    assert_eq!(repeated.plan().unwrap(), lease.plan().unwrap());
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
    // A second original Read cannot be held through the first Write begin.
    drop(read);
    let attempt = begin(&f, lease);
    let read = f
        .store
        .native_replacement_preparation_attempt_read(&duplicate)
        .unwrap();
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
                f.change_fixture()
                    .execute(
                        "UPDATE endpoints SET verification_state='revoked' WHERE id=?1",
                        [lease.plan().unwrap()["expected_preparation"]["endpoint"]
                            .as_str()
                            .unwrap()],
                    )
                    .unwrap();
            }
            "role" => {
                f.change_fixture().execute("UPDATE session_participants SET role='observer' WHERE session_id='session' AND participant_id=?1",[&f.pids[0]]).unwrap();
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
                // The same corruption must also refuse with a genuine bound
                // Writer, so a missing SQL permit cannot mask immutability.
                assert!(f
                    .change_fixture()
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
    fn corrupt_offline_copy(copy: &rusqlite::Connection, names: &[&str], mutation: &str) {
        let schema = || -> Vec<(String, String)> {
            copy.prepare("SELECT name,sql FROM sqlite_schema WHERE type='trigger' ORDER BY name")
                .unwrap()
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
        };
        let before = schema();
        let selected: Vec<_> = before
            .iter()
            .filter(|(name, _)| names.contains(&name.as_str()))
            .collect();
        assert_eq!(selected.len(), names.len());
        // Raw external mutation remains forbidden even on a closed copy.
        assert!(copy.execute_batch(mutation).is_err());
        for (name, _) in &selected {
            copy.execute_batch(&format!("DROP TRIGGER {name}")).unwrap();
        }
        copy.execute_batch(mutation).unwrap();
        for (_, sql) in selected {
            copy.execute_batch(sql).unwrap();
        }
        assert_eq!(
            schema(),
            before,
            "restore exact trigger SQL before digest comparison"
        );
    }
    corrupt_offline_copy(
        &copy,
        &["connect_writer_native_replacement_steps_update"],
        "UPDATE native_replacement_steps SET started=1",
    );
    let step_only = Store::inspect_snapshot(&isolated).unwrap();
    assert_ne!(first.digest, step_only.digest);
    // Deliberate corruption in this disposable backup copy, never the Store:
    // immutable-reservation rows must also be covered by inspection.
    corrupt_offline_copy(
        &copy,
        &[
            "native_replacement_immutable",
            "connect_writer_native_replacements_update",
        ],
        "UPDATE native_replacements SET candidate_runtime_id='corrupted-backup-runtime'",
    );
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
    let c = f.change_fixture();
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
fn reservation_writer_drains_original_packet_reads_and_does_not_pin_later_media() {
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
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let refusal = f
        .store
        .prepare_native_replacement(input, request())
        .unwrap_err()
        .code;
    let old_current = live_packet.current();
    drop(live_packet);
    assert_eq!(refusal, "authority-busy");
    assert!(old_current);
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);

    let lease = prepare(&f, &origin, &sources);
    let packet = f.store.try_native_media_read(&sources[0], None).unwrap();
    let sibling = f.store.try_native_media_read(&sources[1], None).unwrap();
    let current = packet.current() && sibling.current();
    drop(sibling);
    drop(packet);
    assert!(
        current,
        "state-change writer escaped its bounded reservation"
    );
    let duplicate = prepare(&f, &origin, &sources);
    assert_eq!(duplicate.plan().unwrap(), lease.plan().unwrap());
    assert_eq!(duplicate.expires_at(), lease.expires_at());
    assert_eq!(f.count("native_replacements"), 1);
    let record = status(&f, &key, &command);
    assert_eq!(record.state, OperationState::Preparing);
    assert_eq!(record.effects.len(), 1);
    assert_eq!(record.effects[0].state, EffectState::Prepared);
    assert_eq!(record.effects[0].attempt, 0);
}

#[test]
fn reserved_preparation_begin_refusal_keeps_same_lease_effect_and_original_cutoff() {
    let (f, _, _, origin, sources) = setup();
    let lease = prepare(&f, &origin, &sources);
    let read = f
        .store
        .native_replacement_preparation_attempt_read(&lease)
        .unwrap();
    let until = read.expires_at();
    let before = (
        f.count("native_replacements"),
        f.count("native_replacement_steps"),
        f.count("operation_effects"),
    );
    let primary = f.store.connection().unwrap();
    assert_eq!(
        f.store
            .begin_native_replacement_preparation_before(&lease, read, until)
            .err()
            .expect("held primary SQL must defer begin before its transaction")
            .code,
        "authority-busy"
    );
    drop(primary);
    assert_eq!(
        before,
        (
            f.count("native_replacements"),
            f.count("native_replacement_steps"),
            f.count("operation_effects")
        )
    );
    let not_started:(bool,u64,String)=f.store.connection().unwrap().query_row("SELECT s.started,e.attempt,e.state FROM native_replacement_steps s JOIN operation_effects e ON e.id=s.effect_id WHERE s.operation_record_id=?1 AND s.phase='prepare'",[lease.record_id()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(not_started, (false, 0, "prepared".into()));
    let read = f
        .store
        .native_replacement_preparation_attempt_read(&lease)
        .unwrap();
    assert_eq!(
        f.store
            .begin_native_replacement_preparation_before(&lease, read, crate::now() - 1)
            .err()
            .expect("original cutoff cannot renew")
            .code,
        "forbidden"
    );
    let read = f
        .store
        .native_replacement_preparation_attempt_read(&lease)
        .unwrap();
    let (attempt, read) = f
        .store
        .begin_native_replacement_preparation_before(&lease, read, until)
        .unwrap();
    assert!(read.current() && read.expires_at() <= until);
    drop(read);
    assert_eq!(
        before,
        (
            f.count("native_replacements"),
            f.count("native_replacement_steps"),
            f.count("operation_effects")
        )
    );
    let read = f
        .store
        .native_replacement_preparation_attempt_read(&lease)
        .unwrap();
    assert!(
        f.store
            .begin_native_replacement_preparation_before(&lease, read, until)
            .is_err(),
        "successful original begin cannot be replayed"
    );
    f.store
        .native_replacement_preparation_uncertain(attempt)
        .unwrap();
}

#[test]
fn reservation_mixed_scope_still_excludes_same_conversation_readers() {
    let (f, key, _, origin, sources) = setup();
    let identity = f.store.revalidate(&key).unwrap().identity_id;
    let identity_reader = f
        .store
        .try_acquire_authority(&[crate::AuthorityScope::identity(
            &identity,
            crate::AuthorityMode::Read,
        )
        .unwrap()])
        .unwrap();
    let conversation_reader = f
        .store
        .try_acquire_authority(&[crate::AuthorityScope::conversation(
            &f.cid,
            crate::AuthorityMode::Read,
        )
        .unwrap()])
        .unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let read = f
        .store
        .try_native_replacement_preparation_read(
            &origin,
            &crate::NativeReplacementSourceOrigin::Native(sources[0].clone()),
            &sources[1],
        )
        .unwrap();
    let refusal = f
        .store
        .prepare_native_replacement(read, request())
        .unwrap_err();
    assert_eq!(refusal.code, "authority-busy");
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    assert_eq!(f.count("native_replacements"), 0);
    drop(conversation_reader);
    // Keeping the original Identity reader is safe: only exact CID facts mutate.
    let lease = prepare(&f, &origin, &sources);
    let read = f
        .store
        .native_replacement_preparation_attempt_read(&lease)
        .unwrap();
    let until = read.expires_at();
    let (attempt, writer) = f
        .store
        .begin_native_replacement_preparation_before(&lease, read, until)
        .unwrap();
    assert!(writer.current() && writer.expires_at() <= until);
    drop(writer);
    drop(identity_reader);
    f.store
        .native_replacement_preparation_uncertain(attempt)
        .unwrap();
}

#[test]
fn reservation_mixed_scope_retains_identity_policy_fences_and_rechecks_current_selection() {
    let (f, key, _, origin, sources) = setup();
    let identity = f.store.revalidate(&key).unwrap().identity_id;
    let read = f
        .store
        .try_native_replacement_preparation_read(
            &origin,
            &crate::NativeReplacementSourceOrigin::Native(sources[0].clone()),
            &sources[1],
        )
        .unwrap();
    let selection = read.into_selection().unwrap();
    let scopes = {
        let c = f.store.try_connection().unwrap();
        let mut current = selection.scopes(&f.store, &c, false).unwrap();
        let (cid, owner, mutation) = f
            .store
            .native_replacement_bookkeeping_scopes(&c, origin.record_id())
            .unwrap();
        assert_eq!(cid, f.cid);
        assert_eq!(owner, identity);
        current.extend(mutation);
        crate::AuthorityScope::merged(&current).unwrap()
    };
    let held = f.store.try_acquire_authority(&scopes).unwrap();
    assert!(f.store.authority_covers(
        &held,
        &[
            crate::AuthorityScope::identity(&identity, crate::AuthorityMode::Read).unwrap(),
            crate::AuthorityScope::conversation(&f.cid, crate::AuthorityMode::Write).unwrap()
        ]
    ));
    assert!(!f.store.authority_covers(
        &held,
        &[crate::AuthorityScope::identity(&identity, crate::AuthorityMode::Write).unwrap()]
    ));
    // Read is still a real fence, not a dropped/replaced currentness check.
    assert_eq!(
        f.store
            .try_acquire_authority(&[crate::AuthorityScope::identity(
                &identity,
                crate::AuthorityMode::Write
            )
            .unwrap()])
            .err()
            .expect("current identity Read must exclude revocation Writer")
            .code,
        "authority-busy"
    );
    drop(held);
    let before = f.count("operation_effects");
    let admin = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&admin, &key).unwrap();
    // The immutable selection is routing metadata, never a cached permission.
    let c = f.store.try_connection().unwrap();
    assert!(selection.scopes(&f.store, &c, false).is_err());
    drop(c);
    assert!(f
        .store
        .try_native_replacement_preparation_read(
            &origin,
            &crate::NativeReplacementSourceOrigin::Native(sources[0].clone()),
            &sources[1]
        )
        .is_err());
    assert_eq!(f.count("native_replacements"), 0);
    assert_eq!(f.count("operation_effects"), before);
}

#[test]
fn replacement_status_recovery_releases_original_read_before_exact_writer() {
    for started in [false, true] {
        let (f, key, command, origin, sources) = setup();
        let input = f
            .store
            .try_native_replacement_preparation_read(
                &origin,
                &crate::NativeReplacementSourceOrigin::Native(sources[0].clone()),
                &sources[1],
            )
            .unwrap();
        let mut request = request();
        request.timeout_seconds = 2;
        let lease = f
            .store
            .prepare_native_replacement(input, request)
            .unwrap()
            .unwrap();
        let expires = lease.expires_at();
        let _attempt = started.then(|| begin(&f, lease));
        let owner = f.store.revalidate(&key).unwrap().identity_id;
        let scopes = [
            crate::AuthorityScope::identity(&owner, crate::AuthorityMode::Read).unwrap(),
            crate::AuthorityScope::conversation(&f.cid, crate::AuthorityMode::Read).unwrap(),
        ];
        // A current Get must stay read-only and coexist with the existing route.
        let read = f.store.try_acquire_authority(&scopes).unwrap();
        assert_eq!(status(&f, &key, &command).state, OperationState::Preparing);
        drop(read);
        // Let this actual bounded reservation expire. Immutable admission and
        // plan cutoffs, effect IDs and attempt IDs are never rewritten.
        while crate::now() < expires {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let read = f.store.try_acquire_authority(&scopes).unwrap();
        let action = OperationAction::Get {
            operation_id: command.operation_id.clone(),
        };
        let error = f
            .store
            .execute_operation_action_with_authority(&key, action.clone(), &read)
            .err()
            .expect("expired Get must refuse its retained Read");
        assert_eq!(error.code, "authority-busy");
        {
            let c = f.store.connection().unwrap();
            let state: String = c
                .query_row(
                    "SELECT state FROM operations WHERE operation_id=?1 AND owner_id=?2",
                    params![command.operation_id, owner],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(state, "preparing");
        }
        drop(read);
        // Historical recovery does not need to revoke the unrelated actor's
        // Identity Read. The source Conversation is still excluded by Writer.
        let identity_read = f.store.try_acquire_authority(&scopes[..1]).unwrap();
        let recovered = f.store.execute_operation_action(&key, action).unwrap();
        assert_eq!(
            recovered.state,
            if started {
                OperationState::Unknown
            } else {
                OperationState::Failed
            }
        );
        assert_eq!(recovered.effects.len(), 1);
        assert_eq!(recovered.effects[0].attempt, u64::from(started));
        assert_eq!(
            recovered.effects[0].state,
            if started {
                EffectState::Unknown
            } else {
                EffectState::Cancelled
            }
        );
        assert_eq!(status(&f, &key, &command), recovered);
        drop(identity_read);
    }
}

#[test]
fn reserved_control_read_waits_for_actual_sql_drain_without_reacquiring_or_renewing() {
    let (f, key, _, origin, sources) = setup();
    let lease = prepare(&f, &origin, &sources);
    let owner = f.store.revalidate(&key).unwrap().identity_id;
    let writer = f
        .store
        .acquire_authority(
            &[crate::AuthorityScope::identity(&owner, crate::AuthorityMode::Write).unwrap()],
            &crate::AuthorityWait::new(std::time::Duration::from_secs(1)),
        )
        .unwrap();
    let original_until = (crate::now() + 3).min(lease.expires_at());
    let wait = crate::AuthorityWait::new(std::time::Duration::from_secs(2));
    let (at_file, file_wait) = std::sync::mpsc::channel();
    let (done, result) = std::sync::mpsc::channel();
    let (observed, completion_while_sql_held, joined) = std::thread::scope(|scope| {
        let store = &f.store;
        let lease = &lease;
        let worker = scope.spawn(move || {
            let mut until = original_until;
            let read = crate::authority_fences::with_waited_lock_probe(
                move || {
                    let _ = at_file.send(());
                },
                || {
                    store.native_replacement_preparation_attempt_read_with_wait(
                        lease, &mut until, &wait,
                    )
                },
            );
            done.send(()).unwrap();
            (read, until)
        });
        // This real WouldBlock proves first SQL/facts completed, with no SQL
        // mutex held while the worker waits for the actual Identity fence.
        let reached_file = file_wait.recv_timeout(std::time::Duration::from_secs(1));
        let sql = f.store.connection().unwrap();
        drop(writer);
        let early = result.recv_timeout(std::time::Duration::from_millis(30));
        drop(sql);
        let joined = worker.join().unwrap();
        (reached_file, early, joined)
    });
    assert!(observed.is_ok());
    assert!(
        completion_while_sql_held.is_err(),
        "control read must wait for the actual SQL holder"
    );
    let (read, cap) = joined;
    let read = read.unwrap();
    assert!(read.current());
    assert!(read.expires_at() <= cap && cap <= original_until);
    drop(read);
    assert_eq!(f.count("native_replacement_steps"), 1);
    let c = f.store.connection().unwrap();
    let (started, attempt): (bool, u64) = c.query_row(
        "SELECT s.started,e.attempt FROM native_replacement_steps s JOIN operation_effects e ON e.id=s.effect_id WHERE s.operation_record_id=?1",
        [lease.record_id()], |r| Ok((r.get(0)?,r.get(1)?)),
    ).unwrap();
    assert!(!started && attempt == 0);
}

#[test]
fn reserved_control_read_rechecks_withdrawn_original_policy_after_file_wait() {
    let (f, key, _, origin, sources) = setup();
    let lease = prepare(&f, &origin, &sources);
    let effects_before = f.count("operation_effects");
    let owner = f.store.revalidate(&key).unwrap().identity_id;
    let writer = f
        .store
        .acquire_authority(
            &[crate::AuthorityScope::identity(&owner, crate::AuthorityMode::Write).unwrap()],
            &crate::AuthorityWait::new(std::time::Duration::from_secs(1)),
        )
        .unwrap();
    let original_until = (crate::now() + 3).min(lease.expires_at());
    let wait = crate::AuthorityWait::new(std::time::Duration::from_secs(2));
    let (at_file, file_wait) = std::sync::mpsc::channel();
    let (observed, joined) = std::thread::scope(|scope| {
        let store = &f.store;
        let lease = &lease;
        let worker = scope.spawn(move || {
            let mut until = original_until;
            let read = crate::authority_fences::with_waited_lock_probe(
                move || {
                    let _ = at_file.send(());
                },
                || {
                    store.native_replacement_preparation_attempt_read_with_wait(
                        lease, &mut until, &wait,
                    )
                },
            );
            (read, until)
        });
        let observed = file_wait.recv_timeout(std::time::Duration::from_secs(1));
        {
            let c = f.store.connection_with_authority(&writer).unwrap();
            assert_eq!(
                c.execute("UPDATE credentials SET revoked=1 WHERE id=?1", [&key])
                    .unwrap(),
                1
            );
        }
        drop(writer);
        (observed, worker.join().unwrap())
    });
    assert!(observed.is_ok());
    let (read, until) = joined;
    assert_eq!(
        read.err()
            .expect("withdrawn original credential must refuse current authentication")
            .code,
        "unauthorized"
    );
    assert!(until <= original_until);
    assert_eq!(f.count("native_replacement_steps"), 1);
    assert_eq!(f.count("operation_effects"), effects_before);
}

#[test]
fn reserved_control_sql_wait_cancels_without_beginning_original_attempt() {
    let (f, _, _, origin, sources) = setup();
    let lease = prepare(&f, &origin, &sources);
    let effects_before = f.count("operation_effects");
    let wait = crate::AuthorityWait::new(std::time::Duration::from_secs(2));
    let sql = f.store.connection().unwrap();
    let (entered, started) = std::sync::mpsc::channel();
    let (done, completion) = std::sync::mpsc::channel();
    let (early, joined) = std::thread::scope(|scope| {
        let store = &f.store;
        let lease = &lease;
        let worker_wait = wait.clone();
        let worker = scope.spawn(move || {
            let mut until = lease.expires_at();
            entered.send(()).unwrap();
            let read = store.native_replacement_preparation_attempt_read_with_wait(
                lease,
                &mut until,
                &worker_wait,
            );
            done.send(()).unwrap();
            (read, until)
        });
        let _ = started.recv_timeout(std::time::Duration::from_secs(1));
        let early = completion.recv_timeout(std::time::Duration::from_millis(30));
        wait.cancel();
        (early, worker.join().unwrap())
    });
    drop(sql);
    assert!(early.is_err());
    let (read, until) = joined;
    assert_eq!(
        read.err()
            .expect("actual SQL wait must observe cancellation")
            .code,
        "cancelled"
    );
    assert!(until <= lease.expires_at());
    assert_eq!(f.count("native_replacement_steps"), 1);
    assert_eq!(f.count("operation_effects"), effects_before);
}

#[test]
fn reserved_control_first_facts_cutoff_survives_failed_file_admission() {
    let (f, key, _, origin, sources) = setup();
    let lease = prepare(&f, &origin, &sources);
    let owner = f.store.revalidate(&key).unwrap().identity_id;
    let writer = f
        .store
        .acquire_authority(
            &[crate::AuthorityScope::identity(&owner, crate::AuthorityMode::Write).unwrap()],
            &crate::AuthorityWait::new(std::time::Duration::from_secs(1)),
        )
        .unwrap();
    let mut until = lease.expires_at();
    let refused = f
        .store
        .native_replacement_preparation_attempt_read_with_wait(
            &lease,
            &mut until,
            &crate::AuthorityWait::new(std::time::Duration::from_millis(500)),
        );
    drop(writer);
    assert_eq!(
        refused
            .err()
            .expect("actual Identity writer must exclude Read admission")
            .code,
        "authority-busy"
    );
    // Initial SQL and facts completed before file admission refused. Preserve
    // that cutoff despite there having been no successfully returned Read.
    assert!(until <= crate::now() + 5 && until < lease.expires_at());
    let first_until = until;
    let read = f
        .store
        .native_replacement_preparation_attempt_read_with_wait(
            &lease,
            &mut until,
            &crate::AuthorityWait::new(std::time::Duration::from_secs(1)),
        )
        .unwrap();
    assert!(read.current());
    assert!(read.expires_at() <= first_until && until <= first_until);
    drop(read);
    assert_eq!(f.count("native_replacement_steps"), 1);
}

// The race starts after real file admission. A primary SQL owner acquired
// earlier would prevent the initial scope derivation and miss this boundary.
fn preparation_sql_race_keeps_old_source_available<T: Send>(
    store: &std::sync::Arc<Store>,
    source: &std::sync::Arc<crate::NativeMediaOrigin>,
    preserves_epoch: bool,
    prepare: impl FnOnce() -> crate::Result<T> + Send,
) -> T {
    use std::time::{Duration, Instant};
    let mut watcher = store.start_media_authorization_watcher().unwrap();
    let weak = std::sync::Arc::downgrade(store);
    let original = source.clone();
    let snapshot = store
        .register_native_media_snapshot_for(source, None, move || {
            weak.upgrade()
                .ok_or_else(crate::Error::forbidden)?
                .try_native_media_read(&original, None)
        })
        .unwrap();
    let until = Instant::now() + Duration::from_secs(2);
    let first = loop {
        match snapshot.admit() {
            Ok(io) => break io,
            Err(error) if error.code == "authority-busy" && Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(1))
            }
            Err(error) => panic!("original source publication setup refused: {error}"),
        }
    };
    let old_stamp = first.snapshot_stamp().unwrap();
    let old_epoch = first.packet_epoch().unwrap();
    drop(first);
    let before = Store::inspect_snapshot(store.path()).unwrap();
    let (entered, ready) = std::sync::mpsc::sync_channel(1);
    let (release, resume) = std::sync::mpsc::sync_channel(1);
    let (proof_entered, same_state, fresh_current, old_current, result) =
        std::thread::scope(|workers| {
            let worker = workers.spawn(move || {
                crate::authority_admission::with_passive_admission_probe(
                    move |admitted| {
                        let _ = entered.send(admitted);
                        let _ = resume.recv_timeout(Duration::from_secs(1));
                    },
                    prepare,
                )
            });
            let proof_entered = ready.recv_timeout(Duration::from_secs(1)) == Ok(true);
            let primary = store.connection().unwrap();
            let _ = release.send(());
            // Inspect while the exact caller cannot enter its durable transaction.
            // Published IO uses independent observer SQL; this primary owner is
            // therefore no reason to withhold an otherwise current old route.
            let until = Instant::now() + Duration::from_millis(100);
            let mut live_io = loop {
                match snapshot.admit() {
                    Ok(io) => break Some(io),
                    Err(error) if error.code == "authority-busy" && Instant::now() < until => {
                        std::thread::sleep(Duration::from_millis(1))
                    }
                    Err(_) => break None,
                }
            };
            let old_current = old_epoch.current() && snapshot.admit_original(old_stamp).is_ok();
            let same_state =
                Store::inspect_snapshot(store.path()).is_ok_and(|state| state == before);
            let mut fresh_current = live_io.as_ref().is_some_and(|io| io.current());
            if !preserves_epoch {
                // Reservation still genuinely needs Write and must drain this IO.
                drop(live_io.take());
            }
            drop(primary);
            let result = worker.join().unwrap();
            if preserves_epoch {
                // Begin must also finish while genuine old-source IO is retained.
                fresh_current &= live_io.as_ref().is_some_and(|io| io.current());
            }
            (
                proof_entered,
                same_state,
                fresh_current,
                old_current,
                result,
            )
        });
    drop(snapshot);
    watcher.shutdown().unwrap();
    assert!(
        proof_entered,
        "probe must hold the actual acquired Conversation proof"
    );
    assert!(
        same_state,
        "blocked primary custody cannot start a reservation/effect"
    );
    assert_eq!(
        old_current, preserves_epoch,
        "dormant begin preserves its queued epoch; a real Writer revocation stays irreversible"
    );
    assert!(
        fresh_current,
        "preparation kept the old route unavailable while waiting on primary SQL"
    );
    result.unwrap_or_else(|error| panic!("same preparation failed after SQL release: {error}"))
}

fn preparation_effect_state(store: &Store, record: &str) -> (u64, u64, u64) {
    store.connection().unwrap().query_row(
        "SELECT count(*),COALESCE(sum(s.started),0),COALESCE(sum(e.attempt),0) FROM native_replacement_steps s JOIN operation_effects e ON e.id=s.effect_id WHERE s.operation_record_id=?1 AND s.phase='prepare'",
        [record], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))
    ).unwrap()
}

#[test]
fn preparation_reservation_releases_writer_during_primary_sql_reacquire() {
    let (f, _, _, origin, sources) = setup();
    let read = f
        .store
        .try_native_replacement_preparation_read(
            &origin,
            &crate::NativeReplacementSourceOrigin::Native(sources[0].clone()),
            &sources[1],
        )
        .unwrap();
    let store = std::sync::Arc::new(f.store);
    let lease = preparation_sql_race_keeps_old_source_available(&store, &sources[0], false, || {
        store.prepare_native_replacement(read, request())
    })
    .unwrap();
    assert_eq!(lease.record_id(), origin.record_id());
    assert_eq!(
        preparation_effect_state(&store, origin.record_id()),
        (1, 0, 0)
    );
}

#[test]
fn preparation_begin_preserves_epoch_during_primary_sql_reacquire() {
    let (f, _, _, origin, sources) = setup();
    let lease = prepare(&f, &origin, &sources);
    let read = f
        .store
        .native_replacement_preparation_attempt_read(&lease)
        .unwrap();
    let cutoff = read.expires_at();
    let store = std::sync::Arc::new(f.store);
    let (_attempt, read) =
        preparation_sql_race_keeps_old_source_available(&store, &sources[0], true, || {
            store.begin_native_replacement_preparation_before(&lease, read, cutoff)
        });
    assert!(read.current());
    assert!(read.expires_at() <= cutoff);
    drop(read);
    assert_eq!(
        preparation_effect_state(&store, origin.record_id()),
        (1, 1, 1)
    );
}

#[test]
fn preparation_begin_preserves_queued_old_source_epoch() {
    use std::time::{Duration, Instant};
    let (f, _, _, origin, sources) = setup();
    let lease = prepare(&f, &origin, &sources);
    let store = std::sync::Arc::new(f.store);
    let mut watcher = store.start_media_authorization_watcher().unwrap();
    let weak = std::sync::Arc::downgrade(&store);
    let original = sources[0].clone();
    let snapshot = store
        .register_native_media_snapshot_for(&sources[0], None, move || {
            weak.upgrade()
                .ok_or_else(crate::Error::forbidden)?
                .try_native_media_read(&original, None)
        })
        .unwrap();
    let until = Instant::now() + Duration::from_secs(2);
    let first = loop {
        match snapshot.admit() {
            Ok(io) => break io,
            Err(error) if error.code == "authority-busy" && Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(error) => panic!("original old source publication setup refused: {error}"),
        }
    };
    let stamp = first.snapshot_stamp().unwrap();
    let epoch = first.packet_epoch().unwrap();
    drop(first);
    let read = store
        .native_replacement_preparation_attempt_read(&lease)
        .unwrap();
    let cutoff = read.expires_at();
    let begun = store.begin_native_replacement_preparation_before(&lease, read, cutoff);
    let epoch_preserved = epoch.current() && snapshot.admit_original(stamp).is_ok();
    drop(snapshot);
    watcher.shutdown().unwrap();
    let (attempt, read) = begun.unwrap();
    assert_eq!(attempt.record_id(), origin.record_id());
    assert!(read.current() && read.expires_at() <= cutoff);
    drop(read);
    assert_eq!(
        preparation_effect_state(&store, origin.record_id()),
        (1, 1, 1)
    );
    assert!(
        epoch_preserved,
        "begin revoked the original queued old-source epoch despite only submitting its dormant preparation effect"
    );
}

#[test]
fn preparation_begin_permit_authorizes_only_exact_positive_ledger() {
    use crate::authority_writer_boundary::phase_ledger::{Key, Stage};
    let (f, _, _, origin, sources) = setup();
    let _lease = prepare(&f, &origin, &sources);
    let record = origin.record_id();
    let (owner, cid, runtime, reservation, digest, attempt, effect): (String, String, String, String, String, String, String) = f.store.connection().unwrap().query_row(
        "SELECT o.owner_id,o.conversation_id,n.runtime_epoch,n.reservation_id,n.plan_digest,s.attempt_id,s.effect_id FROM native_replacements n JOIN operations o ON o.record_id=n.operation_record_id JOIN native_replacement_steps s ON s.operation_record_id=n.operation_record_id AND s.phase='prepare' WHERE n.operation_record_id=?1",
        [record], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?)),
    ).unwrap();
    let held = f
        .store
        .try_acquire_authority(&[
            crate::AuthorityScope::identity(&owner, crate::AuthorityMode::Read).unwrap(),
            crate::AuthorityScope::conversation(&cid, crate::AuthorityMode::Read).unwrap(),
        ])
        .unwrap();
    let mut sql = f.store.connection().unwrap();
    let tx = sql
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let start = "UPDATE native_replacement_steps SET started=1 WHERE operation_record_id=?1 AND phase='prepare'";
    let submit = "UPDATE operation_effects SET state='submitting',attempt=1,submission_generation=1 WHERE id=?1";
    assert!(
        tx.execute(start, [record]).is_err(),
        "registered Read alone cannot mutate exact phase"
    );
    assert!(
        tx.execute(submit, [&effect]).is_err(),
        "registered Read alone cannot submit exact effect"
    );
    let key = |attempt: String, stage| Key {
        record: record.to_owned(),
        phase: "prepare",
        attempt,
        effect: effect.clone(),
        expected: String::new(),
        runtime: runtime.clone(),
        reservation: reservation.clone(),
        digest: digest.clone(),
        stage,
    };
    let boundary = &f.store.authority_writer_boundary;
    assert!(boundary
        .native_phase_ledger(
            &f.store,
            &tx,
            &held,
            key("foreign-attempt".into(), Stage::Begin)
        )
        .is_err());
    assert!(boundary
        .native_phase_ledger(&f.store, &tx, &held, key(attempt.clone(), Stage::Confirm))
        .is_err());
    let permit = boundary
        .native_phase_ledger(&f.store, &tx, &held, key(attempt, Stage::Begin))
        .unwrap();
    for action in [
        "intent-root",
        "intent-member-birth",
        "intent-endpoint-birth",
    ] {
        let authorized: bool = tx.query_row(
            "SELECT connect_store_native_phase_ledger_v1(?1,?2,NULL,'intent-birth',?3,NULL,NULL)",
            rusqlite::params![action,record,""], |row| row.get(0),
        ).unwrap();
        assert!(
            !authorized,
            "actual non-intent Begin permit authorized {action}"
        );
    }

    assert!(
        tx.execute(submit, [&effect]).is_err(),
        "effect cannot submit before exact started step"
    );
    assert!(tx.execute("UPDATE native_replacement_steps SET started=1,observation_json='{}' WHERE operation_record_id=?1 AND phase='prepare'",[record]).is_err());
    assert_eq!(tx.execute(start, [record]).unwrap(), 1);
    assert!(tx.execute("UPDATE operation_effects SET state='submitting',attempt=1,submission_generation=1,provider_ref='forged' WHERE id=?1",[&effect]).is_err());
    assert_eq!(tx.execute(submit, [&effect]).unwrap(), 1);
    assert!(tx
        .execute(
            "UPDATE operation_effects SET state='confirmed' WHERE id=?1",
            [&effect]
        )
        .is_err());
    assert!(tx
        .execute(
            "UPDATE operations SET revision=revision+1,state='unknown' WHERE record_id=?1",
            [record]
        )
        .is_err());
    let touch = "UPDATE operations SET revision=revision+1 WHERE record_id=?1";
    assert_eq!(tx.execute(touch, [record]).unwrap(), 1);
    assert!(
        tx.execute(touch, [record]).is_err(),
        "same permit cannot advance revision twice"
    );
    drop(permit);
    tx.rollback().unwrap();
    drop(sql);
    drop(held);
    assert_eq!(preparation_effect_state(&f.store, record), (1, 0, 0));
}

#[test]
fn positive_prepared_capture_and_replay_preserve_old_source_epoch() {
    use std::time::{Duration, Instant};
    let (f, _, _, origin, sources) = setup();
    let lease = prepare(&f, &origin, &sources);
    let expected = lease.plan().unwrap()["expected_preparation"].clone();
    let attempt = begin(&f, lease);
    let receipt = attempt.prepared(expected).unwrap();
    let store = std::sync::Arc::new(f.store);
    let mut watcher = store.start_media_authorization_watcher().unwrap();
    let weak = std::sync::Arc::downgrade(&store);
    let original = sources[0].clone();
    let snapshot = store
        .register_native_media_snapshot_for(&sources[0], None, move || {
            weak.upgrade()
                .ok_or_else(crate::Error::forbidden)?
                .try_native_media_read(&original, None)
        })
        .unwrap();
    let until = Instant::now() + Duration::from_secs(2);
    let first = loop {
        match snapshot.admit() {
            Ok(io) => break io,
            Err(error) if error.code == "authority-busy" && Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(error) => panic!("original old source publication setup refused: {error}"),
        }
    };
    let stamp = first.snapshot_stamp().unwrap();
    let epoch = first.packet_epoch().unwrap();
    drop(first);
    let captured = store.record_native_replacement_prepared(&receipt);
    let original_after_capture = epoch.current() && snapshot.admit_original(stamp).is_ok();
    let replayed = store.record_native_replacement_prepared(&receipt);
    let original_after_replay = epoch.current() && snapshot.admit_original(stamp).is_ok();
    drop(snapshot);
    watcher.shutdown().unwrap();
    let captured = captured.unwrap();
    assert_eq!(captured.state, OperationState::Preparing);
    assert_eq!(captured.effects.len(), 1);
    assert_eq!(captured.effects[0].state, EffectState::Confirmed);
    assert_eq!(captured.effects[0].attempt, 1);
    assert!(captured.result.is_none());
    assert_eq!(replayed.unwrap(), captured);
    let rows: u64 = store.connection().unwrap().query_row(
        "SELECT count(*) FROM operation_observations WHERE operation_record_id=?1 AND source='native-replacement-prepare'",
        [origin.record_id()], |row| row.get(0),
    ).unwrap();
    assert_eq!(rows, 1);
    assert!(
        original_after_capture,
        "positive prepared capture revoked the queued old-source epoch"
    );
    assert!(
        original_after_replay,
        "exact captured replay revoked the queued old-source epoch"
    );
}

#[test]
fn exact_prepared_capture_replay_does_not_reserve_sql_writer() {
    let (f, _, _, origin, sources) = setup();
    let lease = prepare(&f, &origin, &sources);
    let expected = lease.plan().unwrap()["expected_preparation"].clone();
    let receipt = begin(&f, lease).prepared(expected).unwrap();
    let original = f
        .store
        .record_native_replacement_prepared(&receipt)
        .unwrap();
    let mut external = rusqlite::Connection::open(f.store.path()).unwrap();
    let writer = external
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let replay = f.store.record_native_replacement_prepared(&receipt);
    writer.rollback().unwrap();
    assert_eq!(replay.unwrap(), original);
}

#[test]
fn fresh_generated_reservation_preserves_queued_old_source_epoch() {
    use std::time::{Duration, Instant};
    let (f, _, _, origin, sources) = setup();
    let store = std::sync::Arc::new(f.store);
    let mut watcher = store.start_media_authorization_watcher().unwrap();
    let weak = std::sync::Arc::downgrade(&store);
    let original = sources[0].clone();
    let snapshot = store
        .register_native_media_snapshot_for(&sources[0], None, move || {
            weak.upgrade()
                .ok_or_else(crate::Error::forbidden)?
                .try_native_media_read(&original, None)
        })
        .unwrap();
    let until = Instant::now() + Duration::from_secs(2);
    let first = loop {
        match snapshot.admit() {
            Ok(io) => break io,
            Err(error) if error.code == "authority-busy" && Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(error) => panic!("original old source publication setup refused: {error}"),
        }
    };
    let stamp = first.snapshot_stamp().unwrap();
    let epoch = first.packet_epoch().unwrap();
    drop(first);
    let candidate = format!("replacement_{}", origin.record_id());
    let prepare = || {
        let read = store.try_native_replacement_preparation_read(
            &origin,
            &crate::NativeReplacementSourceOrigin::Native(sources[0].clone()),
            &sources[1],
        )?;
        let mut request = request();
        request.candidate_connection_id = candidate.clone();
        store.prepare_native_replacement(read, request)
    };
    let reserved = prepare();
    let original_io = snapshot.admit_original(stamp).ok();
    let replayed = prepare();
    let epoch_preserved = epoch.current() && original_io.as_ref().is_some_and(|io| io.current());
    drop(original_io);
    drop(snapshot);
    watcher.shutdown().unwrap();
    let reserved = reserved.unwrap().unwrap();
    assert_eq!(reserved.record_id(), origin.record_id());
    assert_eq!(
        reserved.plan().unwrap()["candidate_connection_id"],
        candidate
    );
    let replayed = replayed.unwrap().unwrap();
    assert_eq!(reserved.plan().unwrap(), replayed.plan().unwrap());
    assert_eq!(reserved.expires_at(), replayed.expires_at());
    assert_eq!(
        preparation_effect_state(&store, origin.record_id()),
        (1, 0, 0)
    );
    assert!(epoch_preserved, "fresh generated reservation revoked the queued old-source epoch without changing existing source facts");
}

#[test]
fn fresh_reservation_permit_refuses_victims_and_other_lease_or_lineage() {
    use crate::authority_writer_boundary::phase_ledger::reservation::Reservation;
    use crate::{AuthorityMode, AuthorityScope};
    let (f, _, _, origin, _) = setup();
    let record = origin.record_id();
    let (owner, cid, victim): (String, String, String) = f.store.connection().unwrap().query_row(
        "SELECT o.owner_id,o.conversation_id,(SELECT id FROM operation_effects e WHERE e.operation_record_id!=o.record_id LIMIT 1) FROM operations o WHERE o.record_id=?1",
        [record], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).unwrap();
    let held = f
        .store
        .try_acquire_authority(&[
            AuthorityScope::identity(&owner, AuthorityMode::Read).unwrap(),
            AuthorityScope::conversation(&cid, AuthorityMode::Read).unwrap(),
        ])
        .unwrap();
    let candidate = format!("replacement_{record}");
    let expires = crate::now() + 5;
    let reservation = |effect| Reservation {
        record,
        credential: &origin.credential,
        runtime: &origin.runtime,
        original_digest: &origin.digest,
        reservation: "fixture-fresh-reservation",
        candidate: &candidate,
        candidate_runtime: "fixture-fresh-runtime",
        generation: 2,
        plan: "{}",
        selection: "{}",
        plan_digest: "fixture-fresh-digest",
        expires,
        attempt: "fixture-fresh-attempt",
        effect,
    };
    let mut sql = f.store.connection().unwrap();
    let tx = sql
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let update = "UPDATE operations SET state='preparing',lease_owner=?2,lease_expires_at=?3,worker_generation=worker_generation+1,worker_runtime=?4,worker_cleanup=0 WHERE record_id=?1";
    let exact = params![
        record,
        "fixture-fresh-reservation",
        expires,
        f.store.transition_runtime
    ];
    assert!(
        tx.execute(update, exact).is_err(),
        "registered Read alone cannot reserve a lineage"
    );
    let boundary = &f.store.authority_writer_boundary;
    assert!(
        boundary
            .native_fresh_reservation_ledger(&f.store, &tx, &held, reservation(&victim))
            .is_err(),
        "a colliding effect cannot borrow another original row"
    );
    let permit = boundary
        .native_fresh_reservation_ledger(&f.store, &tx, &held, reservation("fixture-fresh-effect"))
        .unwrap();
    assert!(tx
        .execute(
            update,
            params![record, "forged-lease", expires, f.store.transition_runtime]
        )
        .is_err());
    assert_eq!(tx.execute(update, exact).unwrap(), 1);
    let insert = "INSERT INTO native_replacements(operation_record_id,credential_id,runtime_epoch,original_digest,reservation_id,candidate_connection_id,candidate_runtime_id,generation,plan_json,selection_json,plan_digest,expires_at) VALUES(?1,?2,?3,?4,'fixture-fresh-reservation',?5,'fixture-fresh-runtime',2,'{}','{}','fixture-fresh-digest',?6)";
    assert!(tx
        .execute(
            insert,
            params![
                record,
                origin.credential,
                origin.runtime,
                origin.digest,
                "forged-lineage",
                expires
            ]
        )
        .is_err());
    assert!(tx.execute("INSERT OR REPLACE INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state) VALUES(?1,?2,'native-replacement-prepare',?2,'none','prepared')",params![victim,record]).is_err());
    assert!(tx.query_row("SELECT EXISTS(SELECT 1 FROM operation_effects WHERE id=?1 AND operation_record_id!=?2)",params![victim,record],|row|row.get::<_,bool>(0)).unwrap());
    drop(permit);
    tx.rollback().unwrap();
    drop(sql);
    drop(held);
    assert_eq!(preparation_effect_state(&f.store, record), (0, 0, 0));
}
