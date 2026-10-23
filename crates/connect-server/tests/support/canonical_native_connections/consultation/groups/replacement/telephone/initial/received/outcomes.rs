//! Captured physical submission survives metadata failure and source revocation.
use super::*;
use rvoip_uctp::v2::{payloads::OperationState, Scope as WireScope};
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_initial_sip_received_captured_output_settles_after_source_revocation_without_replay(
) {
    for algorithm in ["EdDSA", "ES256"] {
        initial_selected_signal_case(
            false,
            algorithm,
            EndpointTransport::Telephone,
            "participant",
            false,
            Some("received-outcome-recovery"),
        )
        .await;
    }
}
pub(in super::super) async fn exercise(
    f: &Fixture,
    peer: &PeerSip,
    native: &mut Peer,
    cid: &str,
    phone_key: &str,
    algorithm: &str,
) {
    let db = rusqlite::Connection::open(&f.config.database).unwrap();
    db.execute_batch("CREATE TRIGGER stop_received_dtmf_fact BEFORE UPDATE OF disposition ON received_dtmf_recipients BEGIN SELECT RAISE(ABORT,'controlled received outcome failure'); END;").unwrap();
    let fresh = f
        .server
        .store
        .issue(
            &f.peer.identity_id,
            connect_store::Scope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let mut config = canonical_http_config(f);
    config.credential = fresh.token;
    let mut http = CanonicalHttpClient::open(config).await.unwrap();
    let scope = CommandScope {
        conversation_id: Some(cid.into()),
        session_id: Some("native-session".into()),
        ..Default::default()
    };
    let (packet, address) = send_info(peer).await;
    let frame = tokio::time::timeout(Duration::from_secs(5), async {
        for _ in 0..128 {
            let frame = native.client.receive().await.unwrap();
            if frame.kind() == "dtmf.received" {
                return frame;
            }
        }
        panic!("captured received source output missing")
    })
    .await
    .unwrap();
    assert_eq!(frame.original()["payload"]["source"], "sip-info");
    assert_eq!(frame.original()["payload"]["digits"], "5");
    assert_eq!(frame.original()["payload"]["duration_ms"], 237);
    let operation = frame.scope(WireScope::Operation).unwrap().to_owned();
    let before = http.operation_get(&operation, scope.clone()).await.unwrap();
    assert_eq!(before.state, OperationState::Preparing);
    assert!(before.result.is_none());
    let reserved:(bool,String,Option<String>,i64)=db.query_row("SELECT r.attempted,r.frame_id,r.disposition,o.expires_at FROM received_dtmf_recipients r JOIN operations o ON o.record_id=r.operation_record_id WHERE o.operation_id=?1",[&operation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert!(reserved.0);
    assert_eq!(reserved.1, frame.id());
    assert!(reserved.2.is_none());
    // Public revocation must acquire the actual source writer fence while the
    // outcome trigger is still failing. No idle IO reader may delay this commit.
    let store = f.server.store.clone();
    let admin = f.admin.clone();
    let phone_key = phone_key.to_owned();
    let revocation = tokio::time::Instant::now();
    tokio::time::timeout(
        Duration::from_secs(3),
        tokio::task::spawn_blocking(move || {
            let actor = store.authenticate(&admin).unwrap();
            store.revoke(&actor, &phone_key).unwrap();
        }),
    )
    .await
    .unwrap()
    .unwrap();
    let revoked_ms = revocation.elapsed().as_millis();
    until(|| {
        db.query_row(
            "SELECT count(*) FROM initial_telephone_answers WHERE retired_at IS NOT NULL",
            [],
            |r| r.get::<_, u64>(0),
        )
        .unwrap()
            == 1
    })
    .await;
    assert!(peer.methods.lock().unwrap().contains(&Method::Bye));
    let mut next = Box::pin(async {
        for _ in 0..128 {
            let frame = native.client.receive().await.unwrap();
            if frame.kind() == "dtmf.received" {
                return frame;
            }
        }
        panic!("signal queue exceeded lifecycle bound")
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(150), next.as_mut())
            .await
            .is_err()
    );
    db.execute_batch("DROP TRIGGER stop_received_dtmf_fact")
        .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    let completed = loop {
        let r = http.operation_get(&operation, scope.clone()).await.unwrap();
        if r.state == OperationState::Completed {
            break r;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "captured local outcome did not settle after source retirement"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let summary = completed.result.as_ref().unwrap();
    assert_eq!(summary["signal_source"], "sip-info");
    assert_eq!(
        summary["received_at"],
        frame.original()["payload"]["received_at"]
    );
    assert_eq!(summary["recipients"][0]["frame_id"], frame.id());
    assert_eq!(summary["recipients"][0]["disposition"], "local_submitted");
    assert_eq!(
        serde_json::to_value(http.operation_get(&operation, scope.clone()).await.unwrap()).unwrap(),
        serde_json::to_value(&completed).unwrap()
    );
    // Replaying the old physical request after retirement cannot recreate the
    // source, recipient attempt or the original signed native output.
    peer.socket
        .send_to(packet.as_bytes(), address)
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(250), next.as_mut())
            .await
            .is_err()
    );
    drop(next);
    assert_eq!(
        db.query_row("SELECT count(*) FROM received_dtmf_sources", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        1
    );
    assert_eq!(db.query_row("SELECT count(*) FROM received_dtmf_recipients WHERE attempted=1 AND disposition='local_submitted'",[],|r|r.get::<_,u64>(0)).unwrap(),1);
    let after:(i64,u32,String)=db.query_row("SELECT o.expires_at,e.attempt,e.state FROM operations o JOIN operation_effects e ON e.operation_record_id=o.record_id WHERE o.operation_id=?1",[&operation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(after, (reserved.3, 1, "confirmed".into()));
    assert_eq!(peer.unique(), 1);
    if let Some(path) = std::env::var_os("CONNECT_RECEIVED_DTMF_OUTCOME_OBSERVATIONS") {
        use std::io::Write;
        let path = std::path::PathBuf::from(path);
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(".local/gate-6");
        assert!(path.is_absolute() && path.starts_with(root));
        let mut bytes=serde_json::to_vec(&json!({"algorithm":algorithm,"preparing":before,"completed":completed,"frame":frame.original(),"calls":peer.unique(),"attempts":after.1,"original_expiry":reserved.3,"final_expiry":after.0,"revocation_ms":revoked_ms,"source_retired":true})).unwrap();
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
