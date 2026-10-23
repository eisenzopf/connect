//! Controlled Store custody only. Production tests prove actual media gating.
use super::*;
use connect_protocol::{
    native_gates::GateStream,
    native_streams::CodecBinding,
    transitions::{OperationAction, OperationState},
};

fn fixture() -> (Fixture, PeerAuthorityRequest) {
    let (f, mut request, _) = parent(true, true);
    let PeerCommand::NativeStream(wire) = &mut request.command else {
        panic!()
    };
    wire.command = Command::Update;
    wire.operation_id = "native-gate".into();
    wire.payload = json!({"action":"mute","streams":["audio"],"future":{"exact":true}});
    (f, request)
}
fn observation(muted: bool) -> Vec<GateStream> {
    vec![GateStream {
        binding: CodecBinding {
            stream_id: "audio".into(),
            medium: "audio".into(),
            direction: "sendrecv".into(),
            codec: json!({"name":"opus","params":{"sample_rate":48000,"channels":1}}),
            stream_local_id: 9,
            stream_generation: 1,
            connection_generation: 1,
        },
        state: "active".into(),
        source_muted: muted,
        held_direction: None,
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
                operation_id: "native-gate".into(),
            },
        )
        .unwrap()
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
fn gate_capture_replay_keeps_original_outcome_without_resetting_a_newer_state() {
    let (f, request) = fixture();
    let first = begin(&f, &request)
        .gates_committed(2, observation(true))
        .unwrap();
    let done = f.store.complete_native_stream(&first).unwrap();
    assert_eq!(done.state, OperationState::Completed);
    assert_eq!(done.result.as_ref().unwrap()["action"], "mute");
    assert_eq!(done.result.as_ref().unwrap()["resources_changed"], false);
    let mut next = request.clone();
    let PeerCommand::NativeStream(wire) = &mut next.command else {
        panic!()
    };
    wire.operation_id = "native-unmute".into();
    wire.expected_revision = Some(2);
    wire.payload = json!({"action":"unmute","streams":["audio"]});
    let second = begin(&f, &next)
        .gates_committed(3, observation(false))
        .unwrap();
    f.store.complete_native_stream(&second).unwrap();
    assert_eq!(f.store.complete_native_stream(&first).unwrap(), done);
    assert_eq!(readiness(&f)["stream_revision"], 3);
    assert_eq!(
        readiness(&f)["last_gate"]["gate_streams"][0]["source_muted"],
        false
    );
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
    assert_eq!(done.effects.len(), 1);
    assert_eq!(f.count("operation_effects"), 3);
}
#[test]
fn gate_capture_refuses_wrong_action_target_generation_and_codec_receipt() {
    for case in 0..6 {
        let (f, request) = fixture();
        let lease = begin(&f, &request);
        let mut streams = observation(true);
        match case {
            0 => streams[0].source_muted = false,
            1 => streams[0].binding.stream_id = "foreign".into(),
            2 => streams[0].binding.connection_generation = 2,
            3 => streams[0].state = "ended".into(),
            4 => streams[0].binding.stream_local_id = 0,
            _ => {
                assert!(lease
                    .codecs_committed(2, vec![streams[0].binding.clone()])
                    .is_err());
                continue;
            }
        }
        assert!(lease.gates_committed(2, streams).is_err());
        assert_eq!(status(&f).state, OperationState::Preparing);
        assert_eq!(readiness(&f)["stream_revision"], 1);
    }
}
#[test]
fn gate_captured_outcome_rolls_back_with_journal_then_recovers_original_after_cancel() {
    let (f, request) = fixture();
    let receipt = begin(&f, &request)
        .gates_committed(2, observation(true))
        .unwrap();
    let before = readiness(&f);
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER controlled_gate_record_failure BEFORE INSERT ON journal WHEN json_extract(NEW.data_json,'$.operation_id')='native-gate' AND json_extract(NEW.data_json,'$.state')='completed' BEGIN SELECT RAISE(ABORT,'controlled gate recording failure'); END;").unwrap();
    assert!(f.store.complete_native_stream(&receipt).is_err());
    assert_eq!(readiness(&f), before);
    assert_eq!(status(&f).state, OperationState::Preparing);
    f.store
        .execute_operation_action(
            &f.keys[0],
            OperationAction::Cancel {
                operation_id: "cancel-gate".into(),
                target_operation_id: "native-gate".into(),
                reason: None,
            },
        )
        .unwrap();
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER controlled_gate_record_failure")
        .unwrap();
    let done = f.store.complete_native_stream(&receipt).unwrap();
    assert!(done.cancel_requested);
    assert_eq!(done.state, OperationState::Completed);
    assert_eq!(readiness(&f)["last_gate"], *done.result.as_ref().unwrap());
    assert_eq!(f.store.complete_native_stream(&receipt).unwrap(), done);
}
#[test]
fn gate_historical_completion_cannot_revive_retired_or_successor_runtime() {
    let (f, request) = fixture();
    let receipt = begin(&f, &request)
        .gates_committed(2, observation(true))
        .unwrap();
    f.store.connection().unwrap().execute_batch("UPDATE native_connection_lifetimes SET retired=1 WHERE connection_id='connection-native'; UPDATE connections SET state='retired',generation=generation+1,readiness_json=NULL WHERE id='connection-native'").unwrap();
    let done = f.store.complete_native_stream(&receipt).unwrap();
    assert_eq!(done.state, OperationState::Completed);
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
    drop(c);
    let path = f.store.path().to_owned();
    drop(f.store);
    let next = Store::open(&path, Limits::default()).unwrap();
    assert!(next.complete_native_stream(&receipt).is_err());
}
#[test]
fn gate_replay_and_status_require_current_update_permission() {
    let (f, request) = fixture();
    let receipt = begin(&f, &request)
        .gates_committed(2, observation(true))
        .unwrap();
    f.store.complete_native_stream(&receipt).unwrap();
    let c = f.store.connection().unwrap();
    let raw: String = c
        .query_row(
            "SELECT operations FROM credentials WHERE id=?1",
            [&f.keys[0]],
            |r| r.get(0),
        )
        .unwrap();
    let mut permissions: Vec<String> = serde_json::from_str(&raw).unwrap();
    permissions.retain(|p| p != "connection.update");
    c.execute(
        "UPDATE credentials SET operations=?2 WHERE id=?1",
        params![f.keys[0], serde_json::to_string(&permissions).unwrap()],
    )
    .unwrap();
    drop(c);
    assert_eq!(admit(&f, &request).err().unwrap().code, "forbidden");
    assert_eq!(
        f.store
            .execute_operation_action(
                &f.keys[0],
                OperationAction::Get {
                    operation_id: "native-gate".into()
                }
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
}
