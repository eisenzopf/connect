//! Physical first-digit stops and independent signal-only call lifetime.
use super::*;
use rvoip_uctp::v2::payloads::OperationState;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_native_to_initial_sip_cancel_after_actual_info_stops_suffix_and_never_replays()
{
    case("info-cancel").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_native_to_initial_sip_source_revocation_stops_next_digit_with_original_uncertainty(
) {
    case("info-source-revoke").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_native_to_initial_sip_signal_only_authority_keeps_call_and_dtmf_without_rtp() {
    case("signal-only-info").await;
    case("signal-only-rfc4733").await;
}

async fn case(method: &str) {
    for algorithm in ["EdDSA", "ES256"] {
        initial_selected_signal_case(
            false,
            algorithm,
            EndpointTransport::Telephone,
            "participant",
            false,
            Some(method),
        )
        .await;
    }
}

pub(super) async fn signal_only(
    f: &Fixture,
    peer: &PeerSip,
    native: &mut [Peer],
    credential: &str,
) {
    let store = f.server.store.clone();
    let identity = f.peer.identity_id.clone();
    let key = credential.to_owned();
    let path = f.config.database.clone();
    // A controlled credential withdrawal takes the actual Store writer fence;
    // no already admitted media reader may survive this commit. This is not a
    // claim of public credential-edit API qualification.
    tokio::task::spawn_blocking(move || {
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
        let db = rusqlite::Connection::open(path).unwrap();
        assert_eq!(
            db.execute(
                "UPDATE credentials SET operations='[\"dtmf.receive\"]' WHERE id=?1",
                [key]
            )
            .unwrap(),
            1
        );
    })
    .await
    .unwrap();
    // Let the actual call monitor cross its former media-only cleanup path,
    // and discard packets admitted before the withdrawal commit.
    tokio::time::sleep(Duration::from_millis(350)).await;
    let drain = tokio::time::Instant::now() + Duration::from_millis(250);
    let mut bytes = [0; 4096];
    while let Ok(Ok(_)) = tokio::time::timeout_at(drain, peer.media.recv_from(&mut bytes)).await {}
    let (_, marked) = tokio::join!(
        probe(native, 0, &[], "initial-signal-only-no-media"),
        sip_audio(peer, 0, Duration::from_millis(2600))
    );
    assert_eq!(
        marked, 0,
        "named signal permission authorized RTP after media withdrawal"
    );
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(db.query_row("SELECT count(*) FROM initial_telephone_answers WHERE connected_at IS NOT NULL AND retired_at IS NULL",[],|r|r.get::<_,u64>(0)).unwrap(),1);
    assert!(!peer.methods.lock().unwrap().contains(&Method::Bye));
    assert_eq!(peer.unique(), 1);
}

pub(super) async fn exercise(
    f: &Fixture,
    peer: &PeerSip,
    client: &mut CanonicalClient,
    cid: &str,
    algorithm: &str,
    mode: &str,
) {
    let operation = format!("native-sip-{mode}");
    let scope = CommandScope {
        conversation_id: Some(cid.into()),
        session_id: Some("native-session".into()),
        connection_id: Some("initial-native".into()),
        operation_id: Some(operation.clone()),
        expected_revision: None,
    };
    client
        .send_native_dtmf(scope.clone(), "58#", 1000, "info")
        .await
        .unwrap();
    let end = tokio::time::Instant::now() + Duration::from_secs(8);
    while peer.info.lock().unwrap().is_empty() {
        assert!(
            tokio::time::Instant::now() < end,
            "first actual INFO was not received"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(peer.info.lock().unwrap().len(), 1);
    let mut config = canonical_http_config(f);
    if mode == "info-source-revoke" {
        let store = f.server.store.clone();
        let admin = store.authenticate(&f.admin).unwrap();
        let key = store
            .authenticate(&f.client.credential)
            .unwrap()
            .credential_id;
        tokio::task::spawn_blocking(move || store.revoke(&admin, &key).unwrap())
            .await
            .unwrap();
        assert!(f.server.store.authenticate(&f.client.credential).is_err());
        config.credential = f
            .server
            .store
            .issue(
                &f.client.identity_id,
                Scope::operator(),
                chrono::Utc::now().timestamp() + 600,
            )
            .unwrap()
            .token;
    }
    let mut http = CanonicalHttpClient::open(config).await.unwrap();
    if mode == "info-cancel" {
        let receipt = http
            .request_value(
                "operation.cancel",
                json!({"operation_id":operation}),
                CommandScope {
                    conversation_id: Some(cid.into()),
                    operation_id: Some(format!("cancel-{operation}")),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        receipt.ack().unwrap();
    }
    let end = tokio::time::Instant::now() + Duration::from_secs(8);
    let stopped = loop {
        let record = http
            .operation_get(
                &operation,
                CommandScope {
                    conversation_id: Some(cid.into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        if record.state == OperationState::Unknown
            && record
                .problem
                .as_ref()
                .is_some_and(|p| p["signal"]["local_work_complete"] == true)
        {
            break record;
        }
        assert!(
            tokio::time::Instant::now() < end,
            "actual SIP stop did not settle uncertainty: {record:?}"
        );
        assert!(
            matches!(
                record.state,
                OperationState::Preparing | OperationState::Unknown
            ),
            "unexpected stop result: {record:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let signal = &stopped.problem.as_ref().unwrap()["signal"];
    assert_eq!(signal["signal_source"], "uctp-native");
    assert_eq!(signal["local_submission_only"], true);
    assert_eq!(signal["recipients"].as_array().unwrap().len(), 1);
    assert_eq!(signal["recipients"][0]["disposition"], "outcome_unknown");
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(db.query_row("SELECT count(*) FROM native_dtmf_gateways g JOIN operations o ON o.record_id=g.operation_record_id WHERE o.operation_id=?1 AND g.attempted=1 AND g.disposition='outcome_unknown'",[&operation],|r|r.get::<_,u64>(0)).unwrap(),1);
    if mode == "info-cancel" {
        let replay = client
            .send_native_dtmf(scope, "58#", 1000, "info")
            .await
            .unwrap();
        assert_eq!(replay.state, stopped.state);
        assert_eq!(replay.problem, stopped.problem);
    } else {
        assert!(client
            .send_native_dtmf(scope, "58#", 1000, "info")
            .await
            .is_err());
        let replay = http
            .operation_get(
                &operation,
                CommandScope {
                    conversation_id: Some(cid.into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(replay.problem, stopped.problem);
    }
    // Cross the original 1000ms tone +70ms gap, not only a short quiet poll.
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let packets = peer.info.lock().unwrap();
    assert_eq!(
        packets.len(),
        1,
        "cancelled/revoked original INFO suffix resumed"
    );
    let Message::Request(request) = parse_message(&packets[0]).unwrap() else {
        panic!()
    };
    assert_eq!(
        std::str::from_utf8(&request.body).unwrap(),
        "Signal=5\r\nDuration=1000\r\n"
    );
    drop(packets);
    assert_eq!(peer.unique(), 1);
    assert!(!peer.methods.lock().unwrap().contains(&Method::Bye));
    if let Some(path) = std::env::var_os("CONNECT_SIP_DTMF_STOP_OBSERVATIONS") {
        use std::io::Write;
        let path = std::path::PathBuf::from(path);
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(".local/gate-6");
        assert!(path.is_absolute() && path.starts_with(root));
        let mut bytes = serde_json::to_vec(&json!({"algorithm":algorithm,"mode":mode,"stopped":stopped,"sip_calls":peer.unique(),"info_packets":1})).unwrap();
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
