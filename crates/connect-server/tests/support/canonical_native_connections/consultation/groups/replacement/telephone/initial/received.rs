//! Actual installed initial SIP input, durable operation and signed native output.
use super::*;
#[path = "received/authority.rs"]
pub(super) mod authority;
#[path = "received/fanout.rs"]
pub(super) mod fanout;
#[path = "received/outcomes.rs"]
pub(super) mod outcomes;
#[path = "received/privacy.rs"]
pub(super) mod privacy;
#[path = "received/restart.rs"]
pub(super) mod restart;
#[path = "received/retained.rs"]
pub(super) mod retained;
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_initial_sip_received_info_and_rfc4733_keep_actual_provenance_and_sealed_audience(
) {
    for algorithm in ["EdDSA", "ES256"] {
        initial_selected_signal_case(
            false,
            algorithm,
            EndpointTransport::Telephone,
            "participant",
            false,
            Some("received-custody"),
        )
        .await;
    }
}
pub(super) async fn exercise(
    f: &Fixture,
    peer: &PeerSip,
    native: &mut Peer,
    cid: &str,
    algorithm: &str,
) {
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let source: String = db
        .query_row(
            "SELECT runtime_connection_id FROM initial_telephone_operations",
            [],
            |r| r.get(0),
        )
        .unwrap();
    // A fresh same-owner HTTP credential reads status; SIP source custody is
    // never invented from HTTP or borrowed from the native recipient.
    let key = f
        .server
        .store
        .issue(
            &f.peer.identity_id,
            connect_store::Scope::operator(),
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
    let mut records = Vec::new();
    for kind in ["sip-info", "rfc4733"] {
        let info = if kind == "sip-info" {
            Some(send_info(peer).await)
        } else {
            send_rfc(peer).await;
            None
        };
        let frame=tokio::time::timeout(Duration::from_secs(5),async {
            for _ in 0..128 {let frame=native.client.receive().await.unwrap();if frame.kind()=="dtmf.received" {return frame;}}
            panic!("received SIP signal missing behind lifecycle hints")
        }).await.unwrap_or_else(|e|panic!("actual {kind} native delivery timed out: {e:?}; sources={:?}",db.query_row("SELECT group_concat(o.state||':'||COALESCE(o.problem_json,'')) FROM operations o JOIN received_dtmf_sources s ON s.operation_record_id=o.record_id",[],|r|r.get::<_,Option<String>>(0)).unwrap()));
        assert_eq!(
            frame.scope(rvoip_uctp::v2::Scope::Connection),
            Some(native.id.as_str())
        );
        let operation = frame
            .scope(rvoip_uctp::v2::Scope::Operation)
            .unwrap()
            .to_owned();
        assert!(operation.starts_with("dtmfr_"));
        assert_eq!(frame.original()["payload"]["source"], kind);
        assert_eq!(frame.original()["payload"]["digits"], "5");
        assert_eq!(frame.original()["payload"]["duration_ms"], 237);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        let status = loop {
            let r = http.operation_get(&operation, scope.clone()).await.unwrap();
            if matches!(r.state, rvoip_uctp::v2::payloads::OperationState::Completed) {
                break r;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "source did not settle: {r:?}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        let result = status.result.as_ref().unwrap();
        assert_eq!(result["signal_source"], kind);
        assert_eq!(
            result["received_at"],
            frame.original()["payload"]["received_at"]
        );
        assert_eq!(result["source_observation"]["kind"], kind);
        assert_eq!(result["source_observation"]["duration_ms"], 237);
        assert_eq!(result["recipients"].as_array().unwrap().len(), 1);
        assert_eq!(result["recipients"][0]["frame_id"], frame.id());
        assert_eq!(result["recipients"][0]["disposition"], "local_submitted");
        if kind == "rfc4733" {
            let r = &result["source_observation"]["rtp"];
            assert_eq!(r["ssrc"], 0x87654321u32);
            assert_eq!(r["payload_type"], 101);
            assert_eq!(r["clock_rate_hz"], 8000);
            assert_eq!(r["duration_samples"], 1896);
            assert_eq!(r["start_observed"], true);
        } else {
            assert!(result["source_observation"]["rtp"].is_null());
        }
        for key in [
            "stream_revision",
            "signing_key_id",
            "signing_algorithm",
            "envelope_id",
        ] {
            assert!(result.get(key).is_none() && result["source_observation"].get(key).is_none());
        }
        assert_eq!(
            serde_json::to_value(http.operation_get(&operation, scope.clone()).await.unwrap())
                .unwrap(),
            serde_json::to_value(&status).unwrap()
        );
        if let Some((packet, address)) = info {
            peer.socket
                .send_to(packet.as_bytes(), address)
                .await
                .unwrap();
        } else {
            send_rfc(peer).await;
        }
        records.push(json!({"algorithm":algorithm,"source_runtime_connection":source,"operation":status,"native_frame":frame.original()}));
    }
    let deadline = tokio::time::Instant::now() + Duration::from_millis(250);
    while let Ok(Ok(frame)) = tokio::time::timeout_at(deadline, native.client.receive()).await {
        assert_ne!(
            frame.kind(),
            "dtmf.received",
            "retransmitted physical input produced another native signal"
        );
    }
    assert_eq!(
        db.query_row("SELECT count(*) FROM native_dtmf_sources", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM received_dtmf_sources", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        2
    );
    assert_eq!(peer.unique(), 1);
    if let Some(path) = std::env::var_os("CONNECT_RECEIVED_DTMF_DELIVERY_OBSERVATIONS") {
        use std::io::Write;
        let path = std::path::PathBuf::from(path);
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(".local/gate-6");
        assert!(path.is_absolute() && path.starts_with(root));
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        for r in records {
            let mut bytes = serde_json::to_vec(&r).unwrap();
            bytes.push(b'\n');
            file.write_all(&bytes).unwrap();
        }
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_initial_sip_received_incomplete_and_fractional_rtp_never_synthesize_native_tones_and_fresh_input_recovers(
) {
    for algorithm in ["EdDSA", "ES256"] {
        initial_selected_signal_case(
            false,
            algorithm,
            EndpointTransport::Telephone,
            "participant",
            false,
            Some("received-rejected-timing"),
        )
        .await;
    }
}
pub(super) async fn rejected_timing(
    f: &Fixture,
    peer: &PeerSip,
    native: &mut Peer,
    cid: &str,
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
            &f.peer.identity_id,
            connect_store::Scope::operator(),
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
    // Prove this exact native signal writer and source permission with actual
    // IO before testing refusal. An offer/ready row alone is not that proof.
    send_info(peer).await;
    let warmup = tokio::time::timeout(Duration::from_secs(5), async {
        for _ in 0..128 {
            let frame = native.client.receive().await.unwrap();
            if frame.kind() == "dtmf.received" {
                return frame;
            }
        }
        panic!("missing exact native signal writer readiness")
    })
    .await
    .unwrap();
    assert_eq!(warmup.original()["payload"]["source"], "sip-info");
    until(|| {
        db.query_row(
            "SELECT count(*) FROM received_dtmf_sources WHERE closed=1",
            [],
            |r| r.get::<_, u64>(0),
        )
        .unwrap()
            == 1
    })
    .await;
    assert_eq!(
        http.operation_get(
            warmup.scope(rvoip_uctp::v2::Scope::Operation).unwrap(),
            scope.clone()
        )
        .await
        .unwrap()
        .state,
        rvoip_uctp::v2::payloads::OperationState::Completed
    );
    // Keep the same read future alive across absence checks. Cancelling an
    // owned SDK control read can retire its physical reader; a timeout around
    // a pinned reference must not destroy that reader before recovery input.
    let mut next_signal = Box::pin(async {
        for _ in 0..128 {
            let frame = native.client.receive().await.unwrap();
            if frame.kind() == "dtmf.received" {
                return frame;
            }
        }
        panic!("fresh signal missing behind lifecycle hints")
    });
    let mut records = Vec::new();
    for (index, (start, samples)) in [(true, 1897u16), (false, 1896u16)].into_iter().enumerate() {
        send_rfc_timing(peer, start, samples, 0x87654322 + index as u32).await;
        let count = index as u64 + 2;
        until(|| {
            db.query_row(
                "SELECT count(*) FROM received_dtmf_sources WHERE closed=1",
                [],
                |r| r.get::<_, u64>(0),
            )
            .unwrap()
                == count
        })
        .await;
        let operation:String=db.query_row("SELECT o.operation_id FROM operations o JOIN received_dtmf_sources s ON s.operation_record_id=o.record_id WHERE json_extract(s.observation_json,'$.rtp.ssrc')=?1",[0x87654322+index as u32],|r|r.get(0)).unwrap();
        let status = http.operation_get(&operation, scope.clone()).await.unwrap();
        assert_eq!(
            status.state,
            rvoip_uctp::v2::payloads::OperationState::Failed
        );
        let summary = &status.problem.as_ref().unwrap()["signal"];
        assert_eq!(
            summary["recipients"].as_array().map(Vec::len),
            Some(1),
            "received source status: {}",
            serde_json::to_string(&status).unwrap()
        );
        let facts = &summary["source_observation"];
        assert_eq!(facts["rtp"]["duration_samples"], samples);
        assert_eq!(facts["rtp"]["start_observed"], start);
        assert_eq!(facts["duration_ms"], if start { 238 } else { 237 });
        assert_eq!(summary["recipients"][0]["disposition"], "not_started");
        assert!(summary["recipients"][0]["frame_id"].is_null());
        assert_eq!(
            serde_json::to_value(http.operation_get(&operation, scope.clone()).await.unwrap())
                .unwrap(),
            serde_json::to_value(&status).unwrap()
        );
        let deadline = tokio::time::Instant::now() + Duration::from_millis(150);
        assert!(
            tokio::time::timeout_at(deadline, next_signal.as_mut())
                .await
                .is_err(),
            "unforwardable observation produced a native tone"
        );
        records.push(json!({"algorithm":algorithm,"profile":if start{"fractional"}else{"missing-start"},"operation":status}));
    }
    send_rfc(peer).await;
    let frame = tokio::time::timeout(Duration::from_secs(5), next_signal.as_mut())
        .await
        .unwrap();
    drop(next_signal);
    assert_eq!(frame.original()["payload"]["source"], "rfc4733");
    assert_eq!(frame.original()["payload"]["duration_ms"], 237);
    let operation = frame.scope(rvoip_uctp::v2::Scope::Operation).unwrap();
    until(|| {
        db.query_row(
            "SELECT count(*) FROM received_dtmf_sources WHERE closed=1",
            [],
            |r| r.get::<_, u64>(0),
        )
        .unwrap()
            == 4
    })
    .await;
    assert_eq!(
        http.operation_get(operation, scope).await.unwrap().state,
        rvoip_uctp::v2::payloads::OperationState::Completed
    );
    assert_eq!(peer.unique(), 1);
    if let Some(path) = std::env::var_os("CONNECT_RECEIVED_DTMF_FAILURE_OBSERVATIONS") {
        use std::io::Write;
        let path = std::path::PathBuf::from(path);
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(".local/gate-6");
        assert!(path.is_absolute() && path.starts_with(root));
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        for r in records {
            let mut bytes = serde_json::to_vec(&r).unwrap();
            bytes.push(b'\n');
            file.write_all(&bytes).unwrap();
        }
    }
}
async fn send_info(peer: &PeerSip) -> (String, std::net::SocketAddr) {
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
async fn send_rfc(peer: &PeerSip) {
    send_rfc_timing(peer, true, 1896, 0x87654321).await;
}
async fn send_rfc_timing(peer: &PeerSip, start: bool, samples: u16, ssrc: u32) {
    let target = peer.media_target.lock().unwrap().unwrap();
    for (index, end) in [!start, true, true, true].into_iter().enumerate() {
        let mut packet = vec![0x80, if end { 101 } else { 0x80 | 101 }];
        packet.extend_from_slice(&(30000 + index as u16).to_be_bytes());
        packet.extend_from_slice(&18000u32.to_be_bytes());
        packet.extend_from_slice(&ssrc.to_be_bytes());
        packet.extend_from_slice(&[5, if end { 0x8a } else { 10 }]);
        packet.extend_from_slice(&samples.to_be_bytes());
        peer.media.send_to(&packet, target).await.unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
