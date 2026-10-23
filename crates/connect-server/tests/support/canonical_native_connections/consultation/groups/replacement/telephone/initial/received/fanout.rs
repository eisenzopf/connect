//! One SIP input reaches distinct native connections of the same Participant.
use super::*;
use rvoip_uctp::v2::{payloads::OperationState, Envelope, Scope as WireScope};
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_initial_sip_received_fanout_keeps_sibling_connections_distinct_and_revocation_local(
) {
    for algorithm in ["EdDSA", "ES256"] {
        initial_selected_signal_case(
            false,
            algorithm,
            EndpointTransport::Telephone,
            "participant",
            false,
            Some("received-fanout"),
        )
        .await;
    }
}
async fn signal(client: &mut CanonicalClient) -> Envelope {
    for _ in 0..128 {
        let frame = client.receive().await.unwrap();
        if frame.kind() == "dtmf.received" {
            return frame;
        }
    }
    panic!("received fanout signal missing behind lifecycle hints")
}
pub(in super::super) async fn exercise(
    f: &Fixture,
    peer: &PeerSip,
    primary: &mut Peer,
    cid: &str,
    algorithm: &str,
) {
    let mut grant = Scope::collaboration(vec!["*".into()]);
    grant.operations.extend([
        "stream.subscribe".into(),
        "dtmf.receive".into(),
        "operation.get".into(),
    ]);
    let sibling_config =
        party_with_scope(f, &f.client.identity_id, 143, vec!["audio".into()], grant);
    let sibling_key = f
        .server
        .store
        .authenticate(&sibling_config.credential)
        .unwrap()
        .credential_id;
    let mut sibling = CanonicalClient::open(sibling_config).await.unwrap();
    let answer = negotiate_named(
        &mut sibling,
        cid,
        "received-sibling-offer",
        "received-sibling",
        "received-sibling-audio",
    )
    .await;
    wait_native_operation(f, &f.client.credential, cid, "received-sibling-offer").await;
    stream_control(
        &mut sibling,
        cid,
        "received-sibling",
        "received-sibling-select",
        1,
        "stream.subscribe",
        json!({"subscriptions":[{"strm_id":"telephone-audio"}]}),
    )
    .await;
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let a: (String, String) = db
        .query_row(
            "SELECT participant_id,identity_id FROM connections WHERE id='initial-native'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let b: (String, String) = db
        .query_row(
            "SELECT participant_id,identity_id FROM connections WHERE id='received-sibling'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(a, b);
    let handle = answer.original()["payload"]["substrate_setup"]["streams"][0]["stream_local_id"]
        .as_u64()
        .unwrap() as u16;
    let mut sibling = [Peer {
        client: sibling,
        config: None,
        identity: b.1.clone(),
        participant: b.0.clone(),
        id: "received-sibling".into(),
        handle,
        clock: tokio::time::Instant::now(),
        sequence: 0,
        live: true,
    }];
    sip_native_duplex(peer, &mut sibling, 0, "received-sibling-actual-duplex").await;
    let owner = f
        .server
        .store
        .issue(
            &f.peer.identity_id,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let mut config = canonical_http_config(f);
    config.credential = owner.token;
    let mut http = CanonicalHttpClient::open(config).await.unwrap();
    let scope = CommandScope {
        conversation_id: Some(cid.into()),
        session_id: Some("native-session".into()),
        ..Default::default()
    };
    let (packet, address) = send_info(peer).await;
    let (left, right) = tokio::join!(
        tokio::time::timeout(Duration::from_secs(5), signal(&mut primary.client)),
        tokio::time::timeout(Duration::from_secs(5), signal(&mut sibling[0].client))
    );
    let left = left.unwrap();
    let right = right.unwrap_or_else(|error| panic!("sibling delivery missing: {error:?}; durable sources={:?}", db.query_row("SELECT group_concat(o.state||':'||COALESCE(o.problem_json,'')) FROM operations o JOIN received_dtmf_sources s ON s.operation_record_id=o.record_id",[],|r|r.get::<_,Option<String>>(0)).unwrap()));
    assert_ne!(left.id(), right.id());
    assert_eq!(left.scope(WireScope::Connection), Some("initial-native"));
    assert_eq!(right.scope(WireScope::Connection), Some("received-sibling"));
    let operation = left.scope(WireScope::Operation).unwrap().to_owned();
    assert_eq!(right.scope(WireScope::Operation), Some(operation.as_str()));
    for frame in [&left, &right] {
        assert_eq!(frame.original()["payload"]["source"], "sip-info");
        assert_eq!(frame.original()["payload"]["digits"], "5");
        assert_eq!(frame.original()["payload"]["duration_ms"], 237);
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    let completed = loop {
        let r = http.operation_get(&operation, scope.clone()).await.unwrap();
        if r.state == OperationState::Completed {
            break r;
        }
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let recipients = completed.result.as_ref().unwrap()["recipients"]
        .as_array()
        .unwrap();
    assert_eq!(recipients.len(), 2);
    assert_eq!(
        recipients[0]["participant_id"],
        recipients[1]["participant_id"]
    );
    assert_eq!(
        recipients[0]["membership_epoch"],
        recipients[1]["membership_epoch"]
    );
    assert_ne!(
        recipients[0]["connection_id"],
        recipients[1]["connection_id"]
    );
    for frame in [&left, &right] {
        assert!(recipients.iter().any(|r| r["connection_id"]
            == frame.scope(WireScope::Connection).unwrap()
            && r["frame_id"] == frame.id()
            && r["disposition"] == "local_submitted"));
    }
    let store = f.server.store.clone();
    let admin = f.admin.clone();
    tokio::task::spawn_blocking(move || {
        let actor = store.authenticate(&admin).unwrap();
        store.revoke(&actor, &sibling_key).unwrap();
    })
    .await
    .unwrap();
    until(|| {
        db.query_row(
            "SELECT state FROM connections WHERE id='received-sibling'",
            [],
            |r| r.get::<_, String>(0),
        )
        .unwrap()
            == "retired"
    })
    .await;
    assert_eq!(
        db.query_row(
            "SELECT state FROM connections WHERE id='initial-native'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "ready"
    );
    let mut next = Box::pin(signal(&mut primary.client));
    peer.socket
        .send_to(packet.as_bytes(), address)
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(200), next.as_mut())
            .await
            .is_err()
    );
    send_rfc(peer).await;
    let recovered = tokio::time::timeout(Duration::from_secs(5), next.as_mut())
        .await
        .unwrap();
    drop(next);
    assert_eq!(
        recovered.scope(WireScope::Connection),
        Some("initial-native")
    );
    assert_eq!(recovered.original()["payload"]["source"], "rfc4733");
    let next_id = recovered.scope(WireScope::Operation).unwrap();
    assert_ne!(next_id, operation);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    let after = loop {
        let r = http.operation_get(next_id, scope.clone()).await.unwrap();
        if r.state == OperationState::Completed {
            break r;
        }
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_eq!(
        after.result.as_ref().unwrap()["recipients"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        after.result.as_ref().unwrap()["recipients"][0]["connection_id"],
        "initial-native"
    );
    assert_eq!(
        serde_json::to_value(http.operation_get(&operation, scope).await.unwrap()).unwrap(),
        serde_json::to_value(&completed).unwrap()
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM received_dtmf_sources", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM received_dtmf_recipients", [], |r| r
            .get::<_, u64>(
            0
        ))
        .unwrap(),
        3
    );
    assert_eq!(peer.unique(), 1);
    drop(sibling);
    if let Some(path) = std::env::var_os("CONNECT_RECEIVED_DTMF_FANOUT_OBSERVATIONS") {
        use std::io::Write;
        let path = std::path::PathBuf::from(path);
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(".local/gate-6");
        assert!(path.is_absolute() && path.starts_with(root));
        let mut bytes=serde_json::to_vec(&json!({"algorithm":algorithm,"first":completed,"after_revocation":after,"first_frames":[left.original(),right.original()],"after_frame":recovered.original(),"same_participant":true,"calls":peer.unique()})).unwrap();
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
