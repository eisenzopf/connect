//! Actual signed WSS, unchanged RTC/Core ports, new ICE credentials and fresh
//! independently decoded duplex markers. No distinct-network/relay/video claim.
use super::*;

fn fresh(mut request: Value) -> Value {
    let header = envelope("connection.update", json!({}));
    request["id"] = header["id"].clone();
    request["ts"] = header["ts"].clone();
    request
}
async fn answer(ws: &mut Peer, request: Value) -> String {
    // Wire replay protection is independent of durable logical-operation
    // recovery: new signed envelope, same operation/scope/offer bytes.
    let request = fresh(request);
    let id = ws.send(request.clone(), true).await;
    for _ in 0..128 {
        let frame = ws.read().await;
        assert_eq!(frame.original()["in_reply_to"], id);
        assert_ne!(frame.kind(), "error", "{:?}", frame.original());
        assert_ne!(frame.kind(), "connection.answer");
        if frame.kind() == "operation.updated" {
            assert!(!frame.original()["payload"]
                .to_string()
                .contains("a=ice-pwd:"));
        }
        if frame.kind() == "ack" {
            for scope in ["cid", "sid", "connid", "operation_id"] {
                assert_eq!(frame.original()[scope], request[scope]);
            }
            let proposed: connect_protocol::webrtc_ice_restart::IceRestartAnswer =
                serde_json::from_value(
                    frame.original()["payload"]["details"]["ice_restart"].clone(),
                )
                .unwrap();
            proposed
                .validate(
                    request["payload"]["details"]["expected_ice_generation"]
                        .as_u64()
                        .unwrap(),
                )
                .unwrap();
            assert!(frame.original()["payload"]["details"]
                .get("ice_generation")
                .is_none());
            return proposed.setup.sdp;
        }
    }
    panic!("original ICE answer missing");
}
fn request(cid: &str, revision: u64, generation: u64, op: &str, sdp: &str) -> Value {
    scoped(
        cid,
        0,
        "connection.update",
        op,
        revision,
        json!({"action":"ice-restart","details":{"expected_ice_generation":generation,"setup":{"kind":"websocket+webrtc","sdp":sdp}}}),
    )
}
fn same_ports(
    before: &[rvoip_core::stream_lifecycle::ReadyMediaBinding],
    after: &[rvoip_core::stream_lifecycle::ReadyMediaBinding],
) {
    assert_eq!(before.len(), after.len());
    for (a, b) in before.iter().zip(after) {
        assert_eq!(a.negotiated, b.negotiated);
        assert_eq!(a.transport_id, b.transport_id);
        assert_eq!(a.stream_local_id, b.stream_local_id);
        assert_eq!(a.generation, b.generation);
        assert_eq!(a.connection_generation, b.connection_generation);
    }
}
pub(super) async fn run(
    f: &Fixture,
    host: &SignalingHost,
    ws: &mut Peer,
    cid: &str,
    endpoint: &str,
    peers: &[Arc<RvoipPeerConnection>],
    controllers: &[Arc<rvoip_core::staged_streams::StreamController>; 2],
    markers: &[Arc<AtomicU8>],
    audio_a: &mut mpsc::Receiver<rvoip_core::stream::MediaFrame>,
    audio_b: &mut mpsc::Receiver<rvoip_core::stream::MediaFrame>,
) {
    let revision = controllers[0].revision();
    let bindings = controllers[0].active_bindings();
    let other = controllers[1].active_bindings();
    let budget = controllers[0].stream_open_budget();
    let mut historical = None;
    for generation in 0..2 {
        let before = peers[0]
            .peer_connection()
            .local_description()
            .await
            .unwrap()
            .sdp;
        let offer = peers[0].restart_ice_and_gather().await.unwrap();
        if let Err(error) =
            connect_protocol::webrtc_ice_restart::validate_offer_change(&before, &offer)
        {
            let dir = tempfile::Builder::new()
                .prefix("connect-ice-shape-")
                .tempdir()
                .unwrap()
                .keep();
            std::fs::write(dir.join("before.sdp"), &before).unwrap();
            std::fs::write(dir.join("proposed.sdp"), &offer).unwrap();
            panic!("Actual RTC restart shape refused: {error:?}; private synthetic SDP retained at {dir:?}");
        }
        let op = format!("rtc-ice-restart-{generation}");
        let input = request(cid, revision, generation, &op, &offer);
        let first_answer = answer(ws, input.clone()).await;
        let pending = record(f, &f.client, &op);
        assert_eq!(pending.state, OperationState::Preparing);
        assert!(pending.result.is_none());
        assert_eq!(pending.effects.len(), 1);
        // Discard the first answer locally. Same logical operation recovers
        // the same private bytes without another physical invocation/offer.
        let recovered = answer(ws, input.clone()).await;
        assert_eq!(first_answer, recovered);
        assert_eq!(host.adapter().total_allocations(), 2);
        peers[0].set_remote_answer(&recovered).await.unwrap();
        let done = completed(f, &f.client, &op).await;
        if generation == 0 {
            historical = Some((input.clone(), recovered.clone(), done.clone()));
        }
        assert_eq!(done.effects.len(), 1);
        assert_eq!(done.effects[0].attempt, 1);
        let result = done.result.as_ref().unwrap();
        assert_eq!(result["action"], "ice-restart");
        assert_eq!(result["ice_generation"], generation + 1);
        assert_eq!(result["stream_revision"], revision);
        assert_eq!(result["connection_generation"], 1);
        assert_eq!(result["resources_changed"], false);
        assert!(!result.to_string().contains("a=ice-pwd:"));
        assert_eq!(controllers[0].revision(), revision);
        assert_eq!(controllers[0].stream_open_budget(), budget);
        same_ports(&bindings, &controllers[0].active_bindings());
        same_ports(&other, &controllers[1].active_bindings());
        // Flip both live source markers AFTER captured ICE completion. A
        // pre-restart queued packet cannot satisfy these fresh decoded signs.
        let a = if generation == 0 { 0x00 } else { 0x80 };
        let b = if generation == 0 { 0x80 } else { 0x00 };
        markers[0].store(a, Ordering::Relaxed);
        markers[1].store(b, Ordering::Relaxed);
        while audio_a.try_recv().is_ok() {}
        while audio_b.try_recv().is_ok() {}
        audible(audio_a, if b == 0x80 { 1 } else { -1 }).await;
        audible(audio_b, if a == 0x80 { 1 } else { -1 }).await;
        assert_eq!(answer(ws, input).await, recovered);
        assert_eq!(record(f, &f.client, &op), done);
    }
    let (old_input, old_answer, old_record) = historical.unwrap();
    assert_eq!(answer(ws, old_input).await, old_answer);
    assert_eq!(record(f, &f.client, "rtc-ice-restart-0"), old_record);
    assert_eq!(
        record(f, &f.client, "rtc-ice-restart-1")
            .result
            .as_ref()
            .unwrap()["ice_generation"],
        2
    );
    same_ports(&bindings, &controllers[0].active_bindings());
    assert_eq!(controllers[0].revision(), revision);
    let offer = peers[0].restart_ice_and_gather().await.unwrap();
    let input = request(cid, revision, 2, "rtc-ice-revoked-in-flight", &offer);
    let fingerprint = offer
        .lines()
        .find(|line| line.starts_with("a=fingerprint:"))
        .unwrap();
    let changed = offer.replace(fingerprint, &format!("{fingerprint}0"));
    assert_ne!(changed, offer);
    refused(
        ws,
        request(cid, revision, 2, "rtc-ice-changed-identity", &changed),
        "invalid-request",
        400,
    )
    .await;
    let mut foreign = Peer::open(f).await;
    refused(&mut foreign, fresh(input.clone()), "forbidden", 403).await;
    foreign.socket.close(None).await.unwrap();
    let mut stale = input.clone();
    stale["operation_id"] = json!("rtc-ice-stale-counter");
    stale["payload"]["details"]["expected_ice_generation"] = json!(1);
    refused(ws, fresh(stale), "conflict", 409).await;
    let _answer_not_applied = answer(ws, input.clone()).await;
    assert_eq!(
        record(f, &f.client, "rtc-ice-revoked-in-flight").state,
        OperationState::Preparing
    );
    let store = f.server.store.clone();
    let admin = store.authenticate(&f.admin).unwrap().credential_id;
    let endpoint = endpoint.to_owned();
    tokio::time::timeout(
        Duration::from_secs(3),
        tokio::task::spawn_blocking(move || {
            store.execute_provisioning(
                &admin,
                ProvisioningCommand::RevokeEndpoint {
                    operation_id: "rtc-ice-withdraw-original-endpoint".into(),
                    endpoint_id: endpoint,
                    expected_revision: 1,
                },
            )
        }),
    )
    .await
    .expect("idle ICE custody holds no authority reader")
    .unwrap()
    .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let unknown = record(f, &f.client, "rtc-ice-revoked-in-flight");
            if unknown.state == OperationState::Unknown
                && host.adapter().route_count() == 1
                && host.available_slots() == 1
            {
                assert!(unknown.result.is_none());
                assert_eq!(unknown.effects.len(), 1);
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("original physical close/join and Unknown outcome after current withdrawal");
    assert_eq!(host.adapter().total_allocations(), 2);
    same_ports(&other, &controllers[1].active_bindings());
    assert_eq!(controllers[1].revision(), 2);
}
