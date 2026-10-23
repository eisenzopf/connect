//! Actual native ingress and INFO/RFC4733 on the committed moved SIP call.
use super::*;
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_native_to_moved_sip_preserves_actual_generation_call_and_original_schedule() {
    for algorithm in ["EdDSA", "ES256"] {
        telephone_case_profile(Stop::Dtmf, true, algorithm).await;
    }
}
async fn completed(
    client: &mut CanonicalClient,
    cid: &str,
    operation: &str,
) -> rvoip_uctp::v2::payloads::OperationUpdated {
    let end = tokio::time::Instant::now() + Duration::from_secs(8);
    loop {
        let status = client
            .operation_get(
                operation,
                CommandScope {
                    conversation_id: Some(cid.into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        if status.state == rvoip_uctp::v2::payloads::OperationState::Completed {
            return status;
        }
        assert!(
            matches!(
                status.state,
                rvoip_uctp::v2::payloads::OperationState::Preparing
            ),
            "moved SIP signal failed: {}",
            serde_json::to_string(&status).unwrap()
        );
        assert!(
            tokio::time::Instant::now() < end,
            "moved SIP signal did not settle: {}",
            serde_json::to_string(&status).unwrap()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
fn observe(value: serde_json::Value) {
    if let Some(path) = std::env::var_os("CONNECT_MOVED_DTMF_OBSERVATIONS") {
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
async fn route_permissions(f: &Fixture, route: &str, operations: serde_json::Value) {
    let store = f.server.store.clone();
    let route = route.to_owned();
    let path = f.config.database.clone();
    tokio::task::spawn_blocking(move || {
        let identity = store.revalidate(&route).unwrap().identity_id;
        let _writer = store
            .acquire_authority(
                &[connect_store::AuthorityScope::identity(
                    &identity,
                    connect_store::AuthorityMode::Write,
                )
                .unwrap()],
                &connect_store::AuthorityWait::new(Duration::from_secs(2)),
            )
            .unwrap();
        assert_eq!(
            rusqlite::Connection::open(path)
                .unwrap()
                .execute(
                    "UPDATE credentials SET operations=?2 WHERE id=?1",
                    rusqlite::params![route, operations.to_string()]
                )
                .unwrap(),
            1
        );
    })
    .await
    .unwrap();
}
pub(super) async fn exercise(
    f: &Fixture,
    cid: &str,
    peers: &mut [Peer],
    phone: &PeerSip,
    payload: u8,
    moved: &OperationRecord,
    route: &str,
) {
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let logical = moved.result.as_ref().unwrap()["connection_id"]
        .as_str()
        .unwrap();
    let call = phone.calls.lock().unwrap()[0].clone();
    let saved: String = db
        .query_row(
            "SELECT operations FROM credentials WHERE id=?1",
            [route],
            |r| r.get(0),
        )
        .unwrap();
    route_permissions(f, route, json!(["sip_dial", "stream.subscribe"])).await;
    let absent_scope = CommandScope {
        conversation_id: Some(cid.into()),
        session_id: Some("native-session".into()),
        connection_id: Some("codec-source".into()),
        operation_id: Some("moved-sip-missing-receive".into()),
        ..Default::default()
    };
    let refused = peers[0]
        .client
        .send_native_dtmf(absent_scope.clone(), "58#", 100, "info")
        .await
        .unwrap();
    assert_eq!(
        refused.state,
        rvoip_uctp::v2::payloads::OperationState::Failed
    );
    assert_eq!(db.query_row("SELECT count(*) FROM native_dtmf_established_gateways g JOIN operations o ON o.record_id=g.operation_record_id WHERE o.operation_id='moved-sip-missing-receive'",[],|r|r.get::<_,u64>(0)).unwrap(),0);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(phone.info.lock().unwrap().is_empty());
    assert_eq!(
        peers[0]
            .client
            .send_native_dtmf(absent_scope, "58#", 100, "info")
            .await
            .unwrap()
            .state,
        refused.state
    );
    observe(
        json!({"algorithm":f.client.signing_algorithm,"method":"missing-receive","refused":refused,"sip_calls":phone.unique(),"info_packets":phone.info.lock().unwrap().len()}),
    );
    route_permissions(f, route, serde_json::from_str(&saved).unwrap()).await;
    sip_native_duplex(phone, peers, payload, "moved-sip-after-named-refusal").await;
    for method in ["info", "rfc4733"] {
        let mut ends = std::collections::BTreeMap::<u32, (u8, usize)>::new();
        let operation = format!("native-to-moved-sip-{method}");
        let scope = CommandScope {
            conversation_id: Some(cid.into()),
            session_id: Some("native-session".into()),
            connection_id: Some("codec-source".into()),
            operation_id: Some(operation.clone()),
            ..Default::default()
        };
        let accepted = peers[0]
            .client
            .send_native_dtmf(scope.clone(), "58#", 100, method)
            .await
            .unwrap();
        assert!(
            matches!(
                accepted.state,
                rvoip_uctp::v2::payloads::OperationState::Preparing
                    | rvoip_uctp::v2::payloads::OperationState::Completed
            ),
            "{}",
            serde_json::to_string(&accepted).unwrap()
        );
        if method == "rfc4733" {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
            let mut bytes = [0; 4096];
            while ends.len() < 3 || ends.values().any(|(_, n)| *n < 3) {
                let (n, from) =
                    tokio::time::timeout_at(deadline, phone.media.recv_from(&mut bytes))
                        .await
                        .unwrap()
                        .unwrap();
                assert_eq!(from.ip(), phone.address.ip());
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
                    let end = ends
                        .entry(packet.header.timestamp)
                        .or_insert((packet.payload[0], 0));
                    assert_eq!(end.0, packet.payload[0]);
                    end.1 += 1;
                }
            }
            assert_eq!(
                ends.values()
                    .map(|(event, _)| *event)
                    .collect::<std::collections::BTreeSet<_>>(),
                std::collections::BTreeSet::from([5, 8, 11])
            );
        }
        let done = completed(&mut peers[0].client, cid, &operation).await;
        let result = done.result.as_ref().unwrap();
        assert_eq!(result["signal_source"], "uctp-native");
        assert_eq!(result["local_submission_only"], true);
        assert_eq!(result["recipients"].as_array().unwrap().len(), 1);
        assert_eq!(result["recipients"][0]["connection_id"], logical);
        assert_eq!(result["recipients"][0]["generation"], 2);
        assert_eq!(result["recipients"][0]["disposition"], "local_submitted");
        let (kind,target):(String,String)=db.query_row("SELECT d.target_kind,d.target_json FROM native_dtmf_deliveries d JOIN operations o ON o.record_id=d.operation_record_id WHERE o.operation_id=?1",[&operation],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(kind, "sip-established");
        let target: serde_json::Value = serde_json::from_str(&target).unwrap();
        assert_eq!(target["sip_call_id"], call);
        assert_eq!(target["route_credential_id"], route);
        assert!(target.get("endpoint_credential_id").is_none());
        assert!(target.get("key_id").is_none());
        observe(
            json!({"algorithm":f.client.signing_algorithm,"method":method,"completed":done,"target":target,"sip_calls":phone.unique(),"info_packets":phone.info.lock().unwrap().len(),"rtp_events":ends}),
        );
        let before = phone.info.lock().unwrap().len();
        if method == "info" {
            let mut requests = std::collections::BTreeMap::new();
            let mut digits = Vec::new();
            for raw in phone.info.lock().unwrap().iter() {
                let Message::Request(request) = parse_message(raw).unwrap() else {
                    panic!()
                };
                assert_eq!(request.raw_header_value(&HeaderName::CallId).unwrap(), call);
                let body = std::str::from_utf8(&request.body).unwrap();
                if let Some(old) = requests.insert(
                    request.raw_header_value(&HeaderName::CSeq).unwrap(),
                    body.to_owned(),
                ) {
                    assert_eq!(old, body);
                    continue;
                }
                assert!(body.lines().any(|line| line == "Duration=100"));
                digits.push(
                    body.lines()
                        .find_map(|line| line.strip_prefix("Signal="))
                        .unwrap()
                        .to_owned(),
                );
            }
            assert_eq!(digits, vec!["5", "8", "#"]);
        }
        let replay = peers[0]
            .client
            .send_native_dtmf(scope, "58#", 100, method)
            .await
            .unwrap();
        assert_eq!(replay.result, done.result);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(phone.info.lock().unwrap().len(), before);
        assert_eq!(phone.unique(), 1);
        if method == "rfc4733" {
            let deadline = tokio::time::Instant::now() + Duration::from_millis(200);
            let mut bytes = [0; 4096];
            while let Ok(Ok((n, _))) =
                tokio::time::timeout_at(deadline, phone.media.recv_from(&mut bytes)).await
            {
                if n > 1 && (200..=211).contains(&bytes[1]) {
                    continue;
                }
                let packet = rvoip_rtp_core::packet::RtpPacket::parse(&bytes[..n]).unwrap();
                assert_ne!(
                    packet.header.payload_type, 101,
                    "completed replay wrote another event"
                );
            }
        }
        sip_native_duplex(phone, peers, payload, &format!("moved-sip-after-{method}")).await;
    }
}
