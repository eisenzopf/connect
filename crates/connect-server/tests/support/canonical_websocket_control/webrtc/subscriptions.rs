//! Real WSS/Store/Core RTC audio, with explicit subscriptions and wire decoding.
use super::*;
#[path = "av.rs"]
mod av;
#[path = "gates.rs"]
mod gates;
#[path = "opus.rs"]
mod opus;
use rvoip_core::participant::ParticipantRole;
use rvoip_core::{adapter::ConnectionAdapter, ids::StreamId, stream::MediaStream};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

fn scoped(cid: &str, index: usize, kind: &str, op: &str, revision: u64, payload: Value) -> Value {
    let mut value = envelope(kind, payload);
    value["cid"] = json!(cid);
    value["sid"] = json!("http3-media-session");
    value["connid"] = json!(format!("rtc-connection-{index}"));
    value["operation_id"] = json!(op);
    value["expected_revision"] = json!(revision);
    value
}
fn record(
    f: &Fixture,
    c: &CanonicalClientConfig,
    op: &str,
) -> connect_protocol::transitions::OperationRecord {
    f.server
        .store
        .execute_operation_action(
            &f.server
                .store
                .authenticate(&c.credential)
                .unwrap()
                .credential_id,
            OperationAction::Get {
                operation_id: op.into(),
            },
        )
        .unwrap()
}
async fn completed(
    f: &Fixture,
    c: &CanonicalClientConfig,
    op: &str,
) -> connect_protocol::transitions::OperationRecord {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let r = record(f, c, op);
            if r.state == OperationState::Completed {
                return r;
            }
            assert!(
                matches!(
                    r.state,
                    OperationState::Accepted | OperationState::Preparing
                ),
                "{r:?}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
async fn command(ws: &mut Peer, value: Value) {
    let id = ws.send(value, true).await;
    for _ in 0..128 {
        let e = ws.read().await;
        assert_eq!(e.original()["in_reply_to"], id);
        assert_ne!(e.kind(), "error", "{:?}", e.original());
        assert_ne!(
            e.kind(),
            "connection.offer",
            "routing cannot borrow native negotiation"
        );
        if e.kind() == "ack" {
            return;
        }
    }
    panic!("command did not acknowledge")
}
async fn refused(ws: &mut Peer, value: Value, code: &str, public_code: u16) {
    let id = ws.send(value, true).await;
    let mut observed = false;
    for _ in 0..128 {
        let e = ws.read().await;
        assert_eq!(e.original()["in_reply_to"], id);
        match e.kind() {
            "error" => {
                assert_eq!(e.original()["payload"]["code"], public_code);
                return;
            }
            "operation.updated" => {
                assert_eq!(e.original()["payload"]["state"], "failed");
                assert_eq!(e.original()["payload"]["problem"]["code"], code);
                observed = true;
            }
            "ack" => {
                assert!(observed);
                return;
            }
            _ => panic!("unexpected refusal frame: {:?}", e.original()),
        }
    }
    panic!("refusal did not finish")
}
async fn session_role_snapshot(ws: &mut Peer, cid: &str, participant: &str) -> (u64, String) {
    use connect_client::events::SnapshotItem;
    let mut request = envelope("conversation.get", json!({"limit":100}));
    request["cid"] = json!(cid);
    let id = ws.send(request, true).await;
    let mut topology = None;
    let mut role = None;
    for _ in 0..128 {
        let e = ws.read().await;
        assert_eq!(e.original()["in_reply_to"], id);
        match e.payload() {
            rvoip_uctp::v2::Payload::ConversationSnapshot(snapshot) => {
                assert!(
                    !snapshot.has_more,
                    "bounded fixture must fit one snapshot page"
                );
                for item in &snapshot.items {
                    match serde_json::from_value::<SnapshotItem>(item.clone()).unwrap() {
                        SnapshotItem::Session(s) if s.id == "http3-media-session" => {
                            topology = Some(s.topology_revision)
                        }
                        SnapshotItem::SessionParticipant(p)
                            if p.session_id == "http3-media-session"
                                && p.participant_id == participant =>
                        {
                            assert_eq!(p.state, "present");
                            role = Some(p.role);
                        }
                        _ => (),
                    }
                }
            }
            rvoip_uctp::v2::Payload::Ack(_) => return (topology.unwrap(), role.unwrap()),
            _ => panic!("unexpected signed snapshot frame: {:?}", e.original()),
        }
    }
    panic!("snapshot did not finish")
}
fn role_command(cid: &str, participant: &str, role: &str, op: &str, topology: u64) -> Value {
    let mut request = envelope(
        "session.update",
        json!({"kind":"role-change","details":{"participant_id":participant,"role":role}}),
    );
    request["cid"] = json!(cid);
    request["sid"] = json!("http3-media-session");
    request["operation_id"] = json!(op);
    request["expected_revision"] = json!(topology);
    request
}
async fn projected_role(f: &Fixture, participant: &str, role: &ParticipantRole) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if f.server
                .service
                .canonical_session_role_for_test("http3-media-session", participant)
                .unwrap()
                .as_ref()
                == Some(role)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("actual Core participant role follows the current durable role");
}
fn publish(
    peer: &Arc<RvoipPeerConnection>,
    marker: u8,
) -> (CancellationToken, tokio::task::JoinHandle<u64>) {
    let track = peer.local_audio_track().unwrap();
    let ssrc = peer.local_audio_ssrc().unwrap();
    let stop = CancellationToken::new();
    let cancel = stop.clone();
    let task = tokio::spawn(async move {
        let mut seq = 1u16;
        let mut sent = 0;
        let mut tick = tokio::time::interval(Duration::from_millis(20));
        loop {
            tokio::select! { biased; _=cancel.cancelled()=>break, _=tick.tick()=>{} }
            let mut packet =
                rvoip_webrtc::media::opus_rtp_packet_for_ssrc(ssrc, seq, u32::from(seq) * 160);
            packet.header.payload_type = 0;
            packet.payload = vec![marker; 160].into();
            if !matches!(
                tokio::time::timeout(
                    Duration::from_millis(500),
                    track.write_rtp_with_extensions(packet, &[])
                )
                .await,
                Ok(Ok(()))
            ) {
                break;
            }
            sent += 1;
            seq = seq.wrapping_add(1);
        }
        sent
    });
    (stop, task)
}
async fn receiver(
    peer: &Arc<RvoipPeerConnection>,
    index: usize,
) -> (
    Arc<rvoip_webrtc::media::WebRtcMediaStream>,
    mpsc::Receiver<rvoip_core::stream::MediaFrame>,
) {
    let remote = tokio::time::timeout(Duration::from_secs(5), peer.recv_remote_track())
        .await
        .unwrap()
        .unwrap();
    let stream = rvoip_webrtc::media::from_tracks(
        StreamId::from_string(format!("independent-decoder-{index}")),
        CodecInfo::from_name_with_defaults("g.711-mu"),
        peer.local_audio_track().unwrap(),
        peer.local_audio_ssrc().unwrap(),
        0,
        Some(remote),
    );
    let rx = stream.try_frames_in().unwrap();
    (stream, rx)
}
// Independent G.711 mu-law decode, after the receiver's real SRTP/RTP path.
fn decode(byte: u8) -> i16 {
    let u = !byte;
    let magnitude = (((i32::from(u & 15) << 3) + 132) << ((u >> 4) & 7)) - 132;
    if u & 128 != 0 {
        -magnitude as i16
    } else {
        magnitude as i16
    }
}
async fn audible(rx: &mut mpsc::Receiver<rvoip_core::stream::MediaFrame>, sign: i16) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let frame = rx.recv().await.expect("original RTC track remains live");
            assert_eq!(frame.payload_type, Some(0));
            if frame.payload.len() == 160
                && frame.payload.iter().all(|b| {
                    let sample = decode(*b);
                    sample.signum() == sign && sample.unsigned_abs() > 20_000
                })
            {
                return;
            }
        }
    })
    .await
    .expect("independently decoded remote marker");
}
async fn inaudible(rx: &mut mpsc::Receiver<rvoip_core::stream::MediaFrame>) {
    tokio::time::sleep(Duration::from_millis(300)).await;
    while rx.try_recv().is_ok() {}
    let until = tokio::time::Instant::now() + Duration::from_millis(400);
    loop {
        tokio::select! {
            _=tokio::time::sleep_until(until)=>break,
            frame=rx.recv()=>{
                let frame = frame.expect("RTC receiver stays live during routing withdrawal");
                assert_eq!(frame.payload.len(), 160);
                assert!(frame.payload.iter().all(|b| decode(*b).unsigned_abs() < 500), "unselected source remained audible");
            }
        }
    }
}
async fn scenario(algorithm: &str, end: bool, roles: bool, gates: bool) {
    if roles || gates {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "warn".into()),
            )
            .try_init();
    }
    let f = configured_end_scope(algorithm, 8, None, end).await;
    let host = SignalingHost::new(audio_config(), 2).unwrap();
    f.server
        .service
        .install_canonical_webrtc(host.clone())
        .unwrap();
    let other = if gates {
        canonical_webtransport_media::other_caps_with_permissions(
            &f,
            vec!["audio".into()],
            false,
            &["connection.update"],
        )
    } else {
        canonical_webtransport_media::other(&f)
    };
    provision_config(&f, &f.client, 0).await;
    let endpoint = provision_config(&f, &other, 1).await;
    let (cid, initiator, recipient) = canonical_webtransport_media::accepted(&f, &other).await;
    let mut first = Peer::open(&f).await;
    let mut second = Peer::open_config(&f, &other).await;
    let mut peers = Vec::new();
    let mut senders = Vec::new();
    for (i, ws, config) in [(0, &mut first, &f.client), (1, &mut second, &other)] {
        let peer = RvoipPeerConnection::new(&audio_config(), PeerRole::Offerer)
            .await
            .unwrap();
        let sdp = peer.create_offer_and_gather().await.unwrap();
        let mut value = offer(&cid, &sdp);
        value["sid"] = json!("http3-media-session");
        value["connid"] = json!(format!("rtc-connection-{i}"));
        value["operation_id"] = json!(format!("rtc-offer-{i}"));
        value["expected_revision"] = json!(0);
        value["payload"]["streams_offered"][0]["id"] = json!(format!("rtc-audio-{i}"));
        let id = ws.send(value, true).await;
        let answer = ws.read().await;
        assert_eq!(
            answer.kind(),
            "connection.answer",
            "{:?}",
            answer.original()
        );
        assert_eq!(answer.original()["in_reply_to"], id);
        peer.set_remote_answer(
            answer.original()["payload"]["substrate_setup"]["sdp"]
                .as_str()
                .unwrap(),
        )
        .await
        .unwrap();
        peer.wait_connected(Duration::from_secs(5)).await.unwrap();
        let held_registry = if roles && i == 1 {
            let service = f.server.service.clone();
            let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel();
            let owner = tokio::task::spawn_blocking(move || {
                service.with_canonical_media_registry_for_test(|| {
                    let _ = entered_tx.send(());
                    // Bounded even if an assertion cancels the observer.
                    let _ = release_rx.recv_timeout(Duration::from_secs(3));
                })
            });
            entered_rx.await.unwrap();
            Some((release_tx, owner))
        } else {
            None
        };
        senders.push(publish(&peer, if i == 0 { 0x80 } else { 0x00 }));
        if let Some((release, owner)) = held_registry {
            tokio::time::sleep(Duration::from_millis(200)).await;
            let waiting = record(&f, config, &format!("rtc-offer-{i}"));
            release.send(()).unwrap();
            owner.await.unwrap().unwrap();
            assert_eq!(
                waiting.state,
                OperationState::Preparing,
                "metadata exclusion cannot withdraw the actual original admission"
            );
        }
        completed(&f, config, &format!("rtc-offer-{i}")).await;
        peers.push(peer);
    }
    let session = f
        .server
        .service
        .canonical_session_metadata_for_test("http3-media-session")
        .unwrap();
    let ids = {
        let state = session.read().unwrap();
        assert_eq!(state.connections.len(), 2);
        [initiator.as_str(), recipient.as_str()].map(|p| {
            state
                .connections
                .iter()
                .find(|(_, c)| c.participant_id.as_str() == p)
                .unwrap()
                .0
                .clone()
        })
    };
    let controllers = ids
        .each_ref()
        .map(|id| host.adapter().staged_controller(id).unwrap());
    // With no selected source the guarded mixer emits no RTP. Both completed
    // physical peers and source producers are already live; the same receiver
    // paths must deliver decoded markers after the explicit commands below.
    for peer in &peers {
        assert!(
            tokio::time::timeout(Duration::from_millis(400), peer.recv_remote_track())
                .await
                .is_err(),
            "no implicit receive subscription"
        );
    }
    let selection = scoped(
        &cid,
        0,
        "stream.subscribe",
        "rtc-select-a",
        1,
        json!({"subscriptions":[{"from_participant":recipient,"kinds":["audio"]}]}),
    );
    // A second physical WSS peer with the SAME credential cannot use the
    // original peer's RTC route, nor poison the original operation/effect.
    let mut foreign = Peer::open(&f).await;
    let mut foreign_attempt = selection.clone();
    foreign_attempt["id"] = json!(uuid::Uuid::new_v4().to_string());
    let id = foreign.send(foreign_attempt, true).await;
    let refusal = foreign.read().await;
    assert_eq!(refusal.original()["in_reply_to"], id);
    assert_eq!(refusal.kind(), "error");
    assert_eq!(controllers[0].revision(), 1);
    foreign.socket.close(None).await.unwrap();
    drop(foreign);
    command(&mut first, selection.clone()).await;
    let original = completed(&f, &f.client, "rtc-select-a").await;
    let held_activation = if roles {
        let service = f.server.service.clone();
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let owner = tokio::task::spawn_blocking(move || {
            service.with_canonical_media_activation_for_test("rtc-connection-1", || {
                let _ = entered_tx.send(());
                // Independent release also progresses when packet/control
                // workers are waiting on this deliberately held leaf mutex.
                std::thread::sleep(Duration::from_millis(500));
            })
        });
        entered_rx.await.unwrap();
        Some(owner)
    } else {
        None
    };
    command(
        &mut second,
        scoped(
            &cid,
            1,
            "stream.subscribe",
            "rtc-select-b",
            1,
            json!({"subscriptions":[{"from_participant":initiator}]}),
        ),
    )
    .await;
    if let Some(owner) = held_activation {
        owner.await.unwrap().unwrap();
    }
    completed(&f, &other, "rtc-select-b").await;
    assert_eq!(controllers[0].revision(), 2);
    assert_eq!(controllers[1].revision(), 2);
    let (stream_a, mut audio_a) = receiver(&peers[0], 0).await;
    let (stream_b, mut audio_b) = receiver(&peers[1], 1).await;
    audible(&mut audio_a, -1).await;
    audible(&mut audio_b, 1).await;
    refused(
        &mut first,
        scoped(
            &cid,
            0,
            "stream.subscribe",
            "rtc-stale-a",
            1,
            json!({"subscriptions":[{"strm_id":"rtc-audio-1"}]}),
        ),
        "conflict",
        409,
    )
    .await;
    refused(
        &mut first,
        scoped(
            &cid,
            0,
            "stream.subscribe",
            "rtc-unsupported-a",
            2,
            json!({"subscriptions":[{"from_participant":recipient,"kinds":["video"]}]}),
        ),
        "unsupported-media-profile",
        501,
    )
    .await;
    assert_eq!(controllers[0].revision(), 2);
    assert_eq!(host.adapter().total_allocations(), 2);
    audible(&mut audio_a, -1).await;
    audible(&mut audio_b, 1).await;
    let mut replay = selection;
    replay["id"] = json!(uuid::Uuid::new_v4().to_string());
    command(&mut first, replay).await;
    assert_eq!(record(&f, &f.client, "rtc-select-a"), original);
    assert_eq!(controllers[0].revision(), 2);
    assert_eq!(original.effects.len(), 1);
    assert_eq!(host.adapter().total_allocations(), 2);
    command(
        &mut first,
        scoped(
            &cid,
            0,
            "stream.unsubscribe",
            "rtc-unselect-a",
            2,
            json!({"strm_ids":["rtc-audio-1"]}),
        ),
    )
    .await;
    completed(&f, &f.client, "rtc-unselect-a").await;
    assert_eq!(controllers[0].revision(), 3);
    inaudible(&mut audio_a).await;
    audible(&mut audio_b, 1).await;
    command(
        &mut first,
        scoped(
            &cid,
            0,
            "stream.subscribe",
            "rtc-reselect-a",
            3,
            json!({"subscriptions":[{"strm_id":"rtc-audio-1"}]}),
        ),
    )
    .await;
    completed(&f, &f.client, "rtc-reselect-a").await;
    assert_eq!(controllers[0].revision(), 4);
    audible(&mut audio_a, -1).await;
    audible(&mut audio_b, 1).await;
    if roles {
        let (topology, original_role) = session_role_snapshot(&mut first, &cid, &recipient).await;
        let original_core_role = f
            .server
            .service
            .canonical_session_role_for_test("http3-media-session", &recipient)
            .unwrap()
            .unwrap();
        assert_ne!(original_core_role, ParticipantRole::Observer);
        let initial_projections = f
            .server
            .service
            .canonical_runtime_snapshot()
            .unwrap()
            .role_projections;
        refused(
            &mut second,
            role_command(&cid, &initiator, "observer", "rtc-refused-role", topology),
            "forbidden",
            403,
        )
        .await;
        assert_eq!(
            session_role_snapshot(&mut first, &cid, &recipient).await,
            (topology, original_role.clone())
        );
        audible(&mut audio_a, -1).await;
        audible(&mut audio_b, 1).await;
        let observer = role_command(&cid, &recipient, "observer", "rtc-observe-b", topology);
        command(&mut first, observer.clone()).await;
        let receipt = completed(&f, &f.client, "rtc-observe-b").await;
        let (observer_topology, role) = session_role_snapshot(&mut first, &cid, &recipient).await;
        assert_eq!(role, "observer");
        assert_eq!(observer_topology, topology + 1);
        projected_role(&f, &recipient, &ParticipantRole::Observer).await;
        inaudible(&mut audio_a).await;
        audible(&mut audio_b, 1).await;
        assert_eq!(controllers[0].revision(), 4);
        assert_eq!(controllers[1].revision(), 2);
        assert_eq!(host.adapter().total_allocations(), 2);
        assert_eq!(host.adapter().route_count(), 2);
        let observed_projections = f
            .server
            .service
            .canonical_runtime_snapshot()
            .unwrap()
            .role_projections;
        assert_eq!(observed_projections, initial_projections + 1);
        let mut replay = observer.clone();
        replay["id"] = json!(uuid::Uuid::new_v4().to_string());
        command(&mut first, replay).await;
        assert_eq!(record(&f, &f.client, "rtc-observe-b"), receipt);
        assert_eq!(
            session_role_snapshot(&mut first, &cid, &recipient).await,
            (observer_topology, "observer".into())
        );
        command(
            &mut first,
            role_command(
                &cid,
                &recipient,
                &original_role,
                "rtc-restore-b",
                observer_topology,
            ),
        )
        .await;
        completed(&f, &f.client, "rtc-restore-b").await;
        projected_role(&f, &recipient, &original_core_role).await;
        assert_eq!(
            session_role_snapshot(&mut first, &cid, &recipient).await,
            (observer_topology + 1, original_role.clone())
        );
        audible(&mut audio_a, -1).await;
        audible(&mut audio_b, 1).await;
        let restored_projections = f
            .server
            .service
            .canonical_runtime_snapshot()
            .unwrap()
            .role_projections;
        assert_eq!(restored_projections, initial_projections + 2);
        // Historical replay must not reinstate a superseded observer role.
        let mut replay = observer;
        replay["id"] = json!(uuid::Uuid::new_v4().to_string());
        command(&mut first, replay).await;
        assert_eq!(record(&f, &f.client, "rtc-observe-b"), receipt);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(
            f.server
                .service
                .canonical_session_role_for_test("http3-media-session", &recipient)
                .unwrap(),
            Some(original_core_role)
        );
        assert_eq!(
            session_role_snapshot(&mut first, &cid, &recipient).await,
            (observer_topology + 1, original_role)
        );
        assert_eq!(
            f.server
                .service
                .canonical_runtime_snapshot()
                .unwrap()
                .role_projections,
            restored_projections
        );
        assert_eq!(controllers[0].revision(), 4);
        assert_eq!(controllers[1].revision(), 2);
        assert!(ids.iter().all(|id| host.adapter().is_connection_live(id)));
        audible(&mut audio_a, -1).await;
        audible(&mut audio_b, 1).await;
    }
    if gates {
        gates::run(
            &f,
            &host,
            &mut first,
            &mut second,
            &other,
            &cid,
            &controllers[0],
            &controllers[1],
            &mut audio_a,
            &mut audio_b,
        )
        .await;
    }
    let final_revision = controllers[0].revision();
    if end {
        let request = scoped(
            &cid,
            0,
            "connection.end",
            "rtc-end-a",
            final_revision,
            json!({"reason_code":0,"reason":"designated local fixture complete"}),
        );
        command(&mut first, request.clone()).await;
        let ended = completed(&f, &f.client, "rtc-end-a").await;
        assert_eq!(ended.result.as_ref().unwrap()["ended"], true);
        assert_eq!(ended.effects.len(), 1);
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if host.adapter().route_count() == 1
                    && host.available_slots() == 1
                    && session.read().unwrap().connections.len() == 1
                    && f.server
                        .service
                        .canonical_runtime_snapshot()
                        .unwrap()
                        .connections
                        == 1
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(!host.adapter().is_connection_live(&ids[0]));
        assert!(host.adapter().is_connection_live(&ids[1]));
        assert!(controllers[0].active_bindings().is_empty());
        assert_eq!(controllers[1].revision(), 2);
        assert_eq!(host.adapter().total_allocations(), 2);
        inaudible(&mut audio_b).await;
        let mut replay = request;
        replay["id"] = json!(uuid::Uuid::new_v4().to_string());
        command(&mut first, replay).await;
        assert_eq!(record(&f, &f.client, "rtc-end-a"), ended);
        assert_eq!(host.adapter().total_allocations(), 2);
        // The original WSS peer remains live after explicit media termination.
        first.create(&f).await;
    } else {
        refused(
            &mut first,
            scoped(
                &cid,
                0,
                "connection.end",
                "rtc-denied-end-a",
                final_revision,
                json!({"reason_code":0,"reason":"no end scope"}),
            ),
            "forbidden",
            403,
        )
        .await;
        assert_eq!(controllers[0].revision(), final_revision);
        audible(&mut audio_a, -1).await;
        audible(&mut audio_b, 1).await;
        let store = f.server.store.clone();
        let admin = store.authenticate(&f.admin).unwrap().credential_id;
        tokio::time::timeout(
            Duration::from_secs(3),
            tokio::task::spawn_blocking(move || {
                store.execute_provisioning(
                    &admin,
                    ProvisioningCommand::RevokeEndpoint {
                        operation_id: "rtc-withdraw-source".into(),
                        endpoint_id: endpoint,
                        expected_revision: 1,
                    },
                )
            }),
        )
        .await
        .unwrap()
        .unwrap()
        .unwrap();
        inaudible(&mut audio_a).await;
        tokio::time::timeout(Duration::from_secs(5), async {
            while host.adapter().route_count() != 1 || host.available_slots() != 1 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(controllers[0].revision(), final_revision);
        assert_eq!(host.adapter().total_allocations(), 2);
        assert!(host.adapter().is_connection_live(&ids[0]));
    }
    stream_a.close().await.unwrap();
    stream_b.close().await.unwrap();
    for (stop, sender) in senders {
        stop.cancel();
        assert!(sender.await.unwrap() > 0);
    }
    for peer in peers {
        peer.close().await.unwrap();
    }
    first.socket.close(None).await.unwrap();
    second.socket.close(None).await.unwrap();
    drop(first);
    drop(second);
    f.server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn signed_rtc_explicit_subscriptions_decode_duplex_replay_and_withdrawal_eddsa() {
    scenario("EdDSA", false, false, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn signed_rtc_explicit_subscriptions_decode_duplex_replay_and_withdrawal_es256() {
    scenario("ES256", false, false, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn signed_rtc_end_joins_original_resources_and_replays_eddsa() {
    scenario("EdDSA", true, false, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn signed_rtc_end_joins_original_resources_and_replays_es256() {
    scenario("ES256", true, false, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn signed_rtc_current_roles_project_and_restore_decoded_duplex_eddsa() {
    scenario("EdDSA", false, true, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn signed_rtc_current_roles_project_and_restore_decoded_duplex_es256() {
    scenario("ES256", false, true, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn signed_rtc_runtime_gates_fence_decoded_duplex_and_replay_eddsa() {
    scenario("EdDSA", true, false, true).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn signed_rtc_runtime_gates_fence_decoded_duplex_and_replay_es256() {
    scenario("ES256", true, false, true).await;
}
