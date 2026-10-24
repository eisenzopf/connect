//! Real IO and journal checks; fenced credential edits are isolated fault setup.
use super::*;

fn count(db: &rusqlite::Connection) -> u64 {
    db.query_row("SELECT count(*) FROM received_dtmf_sources", [], |r| {
        r.get(0)
    })
    .unwrap()
}
fn credential(db: &rusqlite::Connection, key: &str) -> (String, Vec<String>) {
    let (identity, operations): (String, String) = db
        .query_row(
            "SELECT identity_id,operations FROM credentials WHERE id=?1",
            [key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    (identity, serde_json::from_str(&operations).unwrap())
}
fn without(operations: &[String], denied: &str) -> Vec<String> {
    let explicit = [
        "connect:control",
        "connection.offer",
        "session.accept",
        "stream.subscribe",
        "dtmf.send",
        "dtmf.receive",
        "sip_dial",
        "operation.get",
        "conversation.get",
        "session.get",
    ];
    let operations = if operations.iter().any(|p| p == "*") {
        explicit.into_iter().map(str::to_owned).collect::<Vec<_>>()
    } else {
        operations.to_vec()
    };
    operations.into_iter().filter(|p| p != denied).collect()
}
async fn permissions(f: &Fixture, identity: &str, key: &str, operations: Vec<String>) {
    let store = f.server.store.clone();
    let path = f.config.database.clone();
    let identity = identity.to_owned();
    let key = key.to_owned();
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
        assert_eq!(
            rusqlite::Connection::open(path)
                .unwrap()
                .execute(
                    "UPDATE credentials SET operations=?2 WHERE id=?1",
                    rusqlite::params![key, serde_json::to_string(&operations).unwrap()]
                )
                .unwrap(),
            1
        );
    })
    .await
    .unwrap();
}
async fn response(peer: &PeerSip, packet: &str) -> String {
    let sequence = packet
        .lines()
        .find_map(|l| l.strip_prefix("CSeq: "))
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse::<u32>()
        .unwrap();
    let until = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        if let Some((_, status)) = peer
            .info_responses
            .lock()
            .unwrap()
            .iter()
            .find(|(s, _)| *s == sequence)
        {
            return status.clone();
        }
        assert!(
            tokio::time::Instant::now() < until,
            "exact SIP INFO received no correlated response"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
async fn quiet(peers: &mut [Peer]) {
    let (a, b) = peers.split_at_mut(1);
    let (a, b) = tokio::join!(
        tokio::time::timeout(Duration::from_millis(200), signal(&mut a[0].client)),
        tokio::time::timeout(Duration::from_millis(200), signal(&mut b[0].client))
    );
    assert!(
        a.is_err() && b.is_err(),
        "withdrawn private signal permission leaked a frame"
    );
}
async fn recovered(
    peer: &PeerSip,
    peers: &mut [Peer],
    http: &mut CanonicalHttpClient,
    cid: &str,
) -> (Value, Value) {
    let (packet, _) = send_info(peer).await;
    response(peer, &packet).await;
    let frame = tokio::time::timeout(Duration::from_secs(5), signal(&mut peers[1].client))
        .await
        .unwrap()
        .unwrap();
    let status = settled(http, frame.scope(WireScope::Operation).unwrap(), cid).await;
    assert_eq!(status["state"], "completed");
    assert_eq!(status["result"]["recipients"].as_array().unwrap().len(), 1);
    assert_eq!(
        status["result"]["recipients"][0]["connection_id"],
        peers[1].id
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(200), signal(&mut peers[0].client))
            .await
            .is_err()
    );
    (frame.original().clone(), status)
}

pub(in super::super) async fn exercise(
    f: &Fixture,
    peer: &PeerSip,
    peers: &mut [Peer],
    cid: &str,
    phone: &Value,
    http: &mut CanonicalHttpClient,
) -> Value {
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let key = f
        .server
        .store
        .authenticate(&peers[1].config.as_ref().unwrap().credential)
        .unwrap()
        .credential_id;
    let (native_identity, native_operations) = credential(&db, &key);
    assert!(
        native_operations.iter().any(|p| p == "dtmf.receive")
            && !native_operations.iter().any(|p| p == "*")
    );
    let before = count(&db);
    permissions(
        f,
        &native_identity,
        &key,
        native_operations
            .iter()
            .filter(|p| p.as_str() != "dtmf.receive")
            .cloned()
            .collect(),
    )
    .await;
    let (blocked, address) = send_info(peer).await;
    let receive_denied_ack = response(peer, &blocked).await;
    let until = tokio::time::Instant::now() + Duration::from_secs(4);
    let operation = loop {
        if count(&db) == before + 1 {
            break db.query_row("SELECT o.operation_id FROM operations o JOIN received_dtmf_sources s ON s.operation_record_id=o.record_id ORDER BY s.rowid DESC LIMIT 1",[],|r|r.get::<_,String>(0)).unwrap();
        }
        assert!(
            tokio::time::Instant::now() < until,
            "input with withdrawn native receive right was not durably settled"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let denied = settled(http, &operation, cid).await;
    assert_eq!(denied["state"], "failed");
    assert_eq!(denied["problem"]["code"], "signal-no-recipients");
    assert!(denied["problem"]["signal"]["recipients"]
        .as_array()
        .unwrap()
        .is_empty());
    quiet(peers).await;
    // Media rights did not change when the independent signal receive grant did.
    let media_counts = super::sip_to_native(peer, peers).await;
    assert_eq!(media_counts[0], 0);
    assert!(media_counts[1] > 10);
    let (_, media_marked) = tokio::join!(
        probe(peers, 1, &[], "private-receive-right-withdrawn-media"),
        sip_audio(peer, 0, Duration::from_millis(2600))
    );
    assert!(media_marked > 10);
    permissions(f, &native_identity, &key, native_operations.clone()).await;
    peer.socket
        .send_to(blocked.as_bytes(), address)
        .await
        .unwrap();
    response(peer, &blocked).await;
    quiet(peers).await;
    assert_eq!(count(&db), before + 1);
    assert_eq!(settled(http, &operation, cid).await, denied);
    let (receive_recovered_frame, receive_recovered) = recovered(peer, peers, http, cid).await;
    assert_ne!(receive_recovered_frame["operation_id"], operation);
    assert_eq!(count(&db), before + 2);

    let source_key = phone
        .get("endpoint_credential_id")
        .unwrap_or(&phone["route_credential_id"])
        .as_str()
        .unwrap();
    let (source_identity, source_operations) = credential(&db, source_key);
    let narrowed = without(&source_operations, "dtmf.send");
    permissions(f, &source_identity, source_key, narrowed).await;
    let (refused, refused_address) = send_info(peer).await;
    let source_denied_ack = response(peer, &refused).await;
    quiet(peers).await;
    assert_eq!(
        count(&db),
        before + 2,
        "withdrawn SIP source send right still admitted an input"
    );
    permissions(f, &source_identity, source_key, source_operations.clone()).await;
    peer.socket
        .send_to(refused.as_bytes(), refused_address)
        .await
        .unwrap();
    response(peer, &refused).await;
    quiet(peers).await;
    assert_eq!(
        count(&db),
        before + 2,
        "old refused source input replayed after restoring permission"
    );
    let (source_recovered_frame, source_recovered) = recovered(peer, peers, http, cid).await;
    assert_ne!(
        source_recovered_frame["operation_id"],
        receive_recovered_frame["operation_id"]
    );
    assert_eq!(count(&db), before + 3);

    let info_before = peer.info.lock().unwrap().len();
    permissions(
        f,
        &source_identity,
        source_key,
        without(&source_operations, "dtmf.receive"),
    )
    .await;
    let receive_scope = CommandScope {
        conversation_id: Some(cid.into()),
        session_id: Some("native-session".into()),
        connection_id: Some(peers[1].id.clone()),
        operation_id: Some("private-phone-receive-denied".into()),
        ..Default::default()
    };
    let phone_receive_denied = peers[1]
        .client
        .send_native_dtmf(receive_scope.clone(), "9", 100, "info")
        .await
        .unwrap();
    assert_eq!(
        phone_receive_denied.state,
        rvoip_uctp::v2::payloads::OperationState::Failed
    );
    assert_eq!(
        phone_receive_denied.problem.as_ref().unwrap()["code"],
        "not-recipient"
    );
    quiet(peers).await;
    let receive_denied_info = peer.info.lock().unwrap().len() - info_before;
    assert_eq!(receive_denied_info, 0);
    assert_eq!(db.query_row("SELECT count(*) FROM native_dtmf_deliveries d JOIN operations o ON o.record_id=d.operation_record_id WHERE o.operation_id='private-phone-receive-denied'",[],|r|r.get::<_,u64>(0)).unwrap(),0);
    permissions(f, &source_identity, source_key, source_operations).await;
    let phone_receive_replay = peers[1]
        .client
        .send_native_dtmf(receive_scope, "9", 100, "info")
        .await
        .unwrap();
    assert_eq!(
        phone_receive_replay.state,
        rvoip_uctp::v2::payloads::OperationState::Failed
    );
    assert_eq!(phone_receive_replay.problem, phone_receive_denied.problem);
    quiet(peers).await;
    assert_eq!(
        peer.info.lock().unwrap().len(),
        info_before,
        "failed private phone-receive operation was resubmitted after permission restore"
    );

    permissions(
        f,
        &native_identity,
        &key,
        native_operations
            .iter()
            .filter(|p| p.as_str() != "dtmf.send")
            .cloned()
            .collect(),
    )
    .await;
    let native_id = peers[1].id.clone();
    let refused = peers[1]
        .client
        .send_native_dtmf(
            CommandScope {
                conversation_id: Some(cid.into()),
                session_id: Some("native-session".into()),
                connection_id: Some(native_id),
                operation_id: Some("private-consultant-send-denied".into()),
                ..Default::default()
            },
            "9",
            100,
            "info",
        )
        .await;
    assert!(
        matches!(refused,Err(connect_client::ClientError::Application(ref e)) if e.code=="forbidden"),
        "native private sender refusal was not typed forbidden: {refused:?}"
    );
    quiet(peers).await;
    assert_eq!(
        peer.info.lock().unwrap().len(),
        info_before,
        "withdrawn native send right still sent INFO"
    );
    permissions(f, &native_identity, &key, native_operations).await;
    json!({"receive_denied":denied,"receive_denied_ack":receive_denied_ack,"receive_recovered_frame":receive_recovered_frame,"receive_recovered":receive_recovered,"source_denied_ack":source_denied_ack,"source_recovered_frame":source_recovered_frame,"source_recovered":source_recovered,"source_rows_before":before,"source_rows_after":count(&db),"refused_source_rows_added":0,"refused_native_info_added":0,"native_send_refusal":"forbidden","phone_receive_denied":phone_receive_denied,"phone_receive_replay":phone_receive_replay,"phone_receive_denied_info_added":receive_denied_info,"media_counts_with_receive_denied":media_counts,"media_marked_with_receive_denied":media_marked})
}
