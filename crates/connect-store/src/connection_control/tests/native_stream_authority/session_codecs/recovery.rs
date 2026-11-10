use super::*;

#[test]
fn session_codec_cancel_stops_queued_work_and_retains_submitted_uncertainty_until_capture() {
    let (f, origin, manager) = fixture();
    let (_, queued, target) = queue(&f, &manager, "cancel-queued-policy");
    let decision = f
        .store
        .execute_operation_action(
            &manager,
            OperationAction::Cancel {
                operation_id: "cancel-queued".into(),
                target_operation_id: queued
                    .codec_reconciliation
                    .as_ref()
                    .unwrap()
                    .operation_id
                    .clone(),
                reason: None,
            },
        )
        .unwrap();
    assert!(decision.result.is_some());
    assert_eq!(
        status(&f, &manager, &queued).state,
        OperationState::Cancelled
    );
    assert!(f
        .store
        .try_native_session_codec_read(&origin, &target)
        .is_err());
    assert_eq!(f.count("operation_effects"), 1);
    let (_, active, target) = queue(&f, &manager, "cancel-active-policy");
    let read = f
        .store
        .try_native_session_codec_read(&origin, &target)
        .unwrap();
    let lease = f
        .store
        .begin_native_session_codec(read, "cancel-active-child", json!({"streams":[]}))
        .unwrap();
    f.store
        .execute_operation_action(
            &manager,
            OperationAction::Cancel {
                operation_id: "cancel-active".into(),
                target_operation_id: active
                    .codec_reconciliation
                    .as_ref()
                    .unwrap()
                    .operation_id
                    .clone(),
                reason: None,
            },
        )
        .unwrap();
    assert!(f
        .store
        .try_native_session_codec_attempt_read(&origin, &target, &lease)
        .is_err());
    let captured = lease
        .session_codecs_committed(2, true, observation())
        .unwrap();
    assert_eq!(status(&f, &manager, &active).state, OperationState::Unknown);
    assert!(f
        .store
        .try_native_session_codec_read(&origin, &target)
        .is_err());
    f.store.complete_native_stream(&captured).unwrap();
    let done = status(&f, &manager, &active);
    assert_eq!(done.state, OperationState::Completed);
    assert!(done.cancel_requested);
    assert_eq!(counts(&done).updated, 1);
    assert_eq!(f.count("operation_effects"), 2);
}

#[test]
fn session_codec_restart_cannot_reconstruct_old_queued_or_submitted_native_attempts() {
    for submitted in [false, true] {
        let (f, origin, manager) = fixture();
        let (_, receipt, target) = queue(&f, &manager, "restart-policy");
        let captured = if submitted {
            let read = f
                .store
                .try_native_session_codec_read(&origin, &target)
                .unwrap();
            let lease = f
                .store
                .begin_native_session_codec(read, "restart-child", json!({"streams":[]}))
                .unwrap();
            Some(
                lease
                    .session_codecs_committed(2, true, observation())
                    .unwrap(),
            )
        } else {
            None
        };
        let path = f.store.path().to_owned();
        let operation = receipt
            .codec_reconciliation
            .as_ref()
            .unwrap()
            .operation_id
            .clone();
        drop(f.store);
        let successor = Store::open(&path, Limits::default()).unwrap();
        successor.recover_native_streams(8).unwrap();
        successor.reconcile_session_codec_jobs(8).unwrap();
        assert!(successor
            .pending_session_codec_targets(8)
            .unwrap()
            .is_empty());
        assert!(successor
            .try_native_session_codec_read(&origin, &target)
            .is_err());
        if let Some(captured) = captured {
            assert!(successor.complete_native_stream(&captured).is_err());
        }
        let row = successor
            .execute_operation_action(
                &manager,
                OperationAction::Get {
                    operation_id: operation,
                },
            )
            .unwrap();
        assert_eq!(
            row.state,
            if submitted {
                OperationState::Unknown
            } else {
                OperationState::Completed
            }
        );
        if !submitted {
            assert_eq!(counts(&row).unavailable, 1);
        } else {
            assert!(row.result.is_none());
        }
    }
}

#[test]
fn session_codec_no_effect_capture_retries_recording_without_another_native_attempt() {
    let (f, origin, manager) = fixture();
    let (_, receipt, target) = queue(&f, &manager, "no-effect-policy");
    let read = f
        .store
        .try_native_session_codec_read(&origin, &target)
        .unwrap();
    let lease = f
        .store
        .begin_native_session_codec(read, "no-effect-child", json!({"streams":[]}))
        .unwrap();
    let captured =
        lease.without_effect(Error::new("codec-refused", "Controlled preflight refusal"));
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER reject_codec_no_effect BEFORE UPDATE OF state ON operations WHEN NEW.state='failed' BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(f
        .store
        .record_native_stream_without_effect(&captured)
        .is_err());
    assert_eq!(
        status(&f, &manager, &receipt).state,
        OperationState::Preparing
    );
    assert!(f
        .store
        .try_native_session_codec_read(&origin, &target)
        .is_err());
    assert_eq!(f.count("operation_effects"), 2);
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_codec_no_effect")
        .unwrap();
    let recorded = f
        .store
        .record_native_stream_without_effect(&captured)
        .unwrap();
    assert_eq!(
        f.store
            .record_native_stream_without_effect(&captured)
            .unwrap(),
        recorded
    );
    f.store.reconcile_session_codec_jobs(8).unwrap();
    let done = status(&f, &manager, &receipt);
    assert_eq!(done.state, OperationState::Completed);
    assert_eq!(counts(&done).refused, 1);
    assert_eq!(f.count("operation_effects"), 2);
}

#[test]
fn session_codec_bounded_reconciliation_prioritizes_pending_work_over_older_unknown_batches() {
    let (f, origin, manager) = fixture();
    let (_, older, target) = queue(&f, &manager, "older-unknown-policy");
    let read = f
        .store
        .try_native_session_codec_read(&origin, &target)
        .unwrap();
    let lease = f
        .store
        .begin_native_session_codec(read, "older-unknown-child", json!({"streams":[]}))
        .unwrap();
    f.store.native_stream_attempt_uncertain(&lease).unwrap();
    f.store.reconcile_session_codec_jobs(1).unwrap();
    assert_eq!(status(&f, &manager, &older).state, OperationState::Unknown);
    let (_, newer, _) = queue(&f, &manager, "newer-pending-policy");
    // Controlled physical retirement in this Store-only fixture. The scan
    // must reach the queued target with its one-batch budget; it grants no IO.
    f.change_fixture()
        .execute(
            "UPDATE connections SET state='retired' WHERE id=?1",
            [origin.connection_id()],
        )
        .unwrap();
    f.store.reconcile_session_codec_jobs(1).unwrap();
    let done = status(&f, &manager, &newer);
    assert_eq!(done.state, OperationState::Completed);
    assert_eq!(counts(&done).unavailable, 1);
    assert_eq!(status(&f, &manager, &older).state, OperationState::Unknown);
    assert_eq!(f.count("operation_effects"), 2);
}

#[test]
fn empty_codec_reconciliation_skips_external_writer_then_settles_later_batch_once() {
    let (f, origin, manager) = fixture();
    f.store
        .connection()
        .unwrap()
        .busy_timeout(std::time::Duration::ZERO)
        .unwrap();
    let mut writer = rusqlite::Connection::open(f.store.path()).unwrap();
    let held = writer
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let before = (
        f.count("operations"),
        f.count("operation_effects"),
        f.count("journal"),
    );
    assert_eq!(f.store.reconcile_session_codec_jobs(1).unwrap(), 0);
    assert!(!held.is_autocommit(), "The independent writer remains held");
    assert_eq!(
        (
            f.count("operations"),
            f.count("operation_effects"),
            f.count("journal")
        ),
        before
    );
    held.rollback().unwrap();

    let (_, receipt, target) = queue(&f, &manager, "after-empty-scan-policy");
    let original = status(&f, &manager, &receipt);
    let effects = f.count("operation_effects");
    assert_eq!(target.batch_record_id, original.record_id);
    // Controlled retirement in this Store-only fixture makes queued work
    // settle without fabricating a native effect or changing its identity.
    f.change_fixture()
        .execute(
            "UPDATE connections SET state='retired' WHERE id=?1",
            [origin.connection_id()],
        )
        .unwrap();
    let held = writer
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    assert!(
        f.store.reconcile_session_codec_jobs(1).is_err(),
        "Candidates still require the write transaction"
    );
    held.rollback().unwrap();
    assert_eq!(f.store.reconcile_session_codec_jobs(1).unwrap(), 1);
    assert_eq!(f.store.reconcile_session_codec_jobs(1).unwrap(), 0);
    let completed = status(&f, &manager, &receipt);
    assert_eq!(completed.record_id, original.record_id);
    assert_eq!(completed.operation_id, original.operation_id);
    assert_eq!(completed.state, OperationState::Completed);
    assert_eq!(counts(&completed).unavailable, 1);
    assert_eq!(completed.effects, original.effects);
    assert_eq!(f.count("operation_effects"), effects);
    assert!(f.store.pending_session_codec_targets(1).unwrap().is_empty());
}
