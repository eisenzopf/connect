//! Controlled Store custody; actual native configuration is a server test.
use super::*;
use connect_protocol::transitions::OperationState;
fn fixture() -> (Fixture, PeerAuthorityRequest) {
    let (f, mut request, _) = parent(true, true);
    let PeerCommand::NativeStream(wire) = &mut request.command else {
        panic!()
    };
    wire.command = Command::Update;
    wire.operation_id = "native-quality".into();
    wire.payload = json!({"action":"quality","quality_interval_ms":400,"future":{"exact":true}});
    (f, request)
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
fn readiness(f: &Fixture) -> serde_json::Value {
    let raw: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT readiness_json FROM connections WHERE id='connection-native'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    serde_json::from_str(&raw).unwrap()
}
#[test]
fn native_quality_original_receipt_replay_cannot_reset_a_newer_cadence_or_change_bindings() {
    let (f, request) = fixture();
    let before = readiness(&f);
    let first = begin(&f, &request).quality_configured(400).unwrap();
    let done = f.store.complete_native_stream(&first).unwrap();
    assert_eq!(done.state, OperationState::Completed);
    assert_eq!(done.command_type, "connection.update");
    assert_eq!(done.result.as_ref().unwrap()["quality_interval_ms"], 400);
    assert_eq!(done.result.as_ref().unwrap()["stream_revision"], 1);
    let mut after = readiness(&f);
    assert_eq!(
        after.as_object_mut().unwrap().remove("quality_interval_ms"),
        Some(json!(400))
    );
    assert_eq!(after, before);
    let mut newer = request.clone();
    let PeerCommand::NativeStream(wire) = &mut newer.command else {
        panic!()
    };
    wire.operation_id = "native-quality-newer".into();
    wire.payload["quality_interval_ms"] = json!(1200);
    let next = begin(&f, &newer).quality_configured(1200).unwrap();
    f.store.complete_native_stream(&next).unwrap();
    assert_eq!(f.store.complete_native_stream(&first).unwrap(), done);
    assert_eq!(readiness(&f)["quality_interval_ms"], 1200);
    let proof = admit(&f, &request).unwrap();
    assert_eq!(
        f.store
            .accept_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap(),
        done
    );
    assert!(f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .is_none());
    assert_eq!(f.count("operation_effects"), 3);
    assert_eq!(readiness(&f)["stream_revision"], 1);
}
#[test]
fn native_quality_interval_is_part_of_immutable_intent_and_the_captured_receipt() {
    let (f, request) = fixture();
    let lease = begin(&f, &request);
    assert!(lease.quality_configured(401).is_err());
    let mut changed = request.clone();
    let PeerCommand::NativeStream(wire) = &mut changed.command else {
        panic!()
    };
    wire.payload["quality_interval_ms"] = json!(401);
    let proof = admit(&f, &changed).unwrap();
    assert_eq!(
        f.store
            .accept_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(f.count("operation_effects"), 2);
    assert!(readiness(&f).get("quality_interval_ms").is_none());
}
#[test]
fn native_quality_captured_completion_recovers_storage_and_never_revives_retired_media() {
    let (f, request) = fixture();
    let receipt = begin(&f, &request).quality_configured(400).unwrap();
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER controlled_quality_record_failure BEFORE INSERT ON journal WHEN json_extract(NEW.data_json,'$.operation_id')='native-quality' AND json_extract(NEW.data_json,'$.state')='completed' BEGIN SELECT RAISE(ABORT,'controlled quality recording failure'); END;").unwrap();
    assert!(f.store.complete_native_stream(&receipt).is_err());
    assert!(readiness(&f).get("quality_interval_ms").is_none());
    let c = f.store.connection().unwrap();
    // Trusted retirement fixture facts, not a claim of actual media teardown.
    c.execute(
        "UPDATE native_connection_lifetimes SET retired=1 WHERE connection_id='connection-native'",
        [],
    )
    .unwrap();
    c.execute("UPDATE connections SET state='retired',generation=generation+1,readiness_json=NULL WHERE id='connection-native'", []).unwrap();
    c.execute_batch("DROP TRIGGER controlled_quality_record_failure")
        .unwrap();
    drop(c);
    let done = f.store.complete_native_stream(&receipt).unwrap();
    assert_eq!(done.state, OperationState::Completed);
    assert_eq!(done.result.unwrap()["quality_interval_ms"], 400);
    let ready: Option<String> = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT readiness_json FROM connections WHERE id='connection-native'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(ready.is_none());
}
