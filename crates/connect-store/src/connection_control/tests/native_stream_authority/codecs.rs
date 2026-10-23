//! Codec custody uses controlled readiness; no physical codec result is claimed.
use super::*;
use connect_protocol::native_streams::CodecBinding;
use connect_protocol::transitions::{OperationAction, OperationState};
fn fixture() -> (Fixture, PeerAuthorityRequest) {
    let (f, mut request, _) = parent(true, true);
    let PeerCommand::NativeStream(wire) = &mut request.command else {
        panic!()
    };
    wire.command = Command::Update;
    wire.operation_id = "codec-native".into();
    wire.payload = json!({"action":"codec-renegotiate","streams":["audio"],"codec_preferences":["opus"],"future":{"keep":"exact"}});
    (f, request)
}
fn observation() -> Vec<CodecBinding> {
    vec![CodecBinding {
        stream_id: "audio".into(),
        medium: "audio".into(),
        direction: "sendrecv".into(),
        codec: json!({"name":"opus","params":{"sample_rate":48000,"channels":1,"payload_type":111}}),
        stream_local_id: 9,
        stream_generation: 2,
        connection_generation: 1,
    }]
}
fn begin(f: &Fixture, request: &PeerAuthorityRequest) -> crate::NativeStreamLease {
    let proof = admit(f, request).unwrap();
    f.store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    f.store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap()
        .0
}
fn status(f: &Fixture) -> connect_protocol::transitions::OperationRecord {
    f.store
        .execute_operation_action(
            &f.keys[0],
            OperationAction::Get {
                operation_id: "codec-native".into(),
            },
        )
        .unwrap()
}
#[test]
fn native_codec_custody_keeps_exact_original_intent_one_effect_and_actual_binding_metadata() {
    let (f, request) = fixture();
    let lease = begin(&f, &request);
    let row = status(&f);
    assert_eq!(row.command_type, "connection.update");
    assert_eq!(row.state, OperationState::Preparing);
    let mut changed = request.clone();
    let PeerCommand::NativeStream(w) = &mut changed.command else {
        panic!()
    };
    w.payload["codec_preferences"] = json!(["pcmu"]);
    let changed_proof = admit(&f, &changed).unwrap();
    assert_eq!(
        f.store
            .accept_native_stream_with_peer(&f.keys[0], &changed_proof)
            .unwrap_err()
            .code,
        "conflict"
    );
    drop(changed_proof);
    let proof = admit(&f, &request).unwrap();
    assert!(f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .is_none());
    drop(proof);
    let receipt = lease.codecs_committed(2, observation()).unwrap();
    let done = f.store.complete_native_stream(&receipt).unwrap();
    assert_eq!(done.state, OperationState::Completed);
    assert_eq!(
        done.result.as_ref().unwrap()["codec_bindings"],
        serde_json::to_value(observation()).unwrap()
    );
    assert_eq!(f.store.complete_native_stream(&receipt).unwrap(), done);
    let c = f.store.connection().unwrap();
    let saved:String=c.query_row("SELECT json_extract(readiness_json,'$.codec_bindings') FROM connections WHERE id='connection-native'",[],|r|r.get(0)).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&saved).unwrap(),
        serde_json::to_value(observation()).unwrap()
    );
    assert_eq!(done.effects.len(), 1);
    assert_eq!(done.effects[0].correlation_key, done.record_id);
}
#[test]
fn native_codec_permission_is_separate_from_creating_the_connection_and_is_current_on_status() {
    let (f, request) = fixture();
    let lease = begin(&f, &request);
    let receipt = lease.codecs_committed(2, observation()).unwrap();
    f.store.complete_native_stream(&receipt).unwrap();
    let c = f.store.connection().unwrap();
    let raw: String = c
        .query_row(
            "SELECT operations FROM credentials WHERE id=?1",
            [&f.keys[0]],
            |r| r.get(0),
        )
        .unwrap();
    let mut operations: Vec<String> = serde_json::from_str(&raw).unwrap();
    operations.retain(|v| v != "connection.update");
    c.execute(
        "UPDATE credentials SET operations=?2 WHERE id=?1",
        params![f.keys[0], serde_json::to_string(&operations).unwrap()],
    )
    .unwrap();
    drop(c);
    assert_eq!(admit(&f, &request).err().unwrap().code, "forbidden");
    assert_eq!(
        f.store
            .execute_operation_action(
                &f.keys[0],
                OperationAction::Get {
                    operation_id: "codec-native".into()
                }
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(f.count("operation_effects"), 2);
}
#[test]
fn native_codec_captured_result_records_after_journal_failure_cancellation_and_actual_retirement_without_revival(
) {
    let (f, request) = fixture();
    let lease = begin(&f, &request);
    let receipt = lease.codecs_committed(2, observation()).unwrap();
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER controlled_codec_record_failure BEFORE INSERT ON journal WHEN json_extract(NEW.data_json,'$.operation_id')='codec-native' AND json_extract(NEW.data_json,'$.state')='completed' BEGIN SELECT RAISE(ABORT,'controlled codec recording failure'); END;").unwrap();
    assert!(f.store.complete_native_stream(&receipt).is_err());
    assert_eq!(status(&f).state, OperationState::Preparing);
    f.store
        .execute_operation_action(
            &f.keys[0],
            OperationAction::Cancel {
                operation_id: "cancel-codec".into(),
                target_operation_id: "codec-native".into(),
                reason: None,
            },
        )
        .unwrap();
    let c = f.store.connection().unwrap();
    // Controlled native retirement attribution, not actual termination evidence.
    c.execute(
        "UPDATE native_connection_lifetimes SET retired=1 WHERE connection_id='connection-native'",
        [],
    )
    .unwrap();
    c.execute("UPDATE connections SET state='retired',generation=generation+1,readiness_json=NULL WHERE id='connection-native'",[]).unwrap();
    c.execute_batch("DROP TRIGGER controlled_codec_record_failure")
        .unwrap();
    drop(c);
    let done = f.store.complete_native_stream(&receipt).unwrap();
    assert_eq!(done.state, OperationState::Completed);
    assert!(done.cancel_requested);
    let c = f.store.connection().unwrap();
    let (state, ready): (String, Option<String>) = c
        .query_row(
            "SELECT state,readiness_json FROM connections WHERE id='connection-native'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, "retired");
    assert!(ready.is_none());
}
#[test]
fn native_codec_restart_retains_unknown_original_effect_and_rejects_old_captured_success() {
    let (f, request) = fixture();
    let lease = begin(&f, &request);
    let receipt = lease.codecs_committed(2, observation()).unwrap();
    let path = f.store.path().to_owned();
    let key = f.keys[0].clone();
    drop(f.store);
    let next = Store::open(&path, Limits::default()).unwrap();
    assert_eq!(next.recover_native_streams(256).unwrap(), 1);
    assert!(next.complete_native_stream(&receipt).is_err());
    let row = next
        .execute_operation_action(
            &key,
            OperationAction::Get {
                operation_id: "codec-native".into(),
            },
        )
        .unwrap();
    assert_eq!(row.state, OperationState::Unknown);
    assert!(row.result.is_none());
    assert_eq!(row.effects.len(), 1);
}
