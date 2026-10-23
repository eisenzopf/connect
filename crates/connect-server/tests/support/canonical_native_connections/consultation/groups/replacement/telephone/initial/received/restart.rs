//! A successor retains uncertainty when no captured local fact survived shutdown.
use super::*;
use rvoip_uctp::v2::{payloads::OperationState, Scope as WireScope};
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_initial_sip_received_restart_preserves_attempted_output_as_unknown_without_redelivery(
) {
    for algorithm in ["EdDSA", "ES256"] {
        initial_selected_signal_case(
            false,
            algorithm,
            EndpointTransport::Telephone,
            "participant",
            false,
            Some("received-outcome-restart"),
        )
        .await;
    }
}
pub(in super::super) async fn exercise(
    mut f: Fixture,
    peer: PeerSip,
    mut native: [Peer; 1],
    cid: &str,
    algorithm: &str,
) {
    let db = rusqlite::Connection::open(&f.config.database).unwrap();
    db.execute_batch("CREATE TRIGGER stop_received_restart_fact BEFORE UPDATE OF disposition ON received_dtmf_recipients BEGIN SELECT RAISE(ABORT,'controlled received restart cutoff'); END;").unwrap();
    let key = f
        .server
        .store
        .issue(
            &f.peer.identity_id,
            connect_store::Scope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let source_token = key.token;
    let mut config = canonical_http_config(&f);
    config.credential = source_token.clone();
    let mut http = CanonicalHttpClient::open(config).await.unwrap();
    let scope = CommandScope {
        conversation_id: Some(cid.into()),
        session_id: Some("native-session".into()),
        ..Default::default()
    };
    send_info(&peer).await;
    let frame = tokio::time::timeout(Duration::from_secs(5), async {
        for _ in 0..128 {
            let frame = native[0].client.receive().await.unwrap();
            if frame.kind() == "dtmf.received" {
                return frame;
            }
        }
        panic!("received restart output missing")
    })
    .await
    .unwrap();
    let operation = frame.scope(WireScope::Operation).unwrap().to_owned();
    let before = http.operation_get(&operation, scope.clone()).await.unwrap();
    assert_eq!(before.state, OperationState::Preparing);
    let expiry: i64 = db
        .query_row(
            "SELECT expires_at FROM operations WHERE operation_id=?1",
            [&operation],
            |r| r.get(0),
        )
        .unwrap();
    let epoch = f.server.store.runtime_epoch().to_owned();
    drop(native);
    drop(http);
    f.server.shutdown().await.unwrap();
    assert!(peer.methods.lock().unwrap().contains(&Method::Bye));
    assert_eq!(db.query_row("SELECT count(*) FROM received_dtmf_recipients WHERE disposition IS NULL AND attempted=1",[],|r|r.get::<_,u64>(0)).unwrap(),1);
    // No original callback can settle after this barrier. The successor must
    // recover only the attempted-frame reservation, never replay physical IO.
    db.execute_batch("DROP TRIGGER stop_received_restart_fact")
        .unwrap();
    f.server = RunningCanonicalServer::start(f.config.clone())
        .await
        .unwrap();
    assert_ne!(f.server.store.runtime_epoch(), epoch);
    f.client.endpoint = f.server.address;
    let mut config = canonical_http_config(&f);
    config.credential = source_token;
    let mut http = CanonicalHttpClient::open(config).await.unwrap();
    let recovered = http.operation_get(&operation, scope.clone()).await.unwrap();
    assert_eq!(recovered.state, OperationState::Unknown);
    assert_eq!(recovered.record_id, before.record_id);
    let summary = &recovered.problem.as_ref().unwrap()["signal"];
    assert_eq!(summary["signal_source"], "sip-info");
    assert_eq!(
        summary["received_at"],
        frame.original()["payload"]["received_at"]
    );
    assert_eq!(summary["recipients"][0]["disposition"], "outcome_unknown");
    assert_eq!(summary["recipients"][0]["frame_id"], frame.id());
    assert_eq!(
        serde_json::to_value(http.operation_get(&operation, scope.clone()).await.unwrap()).unwrap(),
        serde_json::to_value(&recovered).unwrap()
    );
    assert_eq!(
        db.query_row(
            "SELECT expires_at FROM operations WHERE operation_id=?1",
            [&operation],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        expiry
    );
    let mut observer = CanonicalClient::open(f.client.clone()).await.unwrap();
    negotiate_named(
        &mut observer,
        cid,
        "received-restart-observer-offer",
        "received-restart-observer",
        "received-restart-observer-audio",
    )
    .await;
    wait_native_operation(
        &f,
        &f.client.credential,
        cid,
        "received-restart-observer-offer",
    )
    .await;
    let mut next = Box::pin(async {
        for _ in 0..128 {
            let frame = observer.receive().await.unwrap();
            if frame.kind() == "dtmf.received" {
                return frame;
            }
        }
        panic!("restart observer exceeded lifecycle bound")
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(400), next.as_mut())
            .await
            .is_err()
    );
    drop(next);
    drop(observer);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM received_dtmf_sources WHERE closed=1",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(db.query_row("SELECT count(*) FROM received_dtmf_recipients WHERE attempted=1 AND disposition='outcome_unknown' AND provenance='runtime_recovery'",[],|r|r.get::<_,u64>(0)).unwrap(),1);
    assert_eq!(db.query_row("SELECT attempt FROM operation_effects e JOIN operations o ON o.record_id=e.operation_record_id WHERE o.operation_id=?1",[&operation],|r|r.get::<_,u32>(0)).unwrap(),1);
    assert_eq!(peer.unique(), 1);
    f.server.shutdown().await.unwrap();
    if let Some(path) = std::env::var_os("CONNECT_RECEIVED_DTMF_RESTART_OBSERVATIONS") {
        use std::io::Write;
        let path = std::path::PathBuf::from(path);
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(".local/gate-6");
        assert!(path.is_absolute() && path.starts_with(root));
        let mut bytes=serde_json::to_vec(&json!({"algorithm":algorithm,"before":before,"recovered":recovered,"frame_before_restart":frame.original(),"calls":peer.unique(),"attempts":1,"original_expiry":expiry,"final_expiry":expiry,"new_runtime":true,"new_observer_no_redelivery":true})).unwrap();
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
