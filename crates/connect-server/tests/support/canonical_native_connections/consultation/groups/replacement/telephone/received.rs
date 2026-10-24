//! Real moved SIP source, durable custody and signed native delivery.
use super::*;
#[path = "received/privacy.rs"]
pub(super) mod privacy;
#[path = "received/private_primary.rs"]
pub(super) mod private_primary;
#[path = "received/refresh.rs"]
pub(super) mod refresh;
#[path = "received/retained.rs"]
pub(super) mod retained;
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_moved_sip_received_info_and_rfc4733_preserve_actual_source_and_current_authority(
) {
    for algorithm in ["EdDSA", "ES256"] {
        telephone_case_profile(Stop::DtmfReceive, true, algorithm).await;
    }
}
async fn signal(client: &mut CanonicalClient) -> rvoip_uctp::v2::Envelope {
    for _ in 0..128 {
        let frame = client.receive().await.unwrap();
        if frame.kind() == "dtmf.received" {
            return frame;
        }
    }
    panic!("signal missing behind lifecycle hints");
}
fn observe(value: serde_json::Value) {
    if let Some(path) = std::env::var_os("CONNECT_MOVED_RECEIVED_DTMF_OBSERVATIONS") {
        use std::io::Write;
        let path = std::path::PathBuf::from(path);
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(".local/gate-6");
        assert!(path.is_absolute() && path.starts_with(root));
        let mut bytes = serde_json::to_vec(&value).unwrap();
        bytes.push(b'\n');
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap()
            .write_all(&bytes)
            .unwrap();
    }
}
async fn accepted_capabilities(
    f: &Fixture,
    cid: &str,
    participant: &str,
    value: serde_json::Value,
) {
    let store = f.server.store.clone();
    let cid = cid.to_owned();
    let participant = participant.to_owned();
    let path = f.config.database.clone();
    tokio::task::spawn_blocking(move || {
        let _writer=store.acquire_authority(&[connect_store::AuthorityScope::conversation(&cid,connect_store::AuthorityMode::Write).unwrap()],&connect_store::AuthorityWait::new(Duration::from_secs(2))).unwrap();
        assert_eq!(rusqlite::Connection::open(path).unwrap().execute("UPDATE session_presence_capabilities SET capabilities_json=?3 WHERE session_id='native-session' AND participant_id=?2 AND EXISTS(SELECT 1 FROM sessions WHERE id='native-session' AND conversation_id=?1)",rusqlite::params![cid,participant,value.to_string()]).unwrap(),1);
    }).await.unwrap();
}
pub(super) async fn exercise(
    f: &Fixture,
    cid: &str,
    peers: &mut [Peer],
    phone: &PeerSip,
    payload: u8,
    moved: &OperationRecord,
    route: &str,
    algorithm: &str,
) {
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let key = f
        .server
        .store
        .issue(
            &f.client.identity_id,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let mut config = canonical_http_config(f);
    config.credential = key.token;
    let mut http = CanonicalHttpClient::open(config).await.unwrap();
    let scope = CommandScope {
        conversation_id: Some(cid.into()),
        session_id: Some("native-session".into()),
        ..Default::default()
    };
    let logical = moved.result.as_ref().unwrap()["connection_id"]
        .as_str()
        .unwrap();
    let participant = peers[1].participant.clone();
    let original_caps:String=db.query_row("SELECT capabilities_json FROM session_presence_capabilities WHERE session_id='native-session' AND participant_id=?1",[&participant],|r|r.get(0)).unwrap();
    accepted_capabilities(f, cid, &participant, json!({"audio":["g.711-a"]})).await;
    // Drain bounded previously admitted media before testing independent signal.
    let drain_until = tokio::time::Instant::now() + Duration::from_millis(250);
    let sip_drain = async {
        let mut bytes = [0; 4096];
        while let Ok(Ok(_)) =
            tokio::time::timeout_at(drain_until, phone.media.recv_from(&mut bytes)).await
        {}
    };
    let native_drain = async {
        while let Ok(Ok(_)) =
            tokio::time::timeout_at(drain_until, peers[0].client.receive_native_rtp_datagram())
                .await
        {}
    };
    tokio::join!(sip_drain, native_drain);
    tokio::join!(phone.rtp(payload), private_input_isolation(peers));
    let (_, marked) = tokio::join!(
        probe(peers, 0, &[], "moved-signal-only-native-input"),
        sip_audio(phone, payload, Duration::from_millis(2600))
    );
    assert_eq!(marked, 0, "named signal permission granted RTP output");
    // One pending receive spans quiet, refusal, retransmission and recovery.
    // Cancelling a reliable client receive would retire that physical reader.
    let first = {
        let receive = signal(&mut peers[0].client);
        tokio::pin!(receive);
        assert!(
            tokio::time::timeout(Duration::from_millis(200), &mut receive)
                .await
                .is_err(),
            "private confirmation leaked into the committed source"
        );
        let original: String = db
            .query_row(
                "SELECT operations FROM credentials WHERE id=?1",
                [route],
                |r| r.get(0),
            )
            .unwrap();
        dtmf::route_permissions(
            f,
            route,
            json!(["sip_dial", "stream.subscribe", "dtmf.receive"]),
        )
        .await;
        let (denied, address) = send_info(phone).await;
        assert!(
            tokio::time::timeout(Duration::from_millis(250), &mut receive)
                .await
                .is_err(),
            "missing source dtmf.send borrowed another right"
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM received_dtmf_sources WHERE parent_record_id=?1",
                [&moved.record_id],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            0
        );
        dtmf::route_permissions(f, route, serde_json::from_str(&original).unwrap()).await;
        phone
            .socket
            .send_to(denied.as_bytes(), address)
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(200), &mut receive)
                .await
                .is_err(),
            "refused original input revived after restoring permission"
        );
        observe(
            json!({"algorithm":algorithm,"phase":"source-scope-refusal","source_rows":0,"sip_calls":phone.unique(),"media_denied":true}),
        );
        let packet = send_info(phone).await;
        (
            tokio::time::timeout(Duration::from_secs(5), &mut receive)
                .await
                .unwrap(),
            packet,
        )
    };
    for (index, kind) in ["sip-info", "rfc4733", "sip-info"].into_iter().enumerate() {
        let frame = match index {
            0 => first.0.clone(),
            1 => {
                send_rfc(phone).await;
                tokio::time::timeout(Duration::from_secs(5), signal(&mut peers[0].client))
                    .await
                    .unwrap()
            }
            _ => {
                let receive = signal(&mut peers[0].client);
                tokio::pin!(receive);
                phone
                    .socket
                    .send_to(first.1 .0.as_bytes(), first.1 .1)
                    .await
                    .unwrap();
                send_rfc(phone).await;
                assert!(
                    tokio::time::timeout(Duration::from_millis(250), &mut receive)
                        .await
                        .is_err(),
                    "retransmission produced another native signal"
                );
                send_info(phone).await;
                tokio::time::timeout(Duration::from_secs(5), &mut receive)
                    .await
                    .unwrap()
            }
        };
        assert_eq!(
            frame.scope(rvoip_uctp::v2::Scope::Connection),
            Some(peers[0].id.as_str())
        );
        assert_eq!(frame.scope(rvoip_uctp::v2::Scope::Conversation), Some(cid));
        assert_eq!(frame.original()["payload"]["source"], kind);
        assert_eq!(frame.original()["payload"]["digits"], "5");
        assert_eq!(frame.original()["payload"]["duration_ms"], 237);
        let operation = frame
            .scope(rvoip_uctp::v2::Scope::Operation)
            .unwrap()
            .to_owned();
        assert!(operation.starts_with("dtmfr_"));
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        let status = loop {
            let r = http.operation_get(&operation, scope.clone()).await.unwrap();
            if r.state == rvoip_uctp::v2::payloads::OperationState::Completed {
                break r;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "{}",
                serde_json::to_string(&r).unwrap()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        let result = status.result.as_ref().unwrap();
        assert_eq!(result["connection_id"], logical);
        assert_eq!(result["signal_source"], kind);
        assert_eq!(
            result["received_at"],
            frame.original()["payload"]["received_at"]
        );
        assert_eq!(result["recipients"].as_array().unwrap().len(), 1);
        assert_eq!(result["recipients"][0]["frame_id"], frame.id());
        assert_eq!(result["source_observation"]["duration_ms"], 237);
        if kind == "rfc4733" {
            let r = &result["source_observation"]["rtp"];
            assert_eq!(r["payload_type"], 101);
            assert_eq!(r["clock_rate_hz"], 8000);
            assert_eq!(r["duration_samples"], 1896);
            assert_eq!(r["ssrc"], 0x87654321u32);
            assert_eq!(r["start_observed"], true);
        } else {
            assert!(result["source_observation"]["rtp"].is_null());
        }
        for field in [
            "stream_revision",
            "signing_key_id",
            "signing_algorithm",
            "envelope_id",
        ] {
            assert!(result.get(field).is_none());
            assert!(result["source_observation"].get(field).is_none());
        }
        let (owner,parent,kind,source):(String,String,String,String)=db.query_row("SELECT o.owner_id,s.parent_record_id,s.parent_kind,s.source_json FROM operations o JOIN received_dtmf_sources s ON s.operation_record_id=o.record_id WHERE o.operation_id=?1",[&operation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(owner, f.client.identity_id);
        assert_eq!(parent, moved.record_id);
        assert_eq!(kind, "sip-established");
        let source: serde_json::Value = serde_json::from_str(&source).unwrap();
        assert_eq!(source["generation"], 2);
        assert_eq!(source["connection_id"], logical);
        assert_eq!(source["sip_call_id"], phone.calls.lock().unwrap()[0]);
        assert_ne!(source["route_owner_id"], source["identity_id"]);
        assert!(source.get("endpoint_credential_id").is_none());
        assert!(source.get("key_id").is_none());
        assert_eq!(
            serde_json::to_value(http.operation_get(&operation, scope.clone()).await.unwrap())
                .unwrap(),
            serde_json::to_value(&status).unwrap()
        );
        observe(
            json!({"algorithm":algorithm,"phase":"delivered","source":source,"source_parent":parent,"operation":status,"native_frame":frame.original(),"sip_calls":phone.unique(),"media_denied":true}),
        );
    }
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM received_dtmf_sources WHERE parent_record_id=?1",
            [&moved.record_id],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        3
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM native_dtmf_sources", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
    assert_eq!(phone.unique(), 1);
    accepted_capabilities(
        f,
        cid,
        &participant,
        serde_json::from_str(&original_caps).unwrap(),
    )
    .await;
    sip_native_duplex(
        phone,
        peers,
        payload,
        "moved-received-after-original-replay",
    )
    .await;
}
pub(super) async fn send_info(peer: &PeerSip) -> (String, std::net::SocketAddr) {
    let dialog = peer.dialog.lock().unwrap().clone().unwrap();
    let index = peer
        .info_sequence
        .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    let body = "Signal=5\r\nDuration=237\r\n";
    let request=format!("INFO sip:connect@{} SIP/2.0\r\nVia: SIP/2.0/UDP {};branch=z9hG4bK-received-{index};rport\r\nFrom: {}\r\nTo: {}\r\nCall-ID: {}\r\nCSeq: {index} INFO\r\nMax-Forwards: 70\r\nContent-Type: application/dtmf-relay\r\nContent-Length: {}\r\n\r\n{body}",dialog.remote,peer.address,dialog.to,dialog.from,dialog.call_id,body.len());
    peer.socket
        .send_to(request.as_bytes(), dialog.remote)
        .await
        .unwrap();
    (request, dialog.remote)
}
pub(super) async fn send_rfc(peer: &PeerSip) {
    send_rfc_timing(peer, true, 1896, 0x87654321).await;
}
pub(super) async fn send_next_rfc(peer: &PeerSip) {
    send_rfc_event(peer, true, 1896, 0x87654321, 36000, 30004).await;
}
async fn send_rfc_timing(peer: &PeerSip, start: bool, samples: u16, ssrc: u32) {
    send_rfc_event(peer, start, samples, ssrc, 18000, 30000).await;
}
async fn send_rfc_event(
    peer: &PeerSip,
    start: bool,
    samples: u16,
    ssrc: u32,
    timestamp: u32,
    sequence: u16,
) {
    let target = peer.media_target.lock().unwrap().unwrap();
    for (index, end) in [!start, true, true, true].into_iter().enumerate() {
        let mut packet = vec![0x80, if end { 101 } else { 0x80 | 101 }];
        packet.extend_from_slice(&(sequence + index as u16).to_be_bytes());
        packet.extend_from_slice(&timestamp.to_be_bytes());
        packet.extend_from_slice(&ssrc.to_be_bytes());
        packet.extend_from_slice(&[5, if end { 0x8a } else { 10 }]);
        packet.extend_from_slice(&samples.to_be_bytes());
        peer.media.send_to(&packet, target).await.unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
