//! Actual signed native ingress and real UDP INFO/RFC4733 on the owned call.
use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_native_to_initial_sip_info_uses_original_call_and_never_replays() {
    for algorithm in ["EdDSA", "ES256"] {
        initial_selected_signal_case(
            false,
            algorithm,
            EndpointTransport::Telephone,
            "participant",
            false,
            Some("info"),
        )
        .await;
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_native_to_initial_sip_rfc4733_uses_actual_negotiated_event_profile() {
    for algorithm in ["EdDSA", "ES256"] {
        initial_selected_signal_case(
            false,
            algorithm,
            EndpointTransport::Telephone,
            "participant",
            false,
            Some("rfc4733"),
        )
        .await;
    }
}
pub(super) async fn exercise(
    f: &Fixture,
    peer: &PeerSip,
    client: &mut CanonicalClient,
    cid: &str,
    algorithm: &str,
    method: &str,
) {
    let refused = method == "missing-receive";
    let wire_method = if refused { "info" } else { method };
    let operation = format!("native-sip-{method}");
    let scope = CommandScope {
        conversation_id: Some(cid.into()),
        session_id: Some("native-session".into()),
        connection_id: Some("initial-native".into()),
        operation_id: Some(operation.clone()),
        expected_revision: None,
    };
    let accepted = client
        .send_native_dtmf(scope.clone(), "58#", 100, wire_method)
        .await
        .unwrap();
    if refused {
        assert_eq!(
            accepted.state,
            rvoip_uctp::v2::payloads::OperationState::Failed
        );
        assert!(accepted.result.is_none());
        let db = rusqlite::Connection::open_with_flags(
            &f.config.database,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        assert_eq!(db.query_row("SELECT count(*) FROM native_dtmf_gateways g JOIN operations o ON o.record_id=g.operation_record_id WHERE o.operation_id=?1",[&operation],|r|r.get::<_,u64>(0)).unwrap(),0);
        assert_eq!(peer.info.lock().unwrap().len(), 0);
        assert_eq!(
            client
                .send_native_dtmf(scope, "58#", 100, wire_method)
                .await
                .unwrap()
                .state,
            accepted.state
        );
        assert_eq!(peer.unique(), 1);
        return;
    }
    assert!(matches!(
        accepted.state,
        rvoip_uctp::v2::payloads::OperationState::Preparing
            | rvoip_uctp::v2::payloads::OperationState::Completed
    ));
    let mut events = std::collections::BTreeMap::<u32, (u8, usize)>::new();
    if method == "rfc4733" {
        let end = tokio::time::Instant::now() + Duration::from_secs(8);
        let mut bytes = [0; 4096];
        while events.len() < 3 || events.values().any(|(_, n)| *n < 3) {
            let (n, from) = tokio::time::timeout_at(end, peer.media.recv_from(&mut bytes))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(from.ip(), peer.address.ip());
            if n > 1 && (200..=211).contains(&bytes[1]) {
                continue;
            }
            let packet = rvoip_rtp_core::packet::RtpPacket::parse(&bytes[..n]).unwrap();
            if packet.header.payload_type != 101 {
                continue;
            }
            assert_eq!(packet.payload.len(), 4);
            if packet.payload[1] & 0x80 != 0 {
                assert_eq!(
                    u16::from_be_bytes([packet.payload[2], packet.payload[3]]),
                    800
                );
                let entry = events
                    .entry(packet.header.timestamp)
                    .or_insert((packet.payload[0], 0));
                assert_eq!(entry.0, packet.payload[0]);
                entry.1 += 1;
            }
        }
        let actual = events
            .values()
            .map(|(event, _)| *event)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(actual, std::collections::BTreeSet::from([5, 8, 11]));
    }
    let end = tokio::time::Instant::now() + Duration::from_secs(8);
    let completed = loop {
        let record = client
            .operation_get(
                &operation,
                CommandScope {
                    conversation_id: Some(cid.into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        if record.state == rvoip_uctp::v2::payloads::OperationState::Completed {
            break record;
        }
        assert!(
            tokio::time::Instant::now() < end,
            "SIP DTMF failed: {record:?}"
        );
        assert!(
            matches!(
                record.state,
                rvoip_uctp::v2::payloads::OperationState::Preparing
            ),
            "SIP DTMF failed: {record:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let result = completed.result.as_ref().unwrap();
    assert_eq!(result["signal_source"], "uctp-native");
    assert_eq!(result["local_submission_only"], true);
    assert_eq!(result["recipients"].as_array().unwrap().len(), 1);
    assert_eq!(result["recipients"][0]["disposition"], "local_submitted");
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(db.query_row("SELECT count(*) FROM native_dtmf_gateways g JOIN operations o ON o.record_id=g.operation_record_id WHERE o.operation_id=?1 AND g.attempted=1 AND g.disposition='local_submitted'",[&operation],|r|r.get::<_,u64>(0)).unwrap(),1);
    let before = peer.info.lock().unwrap().len();
    if method == "info" {
        let packets = peer.info.lock().unwrap().clone();
        let mut digits = Vec::new();
        let mut transactions = std::collections::BTreeMap::new();
        for raw in packets {
            let Message::Request(request) = parse_message(&raw).unwrap() else {
                panic!()
            };
            let sequence = request.raw_header_value(&HeaderName::CSeq).unwrap();
            let body = std::str::from_utf8(&request.body).unwrap();
            if let Some(old) = transactions.insert(sequence, body.to_owned()) {
                assert_eq!(old, body);
                continue;
            }
            let digit = body
                .lines()
                .find_map(|line| line.strip_prefix("Signal="))
                .unwrap();
            assert!(body.lines().any(|line| line == "Duration=100"));
            digits.push(digit.to_owned());
        }
        assert_eq!(digits, vec!["5", "8", "#"]);
    } else {
        assert_eq!(before, 0);
    }
    assert_eq!(
        client
            .send_native_dtmf(scope, "58#", 100, method)
            .await
            .unwrap()
            .result,
        completed.result
    );
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(peer.info.lock().unwrap().len(), before);
    if method == "rfc4733" {
        let end = tokio::time::Instant::now() + Duration::from_millis(200);
        let mut bytes = [0; 4096];
        while let Ok(Ok((n, _))) =
            tokio::time::timeout_at(end, peer.media.recv_from(&mut bytes)).await
        {
            if n > 1 && (200..=211).contains(&bytes[1]) {
                continue;
            }
            let packet = rvoip_rtp_core::packet::RtpPacket::parse(&bytes[..n]).unwrap();
            assert_ne!(
                packet.header.payload_type, 101,
                "completed replay sent another telephone event"
            );
        }
    }
    if let Some(path) = std::env::var_os("CONNECT_SIP_DTMF_OBSERVATIONS") {
        use std::io::Write;
        let path = std::path::PathBuf::from(path);
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(".local/gate-6");
        assert!(path.is_absolute() && path.starts_with(root));
        let mut bytes=serde_json::to_vec(&json!({"algorithm":algorithm,"method":method,"completed":completed,"sip_calls":peer.unique(),"info_packets":before,"rtp_events":events})).unwrap();
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_native_to_initial_sip_requires_independent_named_receive_permission() {
    for algorithm in ["EdDSA", "ES256"] {
        initial_selected_signal_case(
            false,
            algorithm,
            EndpointTransport::Telephone,
            "participant",
            false,
            Some("missing-receive"),
        )
        .await;
    }
}
