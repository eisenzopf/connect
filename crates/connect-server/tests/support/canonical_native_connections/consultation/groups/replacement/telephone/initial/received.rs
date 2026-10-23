//! Actual initial SIP input reaches the sealed managed source queue, without
//! pretending that this prerequisite already installs durable forwarding.
use super::*;
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
async fn subscribe(f: &Fixture, source: &str) -> rvoip_core::dtmf::DtmfSourceSubscription {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        match f
            .server
            .service
            .initial_telephone_dtmf_source_for_test(source)
        {
            Ok(subscription) => return subscription,
            Err(error)
                if error.code == "authority-busy" && tokio::time::Instant::now() < deadline =>
            {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            Err(error) => panic!("managed source registration failed: {error:?}"),
        }
    }
}
pub(super) async fn exercise(f: &Fixture, peer: &PeerSip, cid: &str, algorithm: &str) {
    use rvoip_core::dtmf::*;
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
    let mut subscription = subscribe(f, &source).await;
    let mut records = Vec::new();
    for kind in [DtmfSourceKind::SipInfo, DtmfSourceKind::SipRfc4733] {
        if kind == DtmfSourceKind::SipInfo {
            send_info(peer).await;
        } else {
            send_rfc(peer).await;
        }
        let observation = tokio::time::timeout(Duration::from_secs(3), subscription.recv())
            .await
            .unwrap_or_else(|error| panic!("actual {kind:?} source callback timed out: {error:?}; dropped={}; peer_stopped={}", subscription.dropped(), peer.task.is_finished()))
            .expect("managed source subscription ended before input");
        assert_eq!(observation.source().connection().as_str(), source);
        assert_eq!(observation.source().session().as_str(), "native-session");
        assert_eq!(observation.source().conversation().as_str(), cid);
        assert_eq!(observation.input().kind(), kind);
        assert_eq!(observation.input().digits(), "5");
        assert_eq!(observation.input().duration_ms(), 237);
        assert!(observation.disclosure().is_some());
        assert!(!observation.input().observation_id().is_empty());
        assert_eq!(
            format!("{observation:?}"),
            "ReceivedDtmfObservation(redacted)"
        );
        let rtp=observation.input().rtp().map(|event| {
            assert_eq!(event.ssrc,0x87654321);
            assert_eq!(event.payload_type,101);
            assert_eq!(event.clock_rate_hz,8000);
            assert_eq!(event.duration_samples,1896);
            assert!(event.start_observed);
            json!({"ssrc":event.ssrc,"payload_type":event.payload_type,"clock_rate_hz":event.clock_rate_hz,"duration_samples":event.duration_samples,"start_observed":event.start_observed})
        });
        assert_eq!(rtp.is_some(), kind == DtmfSourceKind::SipRfc4733);
        records.push(json!({"kind":kind.as_str(),"observation_id":observation.input().observation_id(),"received_at":observation.input().received_at(),"duration_ms":237,"sealed_audience":true,"rtp":rtp}));
    }
    assert_ne!(records[0]["observation_id"], records[1]["observation_id"]);
    let successor = subscribe(f, &source).await;
    assert!(
        tokio::time::timeout(Duration::from_secs(1), subscription.recv())
            .await
            .unwrap()
            .is_none()
    );
    drop(successor);
    assert_eq!(
        db.query_row("SELECT count(*) FROM native_dtmf_sources", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        0,
        "SIP input must not fabricate signed native custody"
    );
    assert_eq!(peer.unique(), 1);
    if let Some(path) = std::env::var_os("CONNECT_RECEIVED_DTMF_CUSTODY_OBSERVATIONS") {
        use std::io::Write;
        let path = std::path::PathBuf::from(path);
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(".local/gate-6");
        assert!(path.is_absolute() && path.starts_with(root));
        let mut bytes=serde_json::to_vec(&json!({"algorithm":algorithm,"source":"actual-initial-sip","observations":records,"calls":peer.unique(),"scope":"Sealed source queue only; durable Store forwarding remains unimplemented."})).unwrap();
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
async fn send_info(peer: &PeerSip) {
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
}
async fn send_rfc(peer: &PeerSip) {
    let target = peer.media_target.lock().unwrap().unwrap();
    for (index, end) in [false, true, true, true].into_iter().enumerate() {
        let mut packet = vec![0x80, if end { 101 } else { 0x80 | 101 }];
        packet.extend_from_slice(&(30000 + index as u16).to_be_bytes());
        packet.extend_from_slice(&18000u32.to_be_bytes());
        packet.extend_from_slice(&0x87654321u32.to_be_bytes());
        packet.extend_from_slice(&[5, if end { 0x8a } else { 10 }]);
        packet.extend_from_slice(&1896u16.to_be_bytes());
        peer.media.send_to(&packet, target).await.unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
