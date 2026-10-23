//! Controlled durable custody and current authorization. These fixtures do not
//! qualify physical codec execution or the unfinished production coordinator.
use super::*;
use connect_protocol::{session_codecs::*, transitions::*};
use std::sync::Arc;

fn fixture() -> (Fixture, Arc<crate::NativeMediaOrigin>, String) {
    let (f, _, _, origin) = parent_permissions_origin_filtered(
        false,
        true,
        false,
        false,
        true,
        &[],
        &["connection.update"],
    );
    f.store.install_session_codec_coordinator().unwrap();
    let actor = f.store.revalidate(&f.keys[0]).unwrap();
    let manager = f
        .store
        .issue(
            &actor.identity_id,
            Scope {
                operations: vec![
                    "session.update".into(),
                    "operation.get".into(),
                    "operation.cancel".into(),
                    "session_get".into(),
                ],
                recipients: vec!["*".into()],
            },
            crate::now() + 600,
        )
        .unwrap()
        .id;
    (f, origin, manager)
}
fn command(f: &Fixture, id: &str) -> Request {
    Request {
        operation_id: id.into(),
        conversation_id: f.cid.clone(),
        session_id: "session".into(),
        by_participant: None,
        expected_revision: Some(
            f.store
                .session_view(&f.keys[0], &f.cid, "session")
                .unwrap()
                .topology_revision,
        ),
        action: Action::Renegotiate {
            codec_preferences: vec!["opus".into()],
        },
    }
}
fn queue(
    f: &Fixture,
    manager: &str,
    id: &str,
) -> (
    Operation,
    connect_protocol::session_control::ControlOutcome,
    crate::SessionCodecTarget,
) {
    let operation = Operation::SessionControl {
        request: command(f, id),
    };
    let Outcome::SessionControl(receipt) = f.store.execute(manager, &operation).unwrap() else {
        panic!()
    };
    let reference = receipt.codec_reconciliation.as_ref().unwrap();
    let work = f.store.pending_session_codec_targets(8).unwrap();
    let target = work
        .into_iter()
        .find(|t| t.batch_record_id == reference.record_id)
        .unwrap();
    (operation, receipt, target)
}
fn status(
    f: &Fixture,
    manager: &str,
    receipt: &connect_protocol::session_control::ControlOutcome,
) -> OperationRecord {
    f.store
        .execute_operation_action(
            manager,
            OperationAction::Get {
                operation_id: receipt
                    .codec_reconciliation
                    .as_ref()
                    .unwrap()
                    .operation_id
                    .clone(),
            },
        )
        .unwrap()
}
fn counts(row: &OperationRecord) -> ReconciliationCounts {
    serde_json::from_value::<ReconciliationOutcome>(row.result.clone().unwrap())
        .unwrap()
        .counts
}
fn observation() -> Vec<connect_protocol::native_streams::CodecBinding> {
    vec![connect_protocol::native_streams::CodecBinding {
        stream_id: "audio".into(),
        medium: "audio".into(),
        direction: "sendrecv".into(),
        codec: json!({"name":"opus","params":{"sample_rate":48000,"channels":1,"payload_type":111}}),
        stream_local_id: 9,
        stream_generation: 2,
        connection_generation: 1,
    }]
}
#[test]
fn session_codec_policy_and_targets_commit_once_with_separate_manager_and_peer_grants() {
    let (f, origin, manager) = fixture();
    let (operation, receipt, target) = queue(&f, &manager, "policy-one");
    let row = status(&f, &manager, &receipt);
    assert_eq!(row.state, OperationState::Accepted);
    assert!(row.result.is_none());
    assert!(!serde_json::to_string(&row)
        .unwrap()
        .contains(origin.connection_id()));
    let read = f
        .store
        .try_native_session_codec_read(&origin, &target)
        .unwrap();
    assert!(!read.media().permits_subscriptions());
    assert!(!read
        .credential_metadata()
        .actor
        .permits("connection.update"));
    assert_ne!(read.credential_metadata().actor.credential_id, manager);
    let lease = f
        .store
        .begin_native_session_codec(&read, "child-one", json!({"streams":[]}))
        .unwrap();
    f.store
        .validate_native_session_codec(&read, &lease)
        .unwrap();
    assert!(f
        .store
        .begin_native_session_codec(&read, "child-one", json!({"streams":[]}))
        .is_err());
    assert!(f
        .store
        .begin_native_session_codec(&read, "child-two", json!({"streams":[]}))
        .is_err());
    assert_eq!(f.count("operation_effects"), 2); // Original offer and this one child.
    let captured = lease
        .session_codecs_committed(2, true, observation())
        .unwrap();
    let first = f.store.complete_native_stream(&captured).unwrap();
    assert_eq!(
        first.result.as_ref().unwrap()["session_codec_batch"],
        target.batch_record_id
    );
    assert_eq!(f.store.complete_native_stream(&captured).unwrap(), first);
    drop(read);
    let done = status(&f, &manager, &receipt);
    assert_eq!(done.state, OperationState::Completed);
    assert_eq!(counts(&done).updated, 1);
    assert_eq!(
        f.store.execute(&manager, &operation).unwrap(),
        Outcome::SessionControl(receipt)
    );
    assert!(f.store.pending_session_codec_targets(8).unwrap().is_empty());
    assert!(f
        .store
        .try_native_session_codec_read(&origin, &target)
        .is_err());
}
#[test]
fn session_codec_no_change_is_recorded_without_advancing_the_native_revision() {
    let (f, origin, manager) = fixture();
    let (_, receipt, target) = queue(&f, &manager, "unchanged-policy");
    let read = f
        .store
        .try_native_session_codec_read(&origin, &target)
        .unwrap();
    let lease = f
        .store
        .begin_native_session_codec(
            &read,
            "unchanged-child",
            json!({"streams":[],"resources_changed":false}),
        )
        .unwrap();
    let captured = lease
        .session_codecs_committed(1, false, observation())
        .unwrap();
    f.store.complete_native_stream(&captured).unwrap();
    drop(read);
    let row = status(&f, &manager, &receipt);
    assert_eq!(row.state, OperationState::Completed);
    assert_eq!(counts(&row).unchanged, 1);
    assert_eq!(counts(&row).updated, 0);
    let revision: u64 = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1",
            [origin.connection_id()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(revision, 1);
}
#[test]
fn session_codec_latest_policy_supersedes_only_unstarted_work_and_replay_keeps_original_receipt() {
    let (f, origin, manager) = fixture();
    let (old, receipt, old_target) = queue(&f, &manager, "old-policy");
    let (_, latest, new_target) = queue(&f, &manager, "new-policy");
    assert!(f
        .store
        .try_native_session_codec_read(&origin, &old_target)
        .is_err());
    assert_eq!(
        f.store.execute(&manager, &old).unwrap(),
        Outcome::SessionControl(receipt.clone())
    );
    let saved: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT codec_policy_operation_id FROM session_control WHERE session_id='session'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(saved, "new-policy");
    f.store.reconcile_session_codec_jobs(8).unwrap();
    let old = status(&f, &manager, &receipt);
    assert_eq!(old.state, OperationState::Completed);
    assert_eq!(counts(&old).superseded, 1);
    assert_eq!(
        status(&f, &manager, &latest).state,
        OperationState::Preparing
    );
    let mut forged = new_target.clone();
    forged.codec_preferences = vec!["pcma".into()];
    assert!(f
        .store
        .try_native_session_codec_read(&origin, &forged)
        .is_err());
    let mut forged = new_target.clone();
    forged.generation += 1;
    assert!(f
        .store
        .try_native_session_codec_read(&origin, &forged)
        .is_err());
    assert!(f
        .store
        .try_native_session_codec_read(&origin, &new_target)
        .unwrap()
        .current());
    assert_eq!(f.count("operation_effects"), 1);
}
#[test]
fn session_codec_storage_failure_rolls_back_policy_targets_and_native_effect_together() {
    let (f, _, manager) = fixture();
    let before = f.store.session_view(&f.keys[0], &f.cid, "session").unwrap();
    let request = command(&f, "atomic-policy");
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER reject_codec_target BEFORE INSERT ON session_codec_targets BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(f
        .store
        .execute(
            &manager,
            &Operation::SessionControl {
                request: request.clone()
            }
        )
        .is_err());
    assert_eq!(
        f.store.session_view(&f.keys[0], &f.cid, "session").unwrap(),
        before
    );
    assert_eq!(f.count("session_codec_batches"), 0);
    assert_eq!(f.count("session_codec_targets"), 0);
    assert_eq!(f.count("operation_effects"), 1);
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_codec_target")
        .unwrap();
    let Outcome::SessionControl(receipt) = f
        .store
        .execute(&manager, &Operation::SessionControl { request })
        .unwrap()
    else {
        panic!()
    };
    assert!(receipt.codec_reconciliation.is_some());
    assert_eq!(f.count("session_codec_batches"), 1);
}
#[test]
fn session_codec_read_requires_current_manager_and_independent_actual_peer_not_idle_job_metadata() {
    let (f, origin, manager) = fixture();
    let (_, receipt, target) = queue(&f, &manager, "revoked-policy");
    let admin = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&admin, &manager).unwrap();
    assert!(f
        .store
        .try_native_session_codec_read(&origin, &target)
        .is_err());
    assert_eq!(f.count("operation_effects"), 1);
    // Current operator can inspect the original policy, but a copied descriptor
    // never substitutes for its revoked initiating credential when doing IO.
    assert!(f
        .store
        .execute_operation_action(
            &manager,
            OperationAction::Get {
                operation_id: receipt
                    .codec_reconciliation
                    .as_ref()
                    .unwrap()
                    .operation_id
                    .clone()
            }
        )
        .is_err());
    f.store.refuse_session_codec_target(&target, false).unwrap();
    assert_eq!(counts(&status(&f, &f.admin, &receipt)).refused, 1);
}
#[test]
fn session_codec_captured_success_survives_recording_failure_and_uncertainty_without_another_attempt(
) {
    let (f, origin, manager) = fixture();
    let (_, receipt, target) = queue(&f, &manager, "capture-policy");
    let read = f
        .store
        .try_native_session_codec_read(&origin, &target)
        .unwrap();
    let lease = f
        .store
        .begin_native_session_codec(&read, "captured-child", json!({"streams":[]}))
        .unwrap();
    f.store.native_stream_attempt_uncertain(&lease).unwrap();
    let captured = lease
        .session_codecs_committed(2, true, observation())
        .unwrap();
    drop(read);
    f.store.reconcile_session_codec_jobs(8).unwrap();
    assert_eq!(
        status(&f, &manager, &receipt).state,
        OperationState::Unknown
    );
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER reject_codec_capture BEFORE UPDATE OF state ON session_codec_targets WHEN NEW.state='updated' BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(f.store.complete_native_stream(&captured).is_err());
    assert_eq!(
        status(&f, &manager, &receipt).state,
        OperationState::Unknown
    );
    assert!(f
        .store
        .try_native_session_codec_read(&origin, &target)
        .is_err());
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_codec_capture")
        .unwrap();
    f.store.complete_native_stream(&captured).unwrap();
    let done = status(&f, &manager, &receipt);
    assert_eq!(done.state, OperationState::Completed);
    assert_eq!(counts(&done).updated, 1);
    assert_eq!(f.count("operation_effects"), 2);
}

mod recovery;
