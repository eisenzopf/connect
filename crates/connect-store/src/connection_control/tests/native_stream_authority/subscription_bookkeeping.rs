//! Exact ledger birth is not a media grant. Captured outcomes retain real Write.
use super::*;
use connect_protocol::transitions::OperationState;

#[test]
fn native_subscription_birth_preserves_media_read_then_records_original_once_with_writer() {
    for command in [Command::Subscribe, Command::Unsubscribe] {
        let (f, mut request, _, origin) = parent_permissions_origin(true, true, false, false, true);
        let PeerCommand::NativeStream(wire) = &mut request.command else {
            panic!()
        };
        wire.command = command;
        if command == Command::Unsubscribe {
            wire.payload = json!({"strm_ids":["audio"]});
        }
        let proof = admit(&f, &request).unwrap();
        let media = f.store.try_native_media_read(&origin, None).unwrap();
        let before = f.store.authority.test_admission_attempts();
        let accepted = f
            .store
            .accept_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap();
        let (lease, started) = f
            .store
            .begin_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap()
            .unwrap();
        assert_eq!(accepted.state, OperationState::Accepted);
        assert_eq!(started.state, OperationState::Preparing);
        assert_eq!(
            f.store.authority.test_admission_attempts(),
            before,
            "namespace birth must not acquire or queue a writer under the original Read"
        );
        assert!(media.current());
        assert!(f
            .store
            .begin_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap()
            .is_none());
        let receipt = lease.committed(2, true, None).unwrap();
        assert_eq!(
            f.store.complete_native_stream(&receipt).unwrap_err().code,
            "authority-busy"
        );
        assert_eq!(operation(&f).state, OperationState::Preparing);
        assert_eq!(f.count("operation_effects"), 2);
        drop(media);
        drop(proof);
        // Exactly the same immutable engine receipt, no second Core attempt.
        let complete = f.store.complete_native_stream(&receipt).unwrap();
        assert_eq!(complete.state, OperationState::Completed);
        assert_eq!(f.store.complete_native_stream(&receipt).unwrap(), complete);
        assert_eq!(f.count("operation_effects"), 2);
        assert!(f
            .store
            .try_native_media_read(&origin, None)
            .unwrap()
            .current());
        let c = f.store.connection().unwrap();
        let (generation, state, revision):(u64,String,u64) = c.query_row(
            "SELECT generation,state,json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id='connection-native'",
            [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!((generation, state, revision), (1, "ready".into(), 2));
    }
}

#[test]
fn native_subscription_birth_refuses_foreign_member_effect_and_authority_edits() {
    let (f, request, _) = parent(true, true);
    let proof = admit(&f, &request).unwrap();
    let accepted = f
        .store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    {
        let c = f.store.connection().unwrap();
        // Exact own member is immutable. Another participant cannot be grafted
        // into the birth exception even though the registered SQL Unit exists.
        assert!(c.execute("INSERT INTO operation_members(operation_record_id,participant_id,identity_id,membership_epoch) SELECT ?1,participant_id,identity_id,membership_epoch FROM participants WHERE participant_id=?2",params![accepted.record_id,f.pids[1]]).is_err());
        assert!(c.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,attempt,submission_generation) VALUES('effect-foreign',?1,'foreign',?1,'none','submitting',1,1)",[&accepted.record_id]).is_err());
        for sql in [
            "UPDATE operations SET cancel_requested=1 WHERE record_id=?1",
            "UPDATE operations SET tombstoned=1 WHERE record_id=?1",
            "UPDATE operations SET expected_revision=2 WHERE record_id=?1",
            "UPDATE operations SET intent_json='{}' WHERE record_id=?1",
        ] {
            assert!(c.execute(sql, [&accepted.record_id]).is_err(), "{sql}");
        }
        for sql in [
            "UPDATE connections SET generation=generation+1 WHERE id='connection-native'",
            "UPDATE connections SET observed_runtime_id='foreign' WHERE id='connection-native'",
            "UPDATE connections SET readiness_json=json_set(readiness_json,'$.stream_revision',2) WHERE id='connection-native'",
            "UPDATE connections SET readiness_json=json_set(readiness_json,'$.codec_bindings',json('[]')) WHERE id='connection-native'",
            "UPDATE connections SET readiness_json=json_set(readiness_json,'$.last_gate',json('{}')) WHERE id='connection-native'",
        ] { assert!(c.execute(sql,[]).is_err(), "{sql}"); }
    }
    let (lease, _) = f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap();
    let c = f.store.connection().unwrap();
    assert!(c.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,attempt,submission_generation) VALUES('effect-second',?1,'native-stream',?1,'none','submitting',1,1)",[&accepted.record_id]).is_err());
    assert!(c
        .execute(
            "UPDATE operation_effects SET state='confirmed' WHERE id=?1",
            [lease.effect_id()]
        )
        .is_err());
    drop(c);
    let raw = rusqlite::Connection::open(f.store.path()).unwrap();
    let error = raw.execute("INSERT INTO operation_members(operation_record_id,participant_id,identity_id,membership_epoch) SELECT ?1,participant_id,identity_id,membership_epoch FROM participants WHERE participant_id=?2",params![accepted.record_id,f.pids[0]]).unwrap_err();
    assert!(error
        .to_string()
        .contains(crate::authority_writer_boundary::FUNCTION));
}

#[test]
fn native_subscription_definitive_preflight_refusal_has_no_effect_and_no_writer() {
    let (f, request, _, origin) = parent_permissions_origin(true, true, false, false, true);
    let proof = admit(&f, &request).unwrap();
    let media = f.store.try_native_media_read(&origin, None).unwrap();
    f.store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    let before = f.store.authority.test_admission_attempts();
    let problem =
        connect_protocol::Error::new("not-found", "Original source unavailable before effect");
    let failed = f
        .store
        .refuse_native_stream_before_effect_with_peer(&f.keys[0], &proof, problem.clone())
        .unwrap();
    assert_eq!(failed.state, OperationState::Failed);
    assert_eq!(f.store.authority.test_admission_attempts(), before);
    assert!(media.current());
    assert_eq!(f.count("operation_effects"), 1);
    assert_eq!(
        f.store
            .refuse_native_stream_before_effect_with_peer(&f.keys[0], &proof, problem)
            .unwrap(),
        failed
    );
    assert!(f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .is_none());
}

#[test]
fn native_subscription_historical_no_effect_and_uncertain_require_original_read_release() {
    for uncertain in [false, true] {
        let (f, request, _) = parent(true, true);
        let proof = admit(&f, &request).unwrap();
        f.store
            .accept_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap();
        let (lease, _) = f
            .store
            .begin_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap()
            .unwrap();
        if uncertain {
            assert_eq!(
                f.store
                    .native_stream_attempt_uncertain(&lease)
                    .unwrap_err()
                    .code,
                "authority-busy"
            );
            drop(proof);
            let recorded = f.store.native_stream_attempt_uncertain(&lease).unwrap();
            assert_eq!(recorded.state, OperationState::Unknown);
            assert_eq!(
                f.store.native_stream_attempt_uncertain(&lease).unwrap(),
                recorded
            );
        } else {
            let receipt = lease.without_effect(connect_protocol::Error::new(
                "not-found",
                "No engine call was made",
            ));
            assert_eq!(
                f.store
                    .record_native_stream_without_effect(&receipt)
                    .unwrap_err()
                    .code,
                "authority-busy"
            );
            drop(proof);
            let recorded = f
                .store
                .record_native_stream_without_effect(&receipt)
                .unwrap();
            assert_eq!(recorded.state, OperationState::Failed);
            assert_eq!(
                f.store
                    .record_native_stream_without_effect(&receipt)
                    .unwrap(),
                recorded
            );
        }
        assert_eq!(f.count("operation_effects"), 2);
    }
}

#[test]
fn native_subscription_captured_outcome_after_revocation_never_revives_retired_generation() {
    let (f, request, parent) = parent(true, true);
    let proof = admit(&f, &request).unwrap();
    f.store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    let (lease, _) = f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap();
    let receipt = lease.committed(2, true, None).unwrap();
    drop(proof);
    {
        let writer = f.controlled_conversation_writer(0);
        let c = f.store.connection_with_authority(&writer).unwrap();
        c.execute(
            "UPDATE native_connection_lifetimes SET retired=1 WHERE operation_record_id=?1",
            [&parent],
        )
        .unwrap();
        c.execute("UPDATE connections SET state='retired',generation=generation+1 WHERE id='connection-native'",[]).unwrap();
        c.execute("UPDATE credentials SET revoked=1 WHERE id=?1", [&f.keys[0]])
            .unwrap();
    }
    let complete = f.store.complete_native_stream(&receipt).unwrap();
    assert_eq!(complete.state, OperationState::Completed);
    assert_eq!(f.store.complete_native_stream(&receipt).unwrap(), complete);
    let c = f.store.connection().unwrap();
    let observed:(String,u64,u64) = c.query_row("SELECT state,generation,json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id='connection-native'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(observed, ("retired".into(), 2, 1));
    for sql in [
        "UPDATE operations SET state='preparing' WHERE record_id=?1",
        "UPDATE operations SET result_json='{}' WHERE record_id=?1",
        "UPDATE operations SET cancel_requested=1 WHERE record_id=?1",
    ] {
        assert!(c.execute(sql, [&complete.record_id]).is_err(), "{sql}");
    }
}

#[test]
fn native_control_family_birth_never_reacquires_a_writer_under_original_read() {
    for (name, command, payload) in [
        (
            "end",
            Command::End,
            json!({"reason_code":1000,"reason":"finished"}),
        ),
        (
            "gate",
            Command::Update,
            json!({"action":"mute","streams":["audio"]}),
        ),
        (
            "codec",
            Command::Update,
            json!({"action":"codec-renegotiate","streams":["audio"],"codec_preferences":["opus"]}),
        ),
        (
            "quality",
            Command::Update,
            json!({"action":"quality","quality_interval_ms":400}),
        ),
        (
            "dtmf",
            Command::Dtmf,
            json!({"digits":"12","duration_ms":100,"method":"info"}),
        ),
        (
            "advertisement",
            Command::Advertise,
            json!({"capabilities":{"audio_codecs":[{"name":"pcmu","params":{}}]},"trigger":"endpoint-change"}),
        ),
    ] {
        let (f, mut request, _, _) = parent_permissions_origin_extra(
            true,
            true,
            true,
            false,
            true,
            &["connection.update", "dtmf.send", "capability.advertise"],
        );
        let PeerCommand::NativeStream(wire) = &mut request.command else {
            panic!()
        };
        wire.operation_id = format!("birth-{name}");
        wire.command = command;
        wire.payload = payload;
        if command == Command::End || command == Command::Dtmf {
            wire.expected_revision = None;
        }
        if command == Command::Advertise {
            wire.by_participant = Some(f.pids[0].clone());
            wire.payload["by_participant"] = json!(f.pids[0]);
        }
        let proof = admit(&f, &request).unwrap();
        let before = f.store.authority.test_admission_attempts();
        let accepted = f
            .store
            .accept_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap();
        let (lease, started) = f
            .store
            .begin_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap()
            .unwrap();
        assert_eq!(accepted.state, OperationState::Accepted, "{name}");
        assert_eq!(started.state, OperationState::Preparing, "{name}");
        assert_eq!(
            f.store.authority.test_admission_attempts(),
            before,
            "{name}"
        );
        assert!(
            f.store
                .begin_native_stream_with_peer(&f.keys[0], &proof)
                .unwrap()
                .is_none(),
            "{name}"
        );
        assert_eq!(f.count("operation_effects"), 2, "{name}");
        assert_eq!(started.effects[0].id, lease.effect_id(), "{name}");
    }
}

#[test]
fn native_data_birth_preserves_original_message_and_one_attempt_under_same_read() {
    let (f, request, _) = data_request(true);
    let proof = admit(&f, &request).unwrap();
    let before = f.store.authority.test_admission_attempts();
    let accepted = f
        .store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    assert_eq!(accepted.state, OperationState::Accepted);
    assert_eq!(
        f.store
            .accept_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap(),
        accepted
    );
    let (lease, started) = f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap();
    assert_eq!(started.state, OperationState::Preparing);
    assert_eq!(f.store.authority.test_admission_attempts(), before);
    assert_eq!(f.count("messages"), 1);
    assert_eq!(f.count("native_data_messages"), 1);
    assert_eq!(f.count("native_data_recipients"), 1);
    assert_eq!(f.count("operation_effects"), 2);
    assert!(f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .is_none());
    let receipt = lease.queued().unwrap();
    // Successful queue attribution changes only this message's own ledger.
    // The exact positive Data path deliberately retains the initiating Read;
    // uncertainty/cancellation still have their separate Writer controls.
    assert!(proof.current());
    let complete = f.store.complete_native_stream(&receipt).unwrap();
    assert_eq!(complete.state, OperationState::Completed);
    assert_eq!(complete.record_id, started.record_id);
    assert_eq!(complete.operation_id, started.operation_id);
    assert_eq!(complete.effects.len(), 1);
    assert_eq!(complete.effects[0].id, started.effects[0].id);
    assert_eq!(complete.effects[0].attempt, 1);
    assert_eq!(complete.effects[0].submission_generation, 1);
    assert_eq!(
        complete.result.as_ref().unwrap()["message_id"],
        "application-message"
    );
    assert_eq!(
        complete.result.as_ref().unwrap()["core_queue_admission"],
        true
    );
    assert!(proof.current());
    // Known attribution must also be read-only under the SAME original proof.
    // A real external writer prevents silently reacquiring a write transaction.
    let mut external = rusqlite::Connection::open(f.store.path()).unwrap();
    let writer = external
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    assert_eq!(f.store.complete_native_stream(&receipt).unwrap(), complete);
    writer.rollback().unwrap();
    assert!(proof.current());
    assert_eq!(f.count("messages"), 1);
    assert_eq!(f.count("native_data_messages"), 1);
    assert_eq!(f.count("native_data_recipients"), 1);
    assert_eq!(f.count("operation_effects"), 2);
    drop(proof);
    assert_eq!(f.store.complete_native_stream(&receipt).unwrap(), complete);
}

#[test]
fn native_reconciliation_birth_keeps_original_selector_and_exact_original_read() {
    let (f, request, _, origin) = parent_permissions_origin(true, true, false, false, true);
    let proof = admit(&f, &request).unwrap();
    f.store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    let (lease, _) = f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap();
    let receipt = lease.committed(2, true, None).unwrap();
    drop(proof);
    let selection = f.store.complete_native_stream(&receipt).unwrap();
    let source = f.store.try_native_reconciliation_read(&origin).unwrap();
    let before = f.store.authority.test_admission_attempts();
    let lease = f
        .store
        .begin_native_reconciliation(
            &source,
            "reconcile-birth",
            json!({"fixture":"exact original plan"}),
        )
        .unwrap();
    assert_eq!(f.store.authority.test_admission_attempts(), before);
    assert_eq!(source.subscription_record_id(), selection.record_id);
    assert_eq!(source.stream_revision(), 2);
    assert!(source.current());
    f.store
        .validate_native_reconciliation(&source, &lease)
        .unwrap();
    assert!(f
        .store
        .begin_native_reconciliation(
            &source,
            "reconcile-birth",
            json!({"fixture":"exact original plan"})
        )
        .is_err());
    assert_eq!(f.count("operation_effects"), 3);
}

#[test]
fn captured_stream_outcomes_recover_journal_capacity_without_replaying_original_attempt() {
    enum Pending {
        Committed(crate::NativeStreamCommitted),
        NoEffect(crate::NativeStreamNoEffect),
        Uncertain(crate::NativeStreamLease),
    }
    for mode in ["committed", "no-effect", "uncertain"] {
        let (mut f, request, _) = parent(true, true);
        let proof = admit(&f, &request).unwrap();
        f.store
            .accept_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap();
        let (lease, _) = f
            .store
            .begin_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap()
            .unwrap();
        let pending = match mode {
            "committed" => Pending::Committed(lease.committed(2, true, None).unwrap()),
            "no-effect" => Pending::NoEffect(
                lease.without_effect(Error::new("not-recipient", "No media effect submitted")),
            ),
            _ => Pending::Uncertain(lease),
        };
        drop(proof);
        let record = |store: &Store| match &pending {
            Pending::Committed(receipt) => store.complete_native_stream(receipt),
            Pending::NoEffect(receipt) => store.record_native_stream_without_effect(receipt),
            Pending::Uncertain(lease) => store.native_stream_attempt_uncertain(lease),
        };
        let before = Store::inspect_snapshot(f.store.path()).unwrap();
        let budget = f.store.limits.journal_events;
        // Test-only deterministic budget pressure reaches the actual journal
        // guard AFTER recording DML. Restoring space must reuse this capture.
        f.store.limits.journal_events = 0;
        let refused = record(&f.store);
        f.store.limits.journal_events = budget;
        assert_eq!(refused.unwrap_err().code, "capacity-exceeded", "{mode}");
        assert_eq!(
            Store::inspect_snapshot(f.store.path()).unwrap(),
            before,
            "{mode}: partial outcome leaked"
        );
        assert_eq!(operation(&f).state, OperationState::Preparing);
        let recorded = record(&f.store).unwrap();
        assert_eq!(
            recorded.state,
            match mode {
                "committed" => OperationState::Completed,
                "no-effect" => OperationState::Failed,
                _ => OperationState::Unknown,
            }
        );
        let after = Store::inspect_snapshot(f.store.path()).unwrap();
        assert_eq!(record(&f.store).unwrap(), recorded);
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), after);
        assert_eq!(f.count("operation_effects"), 2);
        assert_eq!(recorded.effects[0].attempt, 1);
    }
}
