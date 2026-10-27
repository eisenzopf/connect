//! Actual signed RTC runtime gates; historical receipts never reopen old gates.
use super::*;
use rvoip_core::staged_streams::StreamController;

async fn changed(
    f: &Fixture,
    ws: &mut Peer,
    cid: &str,
    controller: &Arc<StreamController>,
    operation: &str,
    input: Value,
) -> (Value, connect_protocol::transitions::OperationRecord) {
    let revision = controller.revision();
    let request = scoped(
        cid,
        0,
        "connection.update",
        operation,
        revision,
        input.clone(),
    );
    command(ws, request.clone()).await;
    let result = completed(f, &f.client, operation).await;
    assert_eq!(result.effects.len(), 1);
    assert_eq!(result.effects[0].attempt, 1);
    let body = result.result.as_ref().unwrap();
    assert_eq!(body["stream_revision"], revision + 1);
    assert_eq!(body["resources_changed"], false);
    assert_eq!(body["connection_generation"], 1);
    let streams: Vec<connect_protocol::native_gates::GateStream> =
        serde_json::from_value(body["gate_streams"].clone()).unwrap();
    connect_protocol::native_gates::validate_observation(&input, &streams).unwrap();
    assert_eq!(streams.len(), 1);
    assert_eq!(streams[0].binding.stream_id, "rtc-audio-0");
    assert_eq!(streams[0].binding.stream_generation, 1);
    assert_eq!(controller.revision(), revision + 1);
    (request, result)
}

pub(super) async fn run(
    f: &Fixture,
    host: &SignalingHost,
    first: &mut Peer,
    second: &mut Peer,
    other: &CanonicalClientConfig,
    cid: &str,
    controller: &Arc<StreamController>,
    other_controller: &Arc<StreamController>,
    audio_a: &mut mpsc::Receiver<rvoip_core::stream::MediaFrame>,
    audio_b: &mut mpsc::Receiver<rvoip_core::stream::MediaFrame>,
) {
    let original = controller.active_bindings();
    let budget = controller.stream_open_budget();
    refused(
        second,
        scoped(
            cid,
            1,
            "connection.update",
            "rtc-gate-no-scope-b",
            2,
            json!({"action":"mute","streams":["rtc-audio-1"]}),
        ),
        "forbidden",
        403,
    )
    .await;
    assert_eq!(other_controller.revision(), 2);
    // The original owned RTC route cannot be borrowed by another physical WSS
    // connection even when the signer/credential are identical.
    let mute = scoped(
        cid,
        0,
        "connection.update",
        "rtc-gate-mute-a",
        4,
        json!({"action":"mute","streams":["rtc-audio-0"]}),
    );
    let mut foreign = Peer::open(f).await;
    refused(&mut foreign, mute.clone(), "forbidden", 403).await;
    foreign.socket.close(None).await.unwrap();
    assert_eq!(controller.revision(), 4);
    for (operation, revision, input, code, status) in [
        (
            "rtc-gate-stale-a",
            3,
            json!({"action":"mute","streams":["rtc-audio-0"]}),
            "conflict",
            409,
        ),
        (
            "rtc-gate-missing-a",
            4,
            json!({"action":"mute","streams":["rtc-missing"]}),
            "not-found",
            404,
        ),
        (
            "rtc-gate-quality-a",
            4,
            json!({"action":"quality","quality_interval_ms":700}),
            "unsupported-media-profile",
            501,
        ),
    ] {
        refused(
            first,
            scoped(cid, 0, "connection.update", operation, revision, input),
            code,
            status,
        )
        .await;
    }
    assert_eq!(controller.revision(), 4);
    audible(audio_a, -1).await;
    audible(audio_b, 1).await;
    let (mute, muted) = changed(
        f,
        first,
        cid,
        controller,
        "rtc-gate-mute-a",
        json!({"action":"mute","streams":["rtc-audio-0"]}),
    )
    .await;
    inaudible(audio_b).await;
    audible(audio_a, -1).await;
    let mut replay = mute.clone();
    replay["id"] = json!(uuid::Uuid::new_v4().to_string());
    command(first, replay).await;
    assert_eq!(record(f, &f.client, "rtc-gate-mute-a"), muted);
    assert_eq!(controller.revision(), 5);
    inaudible(audio_b).await;
    changed(
        f,
        first,
        cid,
        controller,
        "rtc-gate-unmute-a",
        json!({"action":"unmute","streams":["rtc-audio-0"]}),
    )
    .await;
    audible(audio_a, -1).await;
    audible(audio_b, 1).await;
    // Endpoint recvonly keeps playback and suppresses its publication. The
    // reverse endpoint sendonly keeps publication and suppresses playback.
    for (operation, direction, hears_a, hears_b) in [
        ("rtc-gate-recvonly-a", "recvonly", true, false),
        ("rtc-gate-sendonly-a", "sendonly", false, true),
        ("rtc-gate-inactive-a", "inactive", false, false),
    ] {
        changed(
            f,
            first,
            cid,
            controller,
            operation,
            json!({"action":"hold","details":{"direction":direction}}),
        )
        .await;
        if hears_a {
            audible(audio_a, -1).await;
        } else {
            inaudible(audio_a).await;
        }
        if hears_b {
            audible(audio_b, 1).await;
        } else {
            inaudible(audio_b).await;
        }
        changed(
            f,
            first,
            cid,
            controller,
            &format!("{operation}-resume"),
            json!({"action":"resume"}),
        )
        .await;
        audible(audio_a, -1).await;
        audible(audio_b, 1).await;
    }
    // Superseded completed replay is the same historical receipt and effect.
    // Current graph and Store revision stay at the restored duplex state.
    let revision = controller.revision();
    let mut replay = mute;
    replay["id"] = json!(uuid::Uuid::new_v4().to_string());
    command(first, replay).await;
    assert_eq!(record(f, &f.client, "rtc-gate-mute-a"), muted);
    assert_eq!(controller.revision(), revision);
    audible(audio_a, -1).await;
    audible(audio_b, 1).await;
    let current = controller.active_bindings();
    assert_eq!(current.len(), original.len());
    for (a, b) in original.iter().zip(current.iter()) {
        assert_eq!(a.transport_id, b.transport_id);
        assert_eq!(a.stream_local_id, b.stream_local_id);
        assert_eq!(a.generation, b.generation);
        assert_eq!(a.connection_generation, b.connection_generation);
        assert_eq!(a.negotiated, b.negotiated);
    }
    assert_eq!(controller.stream_open_budget(), budget);
    assert_eq!(other_controller.revision(), 2);
    assert_eq!(host.adapter().total_allocations(), 2);
    assert_eq!(host.adapter().route_count(), 2);
    assert_eq!(
        record(f, other, "rtc-select-b").state,
        OperationState::Completed
    );
}
