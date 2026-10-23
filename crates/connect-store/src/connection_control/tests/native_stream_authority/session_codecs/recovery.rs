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
        .begin_native_session_codec(&read, "cancel-active-child", json!({"streams":[]}))
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
        .validate_native_session_codec(&read, &lease)
        .is_err());
    let captured = lease
        .session_codecs_committed(2, true, observation())
        .unwrap();
    drop(read);
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
                .begin_native_session_codec(&read, "restart-child", json!({"streams":[]}))
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
