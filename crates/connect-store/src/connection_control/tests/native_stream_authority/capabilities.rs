//! Controlled Store receipt custody, not physical publication evidence.
use super::*;
fn fixture() -> (Fixture, PeerAuthorityRequest) {
    let (f, mut request, _) = parent(true, true);
    let PeerCommand::NativeStream(wire) = &mut request.command else {
        panic!()
    };
    wire.command = Command::Advertise;
    wire.operation_id = "native-capabilities".into();
    wire.by_participant = Some(f.pids[0].clone());
    wire.payload = json!({"by_participant":f.pids[0],"capabilities":{"audio_codecs":[{"name":"pcmu","params":{}}],"future":{"keep":true}},"trigger":"endpoint-change","future":{"original":true}});
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
fn native_capability_original_replay_preserves_newer_exact_descriptor_and_media_bindings() {
    let (f, request) = fixture();
    let before = readiness(&f);
    let first = begin(&f, &request).advertisement_received().unwrap();
    let original = f.store.complete_native_stream(&first).unwrap();
    let mut next = request.clone();
    let PeerCommand::NativeStream(wire) = &mut next.command else {
        panic!()
    };
    wire.operation_id = "native-capabilities-newer".into();
    wire.payload["capabilities"] = json!({"audio_codecs":[],"future":{"latest":"exact"}});
    let expected = wire.payload["capabilities"].clone();
    let second = begin(&f, &next).advertisement_received().unwrap();
    f.store.complete_native_stream(&second).unwrap();
    assert_eq!(f.store.complete_native_stream(&first).unwrap(), original);
    let mut after = readiness(&f);
    let latest = after
        .as_object_mut()
        .unwrap()
        .remove("advertised_capabilities")
        .unwrap();
    assert_eq!(latest["capabilities"], expected);
    assert_eq!(after, before);
    let proof = admit(&f, &request).unwrap();
    assert_eq!(
        f.store
            .accept_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap(),
        original
    );
    assert!(f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .is_none());
    assert_eq!(f.count("operation_effects"), 3);
    let PeerCommand::NativeStream(wire) = &mut next.command else {
        panic!()
    };
    wire.payload["future"] = json!({"original":false});
    let proof = admit(&f, &next).unwrap();
    assert_eq!(
        f.store
            .accept_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap_err()
            .code,
        "conflict"
    );
}
#[test]
fn native_capability_recording_is_atomic_and_delayed_receipt_cannot_revive_retired_media() {
    let (f, request) = fixture();
    let receipt = begin(&f, &request).advertisement_received().unwrap();
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER controlled_capability_failure BEFORE INSERT ON journal WHEN json_extract(NEW.data_json,'$.operation_id')='native-capabilities' AND json_extract(NEW.data_json,'$.state')='completed' BEGIN SELECT RAISE(ABORT,'controlled capability recording failure'); END;").unwrap();
    assert!(f.store.complete_native_stream(&receipt).is_err());
    assert!(readiness(&f).get("advertised_capabilities").is_none());
    let c = f.store.connection().unwrap();
    c.execute(
        "UPDATE native_connection_lifetimes SET retired=1 WHERE connection_id='connection-native'",
        [],
    )
    .unwrap();
    c.execute("UPDATE connections SET state='retired',generation=generation+1,readiness_json=NULL WHERE id='connection-native'",[]).unwrap();
    c.execute_batch("DROP TRIGGER controlled_capability_failure")
        .unwrap();
    drop(c);
    assert_eq!(
        f.store.complete_native_stream(&receipt).unwrap().state,
        connect_protocol::transitions::OperationState::Completed
    );
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
