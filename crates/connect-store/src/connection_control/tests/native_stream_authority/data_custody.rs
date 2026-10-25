//! Controlled ledger tests; no native queue call or remote delivery is claimed.
use super::*;
use connect_protocol::transitions::{OperationAction, OperationState};
fn status(f: &Fixture) -> connect_protocol::transitions::OperationRecord {
    f.store
        .execute_operation_action(
            &f.keys[0],
            OperationAction::Get {
                operation_id: "publish-application".into(),
            },
        )
        .unwrap()
}
fn cancel_data(f: &Fixture) {
    f.store
        .execute_operation_action(
            &f.admin,
            OperationAction::Cancel {
                operation_id: "cancel-native-data".into(),
                target_operation_id: "publish-application".into(),
                reason: Some("controlled cancellation".into()),
            },
        )
        .unwrap();
}
fn counts(f: &Fixture) -> [u64; 6] {
    [
        f.count("operations"),
        f.count("messages"),
        f.count("message_audiences"),
        f.count("native_data_messages"),
        f.count("native_data_recipients"),
        f.count("journal"),
    ]
}
#[test]
fn native_data_admission_atomically_keeps_one_message_and_original_audience_without_a_publication_effect(
) {
    let (f, request, _) = data_request(true);
    let proof = admit(&f, &request).unwrap();
    let before = counts(&f);
    let accepted = f
        .store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    assert_eq!(accepted.state, OperationState::Accepted);
    assert!(accepted.effects.is_empty());
    assert_eq!(f.count("messages"), before[1] + 1);
    assert_eq!(f.count("message_audiences"), before[2] + 2);
    assert_eq!(f.count("native_data_messages"), 1);
    assert_eq!(f.count("native_data_recipients"), 1);
    assert_eq!(f.count("operation_effects"), 1); // only the controlled native parent
    assert_eq!(
        f.store
            .accept_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap(),
        accepted
    );
    let c = f.store.connection().unwrap();
    let stored: (String, String, String) = c
        .query_row(
            "SELECT id,body,label FROM messages WHERE id='application-message'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        stored,
        (
            "application-message".into(),
            "{\"hello\":true}".into(),
            "application".into()
        )
    );
    let source: (String, u64) = c
        .query_row(
            "SELECT participant_id,membership_epoch FROM native_data_recipients",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(source, (f.pids[1].clone(), 1));
}
#[test]
fn native_data_queue_receipt_is_local_admission_without_stream_revision_or_delivery_claim() {
    let (f, request, _) = data_request(true);
    let proof = admit(&f, &request).unwrap();
    f.store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    let (lease, started) = f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap();
    assert_eq!(started.state, OperationState::Preparing);
    assert_eq!(started.effects.len(), 1);
    assert!(f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .is_none());
    let receipt = lease.queued().unwrap();
    let completed = f.store.complete_native_stream(&receipt).unwrap();
    assert_eq!(
        completed.result,
        Some(
            json!({"connection_id":"connection-native","message_id":"application-message","core_queue_admission":true})
        )
    );
    assert_eq!(f.store.complete_native_stream(&receipt).unwrap(), completed);
    assert_eq!(
        f.store
            .accept_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap(),
        completed
    );
    assert_eq!(f.count("messages"), 1);
    let c = f.store.connection().unwrap();
    let revision:u64=c.query_row("SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id='connection-native'",[],|r|r.get(0)).unwrap();
    assert_eq!(revision, 1);
}
#[test]
fn native_data_changed_identity_scope_encoding_audience_and_extensions_conflict_before_new_work() {
    let (f, request, _) = data_request(true);
    let proof = admit(&f, &request).unwrap();
    f.store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    let before = counts(&f);
    for field in [
        "msg_id",
        "body",
        "label",
        "to",
        "body_encoding",
        "future",
        "revision",
    ] {
        let mut changed = request.clone();
        let PeerCommand::NativeStream(wire) = &mut changed.command else {
            panic!()
        };
        match field {
            "msg_id" => wire.payload[field] = json!("another-message"),
            "body" => wire.payload[field] = json!("{\"hello\":false}"),
            "label" => wire.payload[field] = json!("another-label"),
            "to" => wire.payload[field] = json!([f.pids[2]]),
            "body_encoding" => wire.payload[field] = json!("utf8"),
            "future" => wire.payload[field] = json!({"keep":false}),
            _ => wire.expected_revision = Some(1),
        }
        assert_eq!(
            admit(&f, &changed).err().unwrap().code,
            "conflict",
            "{field}"
        );
        assert_eq!(counts(&f), before);
    }
}
#[test]
fn native_data_journal_failure_rolls_back_message_audience_and_root_then_same_id_recovers() {
    let (f, request, _) = data_request(true);
    let proof = admit(&f, &request).unwrap();
    let before = counts(&f);
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER fail_native_data_accept BEFORE INSERT ON journal WHEN NEW.kind='operation_accepted' AND EXISTS(SELECT 1 FROM operations o WHERE o.record_id=NEW.operation_record_id AND o.command_type='message.send') BEGIN SELECT RAISE(ABORT,'controlled native message admission failure'); END;").unwrap();
    assert!(f
        .store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .is_err());
    assert_eq!(counts(&f), before);
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_native_data_accept")
        .unwrap();
    assert_eq!(
        f.store
            .accept_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap()
            .state,
        OperationState::Accepted
    );
    assert_eq!(f.count("messages"), 1);
}
#[test]
fn native_data_all_replay_keeps_original_epoch_snapshots_after_presence_changes() {
    let (f, mut request, _) = data_request(true);
    let PeerCommand::NativeStream(wire) = &mut request.command else {
        panic!()
    };
    wire.payload["to"] = json!("all");
    let proof = admit(&f, &request).unwrap();
    let accepted = f
        .store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    f.store.connection().unwrap().execute("UPDATE session_participants SET state='left' WHERE session_id='session' AND participant_id=?1",[&f.pids[1]]).unwrap();
    let projection = f
        .store
        .project_native_data_with_peer(&f.keys[0], &proof)
        .unwrap();
    assert_eq!(projection.recipients.len(), 4);
    assert!(projection
        .recipients
        .iter()
        .all(|p| p.membership_epoch == 1));
    assert_eq!(
        f.store
            .accept_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap(),
        accepted
    );
    assert_eq!(f.count("messages"), 1);
    assert_eq!(f.count("native_data_recipients"), 4);
}
#[test]
fn native_data_cancel_before_publication_keeps_durable_message_and_never_starts_an_effect() {
    let (f, request, _) = data_request(true);
    let proof = admit(&f, &request).unwrap();
    f.store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    cancel_data(&f);
    let cancelled = status(&f);
    assert_eq!(cancelled.state, OperationState::Cancelled);
    assert!(cancelled.effects.is_empty());
    assert!(f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .is_none());
    assert_eq!(
        f.store
            .accept_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap(),
        cancelled
    );
    assert_eq!(f.count("messages"), 1);
}
#[test]
fn native_data_captured_queue_receipt_recovers_after_cancel_and_storage_failure_without_reviving_parent(
) {
    let (f, request, parent) = data_request(true);
    let proof = admit(&f, &request).unwrap();
    f.store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    let (lease, started) = f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap();
    let receipt = lease.queued().unwrap();
    cancel_data(&f);
    assert_eq!(status(&f).state, OperationState::Unknown);
    {
        let c = f.store.connection().unwrap();
        c.execute(
            "UPDATE native_connection_lifetimes SET retired=1 WHERE operation_record_id=?1",
            [parent],
        )
        .unwrap();
        c.execute("UPDATE connections SET state='retired',generation=generation+1 WHERE id='connection-native'",[]).unwrap();
        c.execute_batch("CREATE TRIGGER fail_native_data_complete BEFORE INSERT ON journal WHEN NEW.kind='operation_updated' BEGIN SELECT RAISE(ABORT,'controlled native outcome recording failure'); END;").unwrap();
    }
    assert!(f.store.complete_native_stream(&receipt).is_err());
    assert_eq!(status(&f).state, OperationState::Unknown);
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_native_data_complete")
        .unwrap();
    let complete = f.store.complete_native_stream(&receipt).unwrap();
    assert_eq!(complete.state, OperationState::Completed);
    assert!(complete.cancel_requested);
    assert_eq!(complete.effects[0].id, started.effects[0].id);
    let c = f.store.connection().unwrap();
    let (state,revision):(String,u64)=c.query_row("SELECT state,json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id='connection-native'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!((state.as_str(), revision), ("retired", 1));
}
#[test]
fn native_data_attempt_cannot_be_completed_as_a_subscription() {
    let (f, request, _) = data_request(true);
    let proof = admit(&f, &request).unwrap();
    f.store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    let (lease, _) = f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap();
    assert!(lease.committed(2, true, None).is_err());
    assert_eq!(status(&f).state, OperationState::Preparing);
}
#[test]
fn native_data_shares_message_and_logical_identity_namespaces_with_ordinary_messages() {
    let (f, request, _) = data_request(true);
    let proof = admit(&f, &request).unwrap();
    f.store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    let (lease, _) = f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap();
    f.store
        .complete_native_stream(&lease.queued().unwrap())
        .unwrap();
    let mut duplicate = request.clone();
    let PeerCommand::NativeStream(wire) = &mut duplicate.command else {
        panic!()
    };
    wire.operation_id = "duplicate-message-id".into();
    let duplicate = admit(&f, &duplicate).unwrap();
    assert_eq!(
        f.store
            .accept_native_stream_with_peer(&f.keys[0], &duplicate)
            .unwrap_err()
            .code,
        "conflict"
    );
    let mut message = connect_protocol::messages::MessageDraft::text_all("ordinary");
    message.msg_id = Some("ordinary-message".into());
    assert_eq!(
        f.store
            .execute(
                &f.keys[0],
                &Operation::Send {
                    request_id: "publish-application".into(),
                    conversation_id: f.cid.clone(),
                    message: message.clone()
                }
            )
            .unwrap_err()
            .code,
        "conflict"
    );
    message.msg_id = Some("application-message".into());
    assert_eq!(
        f.store
            .execute(
                &f.keys[0],
                &Operation::Send {
                    request_id: "ordinary-duplicate-message".into(),
                    conversation_id: f.cid.clone(),
                    message
                }
            )
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(f.count("messages"), 1);
}

#[test]
fn native_data_message_does_not_enqueue_an_ambient_provider_route() {
    let (f, request, _) = data_request(true);
    let receiver = f.store.revalidate(&f.keys[1]).unwrap().identity_id;
    f.store
        .configure_route(
            &f.admin,
            &crate::providers::Route {
                id: "native-data-ambient-route".into(),
                identity_id: receiver,
                credential_id: f.keys[1].clone(),
                conversation_id: f.cid.clone(),
                connector: "vapi".into(),
                target: uuid::Uuid::new_v4().to_string(),
                source: None,
                enabled: true,
                incoming_to: connect_protocol::messages::MessageAudience::all(),
            },
        )
        .unwrap();
    let proof = admit(&f, &request).unwrap();
    f.store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    assert_eq!(f.count("provider_jobs"), 0);
    drop(proof);
    // The route is eligible: an ordinary conversation message still enqueues
    // it through the shared writer's normal provider policy.
    f.store
        .execute(
            &f.keys[0],
            &Operation::Send {
                request_id: "ordinary-provider-message".into(),
                conversation_id: f.cid.clone(),
                message: connect_protocol::messages::MessageDraft::text_all(
                    "ordinary route control",
                ),
            },
        )
        .unwrap();
    assert_eq!(f.count("provider_jobs"), 1);
}
#[test]
fn native_data_capacity_refusal_leaves_no_partial_message_or_identity() {
    let (mut f, request, _) = data_request(true);
    let proof = admit(&f, &request).unwrap();
    let before = counts(&f);
    f.store.limits.message_storage_bytes = 0;
    assert_eq!(
        f.store
            .accept_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap_err()
            .code,
        "capacity-exceeded"
    );
    assert_eq!(counts(&f), before);
    f.store.limits.message_storage_bytes = 1024 * 1024;
    assert_eq!(
        f.store
            .accept_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap()
            .state,
        OperationState::Accepted
    );
}
#[test]
fn native_data_successor_preserves_uncertainty_message_and_original_effect_without_resubmission() {
    let (f, request, _) = data_request(true);
    let proof = admit(&f, &request).unwrap();
    f.store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    let (lease, started) = f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap();
    let receipt = lease.queued().unwrap();
    let path = f.store.path.clone();
    let credential = f.keys[0].clone();
    drop(proof);
    drop(f.store);
    let successor = Store::open(&path, Limits::default()).unwrap();
    assert_eq!(successor.recover_native_streams(256).unwrap(), 1);
    let proof = successor
        .admit_peer_authority(&request, &AuthorityWait::new(Duration::from_secs(1)))
        .unwrap();
    let recovered = successor
        .accept_native_stream_with_peer(&credential, &proof)
        .unwrap();
    assert_eq!(recovered.state, OperationState::Unknown);
    assert_eq!(recovered.record_id, started.record_id);
    assert_eq!(recovered.effects[0].id, started.effects[0].id);
    assert!(successor
        .begin_native_stream_with_peer(&credential, &proof)
        .unwrap()
        .is_none());
    assert_eq!(
        successor.complete_native_stream(&receipt).unwrap_err().code,
        "forbidden"
    );
    let c = successor.connection().unwrap();
    let messages: u64 = c
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE id='application-message'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(messages, 1);
}
