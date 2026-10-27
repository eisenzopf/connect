//! Real signed WSS, first-primary RTP, staged Core admission and original
//! durable readiness. Explicit subscriptions/decoded RTC duplex remain separate.
use super::*;
use rvoip_core::adapter::ConnectionAdapter;
use tokio_util::sync::CancellationToken;

fn publish(peer: &Arc<RvoipPeerConnection>) -> (CancellationToken, tokio::task::JoinHandle<u64>) {
    let track = peer.local_audio_track().unwrap();
    let ssrc = peer.local_audio_ssrc().unwrap();
    let stop = CancellationToken::new();
    let cancel = stop.clone();
    let task = tokio::spawn(async move {
        let mut seq = 1u16;
        let mut sent = 0u64;
        let mut tick = tokio::time::interval(Duration::from_millis(20));
        loop {
            tokio::select! {biased;_=cancel.cancelled()=>break,_=tick.tick()=>{}}
            // Reuse the public packet value constructor only; replace every
            // codec-specific field with actual negotiated PCMU primary audio.
            let mut packet =
                rvoip_webrtc::media::opus_rtp_packet_for_ssrc(ssrc, seq, u32::from(seq) * 160);
            packet.header.payload_type = 0;
            packet.payload = vec![0x80; 160].into();
            match tokio::time::timeout(
                Duration::from_millis(500),
                track.write_rtp_with_extensions(packet, &[]),
            )
            .await
            {
                Ok(Ok(())) => sent += 1,
                _ => break,
            }
            seq = seq.wrapping_add(1);
        }
        sent
    });
    (stop, task)
}
fn same_completed_receipt(
    before: &connect_protocol::transitions::OperationRecord,
    after: &connect_protocol::transitions::OperationRecord,
) {
    // Retirement is a new causal journal fact, not a rewrite of the completed
    // effect/result. Every original event remains in its original order.
    assert!(after.causal_event_ids.starts_with(&before.causal_event_ids));
    let mut body = after.clone();
    body.causal_event_ids = before.causal_event_ids.clone();
    assert_eq!(&body, before);
}
async fn scenario(revoke: bool) {
    let f = configured("EdDSA", 8, None).await;
    let host = SignalingHost::new(audio_config(), 1).unwrap();
    f.server
        .service
        .install_canonical_webrtc(host.clone())
        .unwrap();
    let endpoint = provision(&f).await;
    let cid = solo(&f).await;
    let caller = RvoipPeerConnection::new(&audio_config(), PeerRole::Offerer)
        .await
        .unwrap();
    let sdp = caller.create_offer_and_gather().await.unwrap();
    let mut ws = Peer::open(&f).await;
    let original = offer(&cid, &sdp);
    let original_id = ws.send(original.clone(), true).await;
    let answer = ws.read().await;
    assert_eq!(
        answer.kind(),
        "connection.answer",
        "{:?}",
        answer.original()
    );
    assert_eq!(answer.original()["in_reply_to"], original_id);
    caller
        .set_remote_answer(
            answer.original()["payload"]["substrate_setup"]["sdp"]
                .as_str()
                .unwrap(),
        )
        .await
        .unwrap();
    caller.wait_connected(Duration::from_secs(5)).await.unwrap();
    let session = f
        .server
        .service
        .canonical_session_metadata_for_test("wss-webrtc-session")
        .unwrap();
    // DTLS has genuinely connected, but no primary RTP has been supplied.
    assert_eq!(status(&f).state, OperationState::Preparing);
    assert!(session.read().unwrap().connections.is_empty());
    let scope =
        connect_store::AuthorityScope::conversation(&cid, connect_store::AuthorityMode::Write)
            .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(write) = f.server.store.try_acquire_authority(&[scope.clone()]) {
                drop(write);
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("no originating invocation reader survives the actual RTP wait");
    let (stop, send) = publish(&caller);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let record = status(&f);
            assert!(
                !matches!(
                    record.state,
                    OperationState::Failed | OperationState::Cancelled | OperationState::Unknown
                ),
                "{:?}",
                record
            );
            if record.state == OperationState::Completed {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("actual primary RTP and fresh control complete the original offer");
    let record = status(&f);
    assert_eq!(record.effects.len(), 1);
    assert_eq!(
        record.connection_id.as_deref(),
        Some("wss-webrtc-connection")
    );
    assert_eq!(
        record.result.as_ref().unwrap()["streams"],
        json!(["original-audio"])
    );
    let core_id = {
        let state = session.read().unwrap();
        assert_eq!(state.connections.len(), 1);
        assert_eq!(state.participants.len(), 1);
        state.connections.keys().next().unwrap().clone()
    };
    let controller = host.adapter().staged_controller(&core_id).unwrap();
    assert_eq!(controller.revision(), 1);
    let bindings = controller.active_bindings();
    assert_eq!(bindings.len(), 1);
    assert_eq!(
        bindings[0].negotiated.requested.logical_id.as_str(),
        "original-audio"
    );
    assert!(bindings[0].source_ready && bindings[0].sink_ready);
    assert_eq!(host.adapter().total_allocations(), 1);
    assert_eq!(host.adapter().route_count(), 1);
    assert_eq!(host.available_slots(), 0);
    // Fresh historical replay reads the same original result, without another
    // answer, physical allocation, media revision or participant admission.
    let mut replay = original;
    replay["id"] = json!(uuid::Uuid::new_v4().to_string());
    let replay_id = ws.send(replay, true).await;
    let mut observed = false;
    for _ in 0..128 {
        let e = ws.read().await;
        assert_eq!(e.original()["in_reply_to"], replay_id);
        assert_ne!(e.kind(), "connection.answer");
        if e.kind() == "operation.updated" {
            assert_eq!(e.original()["payload"]["state"], "completed");
            observed = true
        }
        if e.kind() == "ack" {
            break;
        }
    }
    assert!(observed);
    same_completed_receipt(&record, &status(&f));
    assert_eq!(controller.revision(), 1);
    assert_eq!(host.adapter().total_allocations(), 1);
    if revoke {
        let store = f.server.store.clone();
        let admin = store.authenticate(&f.admin).unwrap().credential_id;
        tokio::time::timeout(
            Duration::from_secs(3),
            tokio::task::spawn_blocking(move || {
                store.execute_provisioning(
                    &admin,
                    ProvisioningCommand::RevokeEndpoint {
                        operation_id: "revoke-ready-rtc".into(),
                        endpoint_id: endpoint,
                        expected_revision: 1,
                    },
                )
            }),
        )
        .await
        .expect("completed admission retains no idle authority interval")
        .unwrap()
        .unwrap();
    } else {
        caller.close().await.unwrap();
    }
    // A closed remote UDP peer cannot provide a reliable close acknowledgement.
    // The pinned ICE defaults allow 5s disconnected + 25s failed detection;
    // retain actual custody through that protocol boundary, with 5s test slack.
    let retirement_budget = Duration::from_secs(if revoke { 5 } else { 35 });
    tokio::time::timeout(retirement_budget, async {
        loop {
            if host.available_slots() == 1
                && host.adapter().route_count() == 0
                && f.server
                    .service
                    .canonical_runtime_snapshot()
                    .unwrap()
                    .connections
                    == 0
                && session.read().unwrap().connections.is_empty()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("original RTC, port, controller and live-peer Core quota retire before reuse");
    let retired = status(&f);
    same_completed_receipt(&record, &retired);
    assert_eq!(
        retired.causal_event_ids.len(),
        record.causal_event_ids.len() + 1,
        "one new original-retirement journal fact is retained"
    );
    assert!(controller.active_bindings().is_empty());
    // The same WSS physical peer is still usable; the zero quota observation
    // cannot be explained by filtering a closed physical peer from the snapshot.
    ws.create(&f).await;
    assert_eq!(
        f.server
            .service
            .canonical_runtime_snapshot()
            .unwrap()
            .physical_peers,
        1
    );
    stop.cancel();
    assert!(send.await.unwrap() > 0);
    caller.close().await.unwrap();
    ws.socket.close(None).await.unwrap();
    drop(ws);
    f.server.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn original_rtc_primary_media_admits_once_and_joins_core_after_remote_close() {
    scenario(false).await
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn current_withdrawal_after_rtc_media_admission_retires_only_that_route() {
    scenario(true).await
}
