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
        .begin_native_session_codec(read, "child-one", json!({"streams":[]}))
        .unwrap();
    let read = f
        .store
        .try_native_session_codec_attempt_read(&origin, &target, &lease)
        .unwrap();
    f.store
        .validate_native_session_codec_plan(&read, &lease, &json!({"streams":[]}))
        .unwrap();
    drop(read);
    assert!(f
        .store
        .begin_native_session_codec(
            f.store
                .try_native_session_codec_attempt_read(&origin, &target, &lease)
                .unwrap(),
            "child-one",
            json!({"streams":[]}),
        )
        .is_err());
    assert!(f
        .store
        .begin_native_session_codec(
            f.store
                .try_native_session_codec_attempt_read(&origin, &target, &lease)
                .unwrap(),
            "child-two",
            json!({"streams":[]}),
        )
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
            read,
            "unchanged-child",
            json!({"streams":[],"resources_changed":false}),
        )
        .unwrap();
    let captured = lease
        .session_codecs_committed(1, false, observation())
        .unwrap();
    f.store.complete_native_stream(&captured).unwrap();
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
        .begin_native_session_codec(read, "captured-child", json!({"streams":[]}))
        .unwrap();
    f.store.native_stream_attempt_uncertain(&lease).unwrap();
    let captured = lease
        .session_codecs_committed(2, true, observation())
        .unwrap();
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

#[test]
fn session_codec_consuming_reservation_refuses_live_reader_alias_without_creating_an_attempt() {
    let (f, origin, manager) = fixture();
    let (_, receipt, target) = queue(&f, &manager, "reader-drain-policy");
    let read = f
        .store
        .try_native_session_codec_read(&origin, &target)
        .unwrap();
    let original_until = read.media().expires_at();
    let alias = read.media().clone();
    let before = (
        f.count("operations"),
        f.count("operation_effects"),
        f.count("journal"),
    );
    assert_eq!(
        f.store
            .begin_native_session_codec(read, "drained-original-child", json!({"streams":[]}))
            .unwrap_err()
            .code,
        "authority-busy"
    );
    assert_eq!(
        (
            f.count("operations"),
            f.count("operation_effects"),
            f.count("journal")
        ),
        before
    );
    assert!(alias.current());
    assert_eq!(
        status(&f, &manager, &receipt).state,
        OperationState::Accepted
    );
    drop(alias);
    let read = f
        .store
        .try_native_session_codec_read(&origin, &target)
        .unwrap();
    let lease = f
        .store
        .begin_native_session_codec(read, "drained-original-child", json!({"streams":[]}))
        .unwrap();
    assert!(lease.expires_at() <= original_until);
    // The returned original attempt is metadata: no Writer prevents actual
    // current IO admission, and this read is only for the same reserved child.
    let read = f
        .store
        .try_native_session_codec_attempt_read(&origin, &target, &lease)
        .unwrap();
    assert!(read.current());
    f.store
        .validate_native_session_codec_plan(&read, &lease, &json!({"streams":[]}))
        .unwrap();
    drop(read);
    let captured = lease.without_effect(Error::new(
        "codec-refused",
        "Controlled preflight stopped before physical IO",
    ));
    let done = f
        .store
        .record_native_stream_without_effect(&captured)
        .unwrap();
    assert_eq!(
        f.store
            .record_native_stream_without_effect(&captured)
            .unwrap(),
        done
    );
    assert_eq!(f.count("operation_effects"), before.1 + 1);
    assert!(f
        .store
        .try_native_session_codec_read(&origin, &target)
        .is_err());
}

#[test]
fn session_codec_postreservation_refusal_keeps_original_no_effect_custody_after_manager_revocation()
{
    let (f, origin, manager) = fixture();
    let (_, receipt, target) = queue(&f, &manager, "post-reservation-policy");
    let read = f
        .store
        .try_native_session_codec_read(&origin, &target)
        .unwrap();
    let lease = f
        .store
        .begin_native_session_codec(read, "post-reservation-child", json!({"streams":[]}))
        .unwrap();
    let effect = lease.effect_id().to_owned();
    let child = lease.record_id().to_owned();
    let original_until = lease.expires_at();
    let admin = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&admin, &manager).unwrap();
    assert!(f
        .store
        .try_native_session_codec_attempt_read(&origin, &target, &lease)
        .is_err());
    assert!(f
        .store
        .try_native_session_codec_read(&origin, &target)
        .is_err());
    // Original reservation remains owned even though fresh IO was refused.
    // This controlled no-effect receipt does not repeat native preparation.
    let captured = lease.without_effect(Error::new(
        "codec-refused",
        "Original manager was revoked before physical IO",
    ));
    let done = f
        .store
        .record_native_stream_without_effect(&captured)
        .unwrap();
    assert_eq!(done.record_id, child);
    assert_eq!(done.expires_at, Some(original_until));
    assert_eq!(done.effects.len(), 1);
    assert_eq!(done.effects[0].id, effect);
    assert_eq!(done.state, OperationState::Failed);
    assert_eq!(
        f.store
            .record_native_stream_without_effect(&captured)
            .unwrap(),
        done
    );
    f.store.reconcile_session_codec_jobs(8).unwrap();
    assert_eq!(counts(&status(&f, &f.admin, &receipt)).refused, 1);
    assert_eq!(f.count("operation_effects"), 2);
}

#[test]
fn session_codec_fresh_attempt_read_requires_exact_original_stream_and_selected_publisher_plan() {
    let (f, origin, manager) = fixture();
    let (_, _, target) = queue(&f, &manager, "exact-plan-policy");
    let descriptor = json!({"streams":[{"id":"audio","codec":"opus"}],"sources":[{"connection":"original-publisher","generation":1}],"resources_changed":true});
    let read = f
        .store
        .try_native_session_codec_read(&origin, &target)
        .unwrap();
    let lease = f
        .store
        .begin_native_session_codec(read, "exact-plan-child", descriptor.clone())
        .unwrap();
    let read = f
        .store
        .try_native_session_codec_attempt_read(&origin, &target, &lease)
        .unwrap();
    f.store
        .validate_native_session_codec_plan(&read, &lease, &descriptor)
        .unwrap();
    let mut wrong_stream = descriptor.clone();
    wrong_stream["streams"][0]["codec"] = json!("pcmu");
    assert_eq!(
        f.store
            .validate_native_session_codec_plan(&read, &lease, &wrong_stream)
            .unwrap_err()
            .code,
        "conflict"
    );
    let mut wrong_source = descriptor.clone();
    wrong_source["sources"][0]["generation"] = json!(2);
    assert_eq!(
        f.store
            .validate_native_session_codec_plan(&read, &lease, &wrong_source)
            .unwrap_err()
            .code,
        "conflict"
    );
    let mut wrong_target = target.clone();
    wrong_target.generation += 1;
    assert!(f
        .store
        .try_native_session_codec_attempt_read(&origin, &wrong_target, &lease)
        .is_err());
    drop(read);
    let captured = lease.without_effect(Error::new(
        "codec-refused",
        "Controlled changed plan was not executed",
    ));
    f.store
        .record_native_stream_without_effect(&captured)
        .unwrap();
    assert_eq!(f.count("operation_effects"), 2);
}

mod recovery;

#[test]
fn session_codec_reservation_keeps_identity_readers_but_requires_original_conversation_drain() {
    use crate::{AuthorityMode, AuthorityScope};
    let (f, origin, manager) = fixture();
    let (_, _, target) = queue(&f, &manager, "mixed-footprint-policy");
    let actor = f.store.revalidate(&f.keys[0]).unwrap();
    let cid_scope = AuthorityScope::conversation(&f.cid, AuthorityMode::Read).unwrap();
    let cid_reader = f.store.try_acquire_authority(&[cid_scope]).unwrap();
    let read = f
        .store
        .try_native_session_codec_read(&origin, &target)
        .unwrap();
    let refused =
        f.store
            .begin_native_session_codec(read, "mixed-footprint-child", json!({"streams":[]}));
    assert_eq!(refused.err().unwrap().code, "authority-busy");
    assert_eq!(
        f.count("operation_effects"),
        1,
        "a blocked original never reserves a child"
    );
    let c = f.store.connection().unwrap();
    let state:(String,Option<String>) = c.query_row("SELECT state,native_operation_record_id FROM session_codec_targets WHERE batch_record_id=?1 AND connection_id=?2",params![target.batch_record_id,target.connection_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(state, ("queued".into(), None));
    drop(c);
    drop(cid_reader);
    let identity = AuthorityScope::identity(&actor.identity_id, AuthorityMode::Read).unwrap();
    let identity_reader = f.store.try_acquire_authority(&[identity.clone()]).unwrap();
    let read = f
        .store
        .try_native_session_codec_read(&origin, &target)
        .unwrap();
    let original_until = read.media().expires_at();
    let lease = f
        .store
        .begin_native_session_codec(read, "mixed-footprint-child", json!({"streams":[]}))
        .expect("same-room ledger reservation must not exclude unrelated Identity readers");
    assert!(f.store.authority_covers(&identity_reader, &[identity]));
    assert!(
        lease.expires_at() <= original_until,
        "reservation cannot renew the original read cutoff"
    );
    let read = f
        .store
        .try_native_session_codec_attempt_read(&origin, &target, &lease)
        .unwrap();
    f.store
        .validate_native_session_codec_plan(&read, &lease, &json!({"streams":[]}))
        .unwrap();
    assert_eq!(f.count("operation_effects"), 2);
    drop(read);
    drop(identity_reader);
    let captured = lease.without_effect(Error::new(
        "codec-refused",
        "Controlled metadata-only reservation; no physical IO",
    ));
    let done = f
        .store
        .record_native_stream_without_effect(&captured)
        .unwrap();
    assert_eq!(done.effects.len(), 1);
    assert_eq!(done.effects[0].attempt, 1);
    assert_eq!(
        f.store
            .record_native_stream_without_effect(&captured)
            .unwrap(),
        done
    );
    assert_eq!(f.count("operation_effects"), 2);
}

#[cfg(feature = "test-support")]
#[test]
fn subscription_video_withdrawal_changes_only_current_acceptance_under_exact_writer() {
    let (f, origin, _) = fixture();
    let writer = f.controlled_conversation_writer(0);
    // Controlled accepted state only: actual media/revocation is verified by
    // the signed Service fixture. This unit exercises the real mutation guard.
    let (before, endpoint, connection) = {
        let c = f.store.connection_with_authority(&writer).unwrap();
        let raw: String = c.query_row("SELECT cap.capabilities_json FROM session_presence_capabilities cap JOIN connections n ON n.session_id=cap.session_id AND n.participant_id=cap.participant_id AND n.membership_epoch=cap.membership_epoch WHERE n.id=?1", [origin.connection_id()], |r| r.get(0)).unwrap();
        let mut caps: serde_json::Value = serde_json::from_str(&raw).unwrap();
        caps["video"] = json!(["vp8"]);
        c.execute("UPDATE session_presence_capabilities SET capabilities_json=?2 WHERE (session_id,participant_id,membership_epoch)=(SELECT session_id,participant_id,membership_epoch FROM connections WHERE id=?1)", params![origin.connection_id(),caps.to_string()]).unwrap();
        let endpoint: String = c.query_row("SELECT e.capabilities_json FROM endpoints e JOIN connections n ON n.endpoint_id=e.id WHERE n.id=?1", [origin.connection_id()], |r| r.get(0)).unwrap();
        let connection: String = c.query_row("SELECT json_object('state',state,'generation',generation,'revision',revision,'runtime',observed_runtime_id,'readiness',readiness_json,'membership_epoch',membership_epoch) FROM connections WHERE id=?1", [origin.connection_id()], |r| r.get(0)).unwrap();
        (caps, endpoint, connection)
    };
    drop(writer);
    let foreign = f
        .store
        .try_acquire_authority(&[crate::AuthorityScope::conversation(
            "foreign-conversation",
            crate::AuthorityMode::Write,
        )
        .unwrap()])
        .unwrap();
    assert_eq!(
        f.store
            .withdraw_native_session_video_for_test(&foreign, origin.connection_id())
            .unwrap_err()
            .code,
        "forbidden"
    );
    drop(foreign);
    let writer = f.controlled_conversation_writer(0);
    f.store
        .withdraw_native_session_video_for_test(&writer, origin.connection_id())
        .unwrap();
    drop(writer);
    let c = f.store.connection().unwrap();
    let raw: String = c.query_row("SELECT cap.capabilities_json FROM session_presence_capabilities cap JOIN connections n ON n.session_id=cap.session_id AND n.participant_id=cap.participant_id AND n.membership_epoch=cap.membership_epoch WHERE n.id=?1", [origin.connection_id()], |r| r.get(0)).unwrap();
    let mut expected = before;
    expected.as_object_mut().unwrap().remove("video");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&raw).unwrap(),
        expected
    );
    assert_eq!(c.query_row("SELECT e.capabilities_json FROM endpoints e JOIN connections n ON n.endpoint_id=e.id WHERE n.id=?1", [origin.connection_id()], |r| r.get::<_,String>(0)).unwrap(), endpoint);
    assert_eq!(c.query_row("SELECT json_object('state',state,'generation',generation,'revision',revision,'runtime',observed_runtime_id,'readiness',readiness_json,'membership_epoch',membership_epoch) FROM connections WHERE id=?1", [origin.connection_id()], |r| r.get::<_,String>(0)).unwrap(), connection);
    drop(c);
}
