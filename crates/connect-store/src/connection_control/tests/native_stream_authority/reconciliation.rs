//! Controlled ledger custody and source authorization, not physical readiness.
use super::*;
use connect_protocol::transitions::{OperationAction, OperationState};
#[test]
fn native_reconciliation_successor_keeps_unknown_custody_and_refuses_old_origins_and_receipts() {
    let (f, _, origin) = selected();
    let source = f.store.try_native_reconciliation_read(&origin).unwrap();
    let lease = f
        .store
        .begin_native_reconciliation(&source, "restart-reconcile", json!({"streams":[]}))
        .unwrap();
    let record = lease.record_id().to_owned();
    let receipt = lease.committed(3, true, None).unwrap();
    let path = f.store.path().to_owned();
    let key = f.keys[0].clone();
    drop(source);
    drop(f.store);
    let successor = Store::open(&path, Limits::default()).unwrap();
    assert_eq!(successor.recover_native_streams(256).unwrap(), 1);
    assert_eq!(successor.recover_native_streams(256).unwrap(), 0);
    assert!(successor.complete_native_stream(&receipt).is_err());
    assert!(successor.try_native_reconciliation_read(&origin).is_err());
    let status = successor
        .execute_operation_action(
            &key,
            OperationAction::Get {
                operation_id: "restart-reconcile".into(),
            },
        )
        .unwrap();
    assert_eq!(status.record_id, record);
    assert_eq!(status.state, OperationState::Unknown);
    assert!(status.subscription_record_id.is_some());
    assert!(status.result.is_none());
    assert_eq!(status.effects.len(), 1);
    assert_eq!(
        status.effects[0].state,
        connect_protocol::transitions::EffectState::Unknown
    );
}
fn selected() -> (
    Fixture,
    PeerAuthorityRequest,
    std::sync::Arc<crate::NativeMediaOrigin>,
) {
    selected_with_end(false)
}
fn selected_with_end(
    end: bool,
) -> (
    Fixture,
    PeerAuthorityRequest,
    std::sync::Arc<crate::NativeMediaOrigin>,
) {
    selected_with_profile(end, false)
}
fn selected_with_profile(
    end: bool,
    data: bool,
) -> (
    Fixture,
    PeerAuthorityRequest,
    std::sync::Arc<crate::NativeMediaOrigin>,
) {
    let (f, mut request, _, origin) = parent_permissions_origin(true, true, end, data, true);
    if data {
        let PeerCommand::NativeStream(wire) = &mut request.command else {
            panic!()
        };
        wire.operation_id = "select-data".into();
        wire.payload = json!({"subscriptions":[{"from_participant":f.pids[1],"kinds":["data"]}]});
    }
    assert!(f.store.try_native_reconciliation_read(&origin).is_err());
    let proof = admit(&f, &request).unwrap();
    f.store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    let (lease, _) = f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap();
    // A fabricated engine observation is permitted only in this controlled
    // Store fixture. Production obtains the receipt from the actual Core call.
    let receipt = lease.committed(2, false, None).unwrap();
    f.store.complete_native_stream(&receipt).unwrap();
    drop(proof);
    (f, request, origin)
}

#[test]
fn terminal_admission_waits_only_for_managed_reconciliation_without_creating_an_effect() {
    let (f, mut end, origin) = selected_with_end(true);
    let source = f.store.try_native_reconciliation_read(&origin).unwrap();
    let managed = f
        .store
        .begin_native_reconciliation(&source, "managed-before-end", json!({"streams":[]}))
        .unwrap();
    let PeerCommand::NativeStream(wire) = &mut end.command else {
        panic!()
    };
    wire.operation_id = "same-terminal-intent".into();
    wire.command = Command::End;
    wire.expected_revision = None;
    wire.payload = json!({"reason_code":200,"reason":"normal-clearing"});
    let proof = admit(&f, &end).unwrap();
    let before = (
        f.count("operations"),
        f.count("operation_effects"),
        f.count("journal"),
    );
    // The old immediate API reports a conflict for this legitimate race.
    assert_eq!(
        f.store
            .accept_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap_err()
            .code,
        "conflict"
    );
    for _ in 0..3 {
        assert!(f
            .store
            .try_accept_native_control_with_peer(&f.keys[0], &proof)
            .unwrap()
            .is_none());
        assert_eq!(
            (
                f.count("operations"),
                f.count("operation_effects"),
                f.count("journal")
            ),
            before
        );
        assert!(
            f.store.try_connection().is_ok(),
            "No SQL custody across a wait"
        );
    }
    f.store
        .complete_native_stream(&managed.committed(3, true, None).unwrap())
        .unwrap();
    let accepted = f
        .store
        .try_accept_native_control_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap();
    assert_eq!(accepted.state, OperationState::Accepted);
    assert_eq!(accepted.operation_id, "same-terminal-intent");
    assert_eq!(f.count("operations"), before.0 + 1);
    let replay = f
        .store
        .try_accept_native_control_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap();
    assert_eq!(replay.record_id, accepted.record_id);
    assert_eq!(f.count("operations"), before.0 + 1);
    assert_eq!(
        f.count("operation_effects"),
        before.1,
        "Admission has no engine attempt"
    );
}

#[test]
fn terminal_admission_keeps_competing_peer_controls_and_changed_intents_as_conflicts() {
    let (f, mut request, _) = selected_with_end(true);
    let PeerCommand::NativeStream(wire) = &mut request.command else {
        panic!()
    };
    wire.operation_id = "competing-selector".into();
    wire.expected_revision = Some(2);
    let competitor = admit(&f, &request).unwrap();
    f.store
        .accept_native_stream_with_peer(&f.keys[0], &competitor)
        .unwrap();
    let PeerCommand::NativeStream(wire) = &mut request.command else {
        panic!()
    };
    wire.command = Command::End;
    wire.expected_revision = None;
    wire.payload = json!({"reason_code":200,"reason":"normal-clearing"});
    // Reusing the competitor ID with a different exact intent must be a
    // conflict even while a managed-wait capable terminal caller uses it.
    assert_eq!(admit(&f, &request).err().unwrap().code, "conflict");
    let PeerCommand::NativeStream(wire) = &mut request.command else {
        panic!()
    };
    wire.operation_id = "terminal-behind-peer".into();
    let end = admit(&f, &request).unwrap();
    assert_eq!(
        f.store
            .try_accept_native_control_with_peer(&f.keys[0], &end)
            .unwrap_err()
            .code,
        "conflict"
    );
}
fn status(f: &Fixture, id: &str) -> connect_protocol::transitions::OperationRecord {
    f.store
        .execute_operation_action(
            &f.keys[0],
            OperationAction::Get {
                operation_id: id.into(),
            },
        )
        .unwrap()
}
#[test]
fn native_reconciliation_requires_completed_selector_and_current_exact_original_source() {
    let (f, _, origin) = selected();
    let source = f.store.try_native_reconciliation_read(&origin).unwrap();
    assert_eq!(source.stream_revision(), 2);
    let lease = f
        .store
        .begin_native_reconciliation(&source, "server-reconcile-1", json!({"streams":[]}))
        .unwrap();
    f.store
        .validate_native_reconciliation(&source, &lease)
        .unwrap();
    let row = status(&f, "server-reconcile-1");
    assert_eq!(row.command_type, "stream.reconcile");
    assert_eq!(row.state, OperationState::Preparing);
    assert_eq!(
        row.subscription_record_id.as_deref(),
        Some(source.subscription_record_id())
    );
    assert!(
        row.result.is_none(),
        "Parent identity is available before confirmation"
    );
    let public = serde_json::to_value(&row).unwrap();
    assert_eq!(
        public["subscription_record_id"],
        source.subscription_record_id()
    );
    assert!(public.get("plan").is_none());
    assert!(public.get("intent").is_none());
    assert_eq!(row.effects.len(), 1);
    assert_eq!(row.effects[0].correlation_key, row.record_id);
    assert!(f
        .store
        .begin_native_reconciliation(&source, "server-reconcile-1", json!({"streams":[]}))
        .is_err());
    assert!(f
        .store
        .begin_native_reconciliation(&source, "server-reconcile-2", json!({"streams":[]}))
        .is_err());
    let receipt = lease.committed(3, true, None).unwrap();
    let first = f.store.complete_native_stream(&receipt).unwrap();
    assert_eq!(first.state, OperationState::Completed);
    assert_eq!(first.subscription_record_id, row.subscription_record_id);
    assert_eq!(
        first.result.as_ref().unwrap()["server_reconciliation"],
        true
    );
    assert_eq!(
        first.result.as_ref().unwrap()["subscription_record_id"],
        source.subscription_record_id()
    );
    assert_eq!(f.store.complete_native_stream(&receipt).unwrap(), first);
    assert!(f
        .store
        .begin_native_reconciliation(&source, "stale-source", json!({"streams":[]}))
        .is_err());
    drop(source);
    let source = f.store.try_native_reconciliation_read(&origin).unwrap();
    assert_eq!(source.stream_revision(), 3);
    assert!(f
        .store
        .begin_native_reconciliation(&source, "next-source", json!({"streams":[]}))
        .is_ok());
}
#[test]
fn native_reconciliation_cannot_enter_through_a_signed_peer_command_or_borrow_another_kernel() {
    let (f, mut request, origin) = selected();
    let source = f.store.try_native_reconciliation_read(&origin).unwrap();
    let PeerCommand::NativeStream(wire) = &mut request.command else {
        panic!()
    };
    wire.operation_id = "forged-reconcile".into();
    wire.command = Command::Reconcile;
    wire.expected_revision = Some(2);
    wire.payload =
        json!({"subscription_record_id":source.subscription_record_id(),"plan":{"streams":[]}});
    wire.validate().unwrap();
    assert_eq!(admit(&f, &request).err().unwrap().code, "forbidden");
    let other = Fixture::new();
    assert_eq!(
        other
            .store
            .begin_native_reconciliation(&source, "foreign-source", json!({"streams":[]}))
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    assert_eq!(f.count("operation_effects"), 2);
}
#[test]
fn native_reconciliation_storage_rollback_preserves_the_original_id_and_explicit_plan() {
    let (f, _, origin) = selected();
    let source = f.store.try_native_reconciliation_read(&origin).unwrap();
    let before = (
        f.count("operations"),
        f.count("operation_effects"),
        f.count("journal"),
    );
    let c = f.store.connection().unwrap();
    c.execute_batch("CREATE TRIGGER controlled_reconcile_failure BEFORE INSERT ON operation_effects WHEN NEW.provider='native-stream' BEGIN SELECT RAISE(ABORT,'controlled reconciliation storage failure'); END;").unwrap();
    drop(c);
    assert!(f
        .store
        .begin_native_reconciliation(&source, "same-reconcile", json!({"streams":["camera"]}))
        .is_err());
    assert_eq!(
        (
            f.count("operations"),
            f.count("operation_effects"),
            f.count("journal")
        ),
        before
    );
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER controlled_reconcile_failure")
        .unwrap();
    let lease = f
        .store
        .begin_native_reconciliation(&source, "same-reconcile", json!({"streams":["camera"]}))
        .unwrap();
    let receipt = lease.committed(3, true, None).unwrap();
    let c = f.store.connection().unwrap();
    c.execute_batch("CREATE TRIGGER controlled_reconcile_record_failure BEFORE INSERT ON journal WHEN json_extract(NEW.data_json,'$.operation_id')='same-reconcile' AND json_extract(NEW.data_json,'$.state')='completed' BEGIN SELECT RAISE(ABORT,'controlled captured outcome storage failure'); END;").unwrap();
    drop(c);
    assert!(f.store.complete_native_stream(&receipt).is_err());
    assert_eq!(
        status(&f, "same-reconcile").state,
        OperationState::Preparing
    );
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER controlled_reconcile_record_failure")
        .unwrap();
    assert_eq!(
        f.store.complete_native_stream(&receipt).unwrap().state,
        OperationState::Completed
    );
}

#[test]
fn original_no_effect_recording_releases_managed_admission_only_after_its_commit() {
    for uncertain in [false, true] {
        let (f, mut request, origin) = selected_with_end(true);
        let source = f.store.try_native_reconciliation_read(&origin).unwrap();
        let lease = f
            .store
            .begin_native_reconciliation(
                &source,
                "routing-definitely-failed",
                json!({"streams":[],"resources_changed":false}),
            )
            .unwrap();
        drop(source);
        if uncertain {
            f.store.native_stream_attempt_uncertain(&lease).unwrap();
        }
        let original = status(&f, "routing-definitely-failed");
        // This Store-only fixture supplies the trusted owned-job observation.
        // Production accepts it only after the adapter's opaque no-effect proof.
        let receipt = lease.without_effect(Error::new("atomic-routing-refused", "No graph commit"));
        let PeerCommand::NativeStream(wire) = &mut request.command else {
            panic!()
        };
        wire.command = Command::End;
        wire.operation_id = "original-end-after-failed-routing".into();
        wire.expected_revision = None;
        wire.payload = json!({"reason_code":200,"reason":"normal-clearing"});
        let proof = admit(&f, &request).unwrap();
        let before = (
            f.count("operations"),
            f.count("operation_effects"),
            f.count("journal"),
        );
        f.store.connection().unwrap().execute_batch("CREATE TRIGGER fail_original_routing_receipt BEFORE INSERT ON journal WHEN json_extract(NEW.data_json,'$.operation_id')='routing-definitely-failed' AND json_extract(NEW.data_json,'$.state')='failed' BEGIN SELECT RAISE(ABORT,'controlled failed routing recording'); END;").unwrap();
        assert!(f
            .store
            .record_native_stream_without_effect(&receipt)
            .is_err());
        assert_eq!(status(&f, "routing-definitely-failed"), original);
        assert!(f
            .store
            .try_accept_native_control_with_peer(&f.keys[0], &proof)
            .unwrap()
            .is_none());
        assert_eq!(
            (
                f.count("operations"),
                f.count("operation_effects"),
                f.count("journal")
            ),
            before
        );
        f.store
            .connection()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_original_routing_receipt")
            .unwrap();
        let failed = f
            .store
            .record_native_stream_without_effect(&receipt)
            .unwrap();
        assert_eq!(failed.state, OperationState::Failed);
        assert_eq!(failed.record_id, original.record_id);
        assert_eq!(failed.operation_id, "routing-definitely-failed");
        assert_eq!(
            f.store
                .record_native_stream_without_effect(&receipt)
                .unwrap(),
            failed
        );
        let accepted = f
            .store
            .try_accept_native_control_with_peer(&f.keys[0], &proof)
            .unwrap()
            .unwrap();
        assert_eq!(accepted.operation_id, "original-end-after-failed-routing");
        assert_eq!(
            f.store
                .try_accept_native_control_with_peer(&f.keys[0], &proof)
                .unwrap()
                .unwrap()
                .record_id,
            accepted.record_id
        );
        assert_eq!(f.count("operations"), before.0 + 1);
        assert_eq!(
            f.count("operation_effects"),
            before.1,
            "failed receipt resubmitted an effect"
        );
    }
}
#[test]
fn native_reconciliation_cancellation_cannot_create_a_second_attempt_or_invent_a_failed_commit() {
    let (f, _, origin) = selected();
    let source = f.store.try_native_reconciliation_read(&origin).unwrap();
    let lease = f
        .store
        .begin_native_reconciliation(&source, "cancel-reconcile", json!({"streams":[]}))
        .unwrap();
    drop(source);
    f.store
        .execute_operation_action(
            &f.keys[0],
            OperationAction::Cancel {
                operation_id: "cancel-server-reconcile".into(),
                target_operation_id: "cancel-reconcile".into(),
                reason: None,
            },
        )
        .unwrap();
    let source = f.store.try_native_reconciliation_read(&origin).unwrap();
    assert!(f
        .store
        .validate_native_reconciliation(&source, &lease)
        .is_err());
    assert!(f
        .store
        .begin_native_reconciliation(&source, "replacement-reconcile", json!({"streams":[]}))
        .is_err());
    // Captured actual success is still historical success if cancellation raced
    // the already-running engine. It never authorizes another engine attempt.
    let receipt = lease.committed(3, true, None).unwrap();
    let completed = f.store.complete_native_stream(&receipt).unwrap();
    assert_eq!(completed.state, OperationState::Completed);
    assert!(completed.cancel_requested);
}
#[test]
fn native_reconciliation_fresh_source_keeps_subscription_permission_separate_and_fences_generation()
{
    let (f, _, origin) = selected();
    let c = f.store.connection().unwrap();
    let original: String = c
        .query_row(
            "SELECT operations FROM credentials WHERE id=?1",
            [&f.keys[0]],
            |r| r.get(0),
        )
        .unwrap();
    let mut operations: Vec<String> = serde_json::from_str(&original).unwrap();
    operations.retain(|s| s != "stream.subscribe");
    c.execute(
        "UPDATE credentials SET operations=?2 WHERE id=?1",
        params![f.keys[0], serde_json::to_string(&operations).unwrap()],
    )
    .unwrap();
    drop(c);
    assert!(f.store.try_native_media_read(&origin, None).is_ok());
    assert_eq!(
        f.store
            .try_native_reconciliation_read(&origin)
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    let c = f.store.connection().unwrap();
    c.execute(
        "UPDATE credentials SET operations=?2 WHERE id=?1",
        params![f.keys[0], original],
    )
    .unwrap();
    drop(c);
    assert!(f.store.try_native_reconciliation_read(&origin).is_ok());
    let c = f.store.connection().unwrap();
    c.execute(
        "UPDATE connections SET generation=generation+1,state='retiring' WHERE id=?1",
        [origin.connection_id()],
    )
    .unwrap();
    drop(c);
    assert_eq!(
        f.store
            .try_native_reconciliation_read(&origin)
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    assert_eq!(f.count("operation_effects"), 2);
}

#[test]
fn data_admission_waits_without_a_message_or_effect_then_accepts_the_original_audience_once() {
    let (f, mut request, origin) = selected_with_profile(false, true);
    let selection = admit(&f, &request).unwrap();
    assert_eq!(
        f.store
            .try_accept_native_control_with_peer(&f.keys[0], &selection)
            .unwrap_err()
            .code,
        "forbidden",
        "Managed waiting cannot admit arbitrary revision-sensitive controls"
    );
    drop(selection);
    let source = f.store.try_native_reconciliation_read(&origin).unwrap();
    let managed = f
        .store
        .begin_native_reconciliation(&source, "managed-before-data", json!({"streams":[]}))
        .unwrap();
    let PeerCommand::NativeStream(wire) = &mut request.command else {
        panic!()
    };
    wire.command = Command::Data;
    wire.operation_id = "same-data-intent".into();
    wire.expected_revision = None;
    wire.by_participant = Some(f.pids[0].clone());
    wire.payload = json!({"from":f.pids[0],"msg_id":"same-data-message","label":"application","to":[f.pids[1]],"content_type":"application/json","body":"{\"marker\":true}"});
    let proof = admit(&f, &request).unwrap();
    let before = (
        f.count("operations"),
        f.count("operation_effects"),
        f.count("journal"),
        f.count("native_data_messages"),
    );
    for _ in 0..3 {
        assert!(f
            .store
            .try_accept_native_control_with_peer(&f.keys[0], &proof)
            .unwrap()
            .is_none());
        assert_eq!(
            (
                f.count("operations"),
                f.count("operation_effects"),
                f.count("journal"),
                f.count("native_data_messages")
            ),
            before
        );
    }
    f.store
        .complete_native_stream(&managed.committed(3, true, None).unwrap())
        .unwrap();
    let accepted = f
        .store
        .try_accept_native_control_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap();
    let replay = f
        .store
        .try_accept_native_control_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap();
    assert_eq!(accepted.record_id, replay.record_id);
    assert_eq!(accepted.operation_id, "same-data-intent");
    assert_eq!(f.count("operations"), before.0 + 1);
    assert_eq!(f.count("native_data_messages"), before.3 + 1);
    assert_eq!(f.count("operation_effects"), before.1);
    assert_eq!(f.count("native_data_writes"), 0);
}

#[test]
fn routing_hint_requires_durable_completed_original_selector_and_exposes_no_capture() {
    let (f, _, origin) = selected();
    let source = f.store.try_native_reconciliation_read(&origin).unwrap();
    let parent = source.subscription_record_id().to_owned();
    let lease = f
        .store
        .begin_native_reconciliation(&source, "hint-routing", json!({"streams":[]}))
        .unwrap();
    let record = lease.record_id().to_owned();
    assert!(f
        .store
        .try_native_reconciliation_hint_read(&origin, &record)
        .is_err());
    let completed = f
        .store
        .complete_native_stream(&lease.committed(3, false, None).unwrap())
        .unwrap();
    drop(source);
    let before = (
        f.count("operations"),
        f.count("operation_effects"),
        f.count("journal"),
    );
    let (read, hint) = f
        .store
        .try_native_reconciliation_hint_read(&origin, &record)
        .unwrap();
    assert!(read.current());
    assert_eq!(hint.operation_id, "hint-routing");
    assert_eq!(hint.record_id, completed.record_id);
    assert_eq!(hint.subscription_record_id, parent);
    assert_eq!(hint.expected_revision, 2);
    assert_eq!(hint.stream_revision, 3);
    assert!(!hint.resources_changed);
    let value = serde_json::to_value(hint).unwrap();
    for key in ["result", "stream_bindings", "plan", "intent"] {
        assert!(value.get(key).is_none());
    }
    assert_eq!(
        before,
        (
            f.count("operations"),
            f.count("operation_effects"),
            f.count("journal")
        )
    );
    drop(read);
    assert!(f
        .store
        .try_native_reconciliation_hint_read(&origin, &parent)
        .is_err());
    assert!(f
        .store
        .try_native_reconciliation_hint_read(&origin, "opr_foreign")
        .is_err());
}
#[test]
fn routing_hint_does_not_relabel_a_resource_replacement_as_a_metadata_only_change() {
    let (f, _, origin) = selected();
    let source = f.store.try_native_reconciliation_read(&origin).unwrap();
    let lease = f
        .store
        .begin_native_reconciliation(&source, "hint-replacement", json!({"streams":[]}))
        .unwrap();
    let record = lease.record_id().to_owned();
    f.store
        .complete_native_stream(&lease.committed(3, true, None).unwrap())
        .unwrap();
    drop(source);
    assert!(f
        .store
        .try_native_reconciliation_hint_read(&origin, &record)
        .is_err());
}
#[test]
fn routing_hint_cannot_revive_an_old_physical_origin_after_store_succession() {
    let (f, _, origin) = selected();
    let source = f.store.try_native_reconciliation_read(&origin).unwrap();
    let lease = f
        .store
        .begin_native_reconciliation(&source, "hint-successor", json!({"streams":[]}))
        .unwrap();
    let record = lease.record_id().to_owned();
    f.store
        .complete_native_stream(&lease.committed(3, false, None).unwrap())
        .unwrap();
    drop(source);
    let path = f.store.path().to_owned();
    let key = f.keys[0].clone();
    drop(f.store);
    let successor = Store::open(&path, Limits::default()).unwrap();
    assert!(successor
        .try_native_reconciliation_hint_read(&origin, &record)
        .is_err());
    let status = successor
        .execute_operation_action(
            &key,
            OperationAction::Get {
                operation_id: "hint-successor".into(),
            },
        )
        .unwrap();
    assert_eq!(status.state, OperationState::Completed);
    assert_eq!(status.record_id, record);
}
