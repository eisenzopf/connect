//! Installed Store/service/Core/SDK signal path on real signed native QUIC.
use super::*;
use rvoip_uctp::v2::payloads::OperationState;
async fn fixture(algorithm: &str, receive: bool) -> codecs::Running {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "connect_service=debug".into()),
        )
        .with_test_writer()
        .try_init();
    let f = Fixture::with_native_dtmf_scope(algorithm, 12).await;
    codecs::setup_started_dtmf(f, receive).await
}
async fn received(client: &mut CanonicalClient, operation: &str) -> Envelope {
    for _ in 0..128 {
        let frame = client.receive().await.unwrap();
        if lifecycle::session_start(&frame) {
            continue;
        }
        if frame.kind() == "dtmf.received" {
            assert_eq!(
                frame.scope(rvoip_uctp::v2::Scope::Operation),
                Some(operation)
            );
            return frame;
        }
        assert!(
            matches!(
                frame.kind(),
                "connection.ready" | "stream.opened" | "stream.closed" | "connection.quality"
            ),
            "unexpected frame {:?}",
            frame.original()
        );
    }
    panic!("signal missing behind lifecycle hints")
}
fn record(s: &codecs::Running, operation: &str) -> connect_protocol::transitions::OperationRecord {
    let actor =
        s.f.server
            .store
            .authenticate(&s.f.client.credential)
            .unwrap();
    s.f.server
        .store
        .execute_operation_action(
            &actor.credential_id,
            connect_protocol::transitions::OperationAction::Get {
                operation_id: operation.into(),
            },
        )
        .unwrap()
}
async fn complete(
    s: &codecs::Running,
    operation: &str,
) -> connect_protocol::transitions::OperationRecord {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let r = record(s, operation);
        if r.state == connect_protocol::transitions::OperationState::Completed {
            return r;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "signal not completed: {r:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
#[cfg(feature = "test-support")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::await_holding_lock)] // Deliberate actual Core metadata contention.
async fn production_native_dtmf_metadata_contention_waits_without_io_readers_and_keeps_original_intent(
) {
    use connect_store::{AuthorityMode, AuthorityScope};
    for algorithm in ["EdDSA", "ES256"] {
        let mut s = fixture(algorithm, true).await;
        let metadata_session =
            s.f.server
                .service
                .canonical_session_metadata_for_test("native-session")
                .unwrap();
        let metadata = metadata_session.write().unwrap();
        let scope = capabilities::scope(&s.cid, "signal-metadata-busy", 2);
        s.clients[1]
            .send_native_dtmf(scope.clone(), "8#", 100, "info")
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let current = record(&s, "signal-metadata-busy");
        if current.state != connect_protocol::transitions::OperationState::Preparing {
            drop(metadata);
            panic!(
                "transient actual Core metadata contention became terminal: {}",
                serde_json::to_value(&current).unwrap()
            );
        }
        let db = rusqlite::Connection::open(&s.f.config.database).unwrap();
        assert_eq!(db.query_row("SELECT count(*) FROM native_dtmf_recipients r JOIN operations o ON o.record_id=r.operation_record_id WHERE o.operation_id='signal-metadata-busy' AND (r.attempted=1 OR r.frame_id IS NOT NULL)", [], |r| r.get::<_, i64>(0)).unwrap(),0);
        let scopes = [&s.f.client.identity_id, &s.f.peer.identity_id]
            .into_iter()
            .map(|id| AuthorityScope::identity(id, AuthorityMode::Write).unwrap())
            .collect::<Vec<_>>();
        let writers =
            s.f.server
                .store
                .try_acquire_authority(&scopes)
                .expect("idle Core metadata wait must retain no source/recipient readers");
        drop(writers);
        drop(metadata);
        let frame = received(&mut s.clients[0], "signal-metadata-busy").await;
        assert_eq!(frame.original()["payload"]["digits"], "8#");
        let original = complete(&s, "signal-metadata-busy").await;
        assert_eq!(
            original.result.as_ref().unwrap()["recipients"][0]["frame_id"],
            frame.id()
        );
        let replay = s.clients[1]
            .send_native_dtmf(scope, "8#", 100, "info")
            .await
            .unwrap();
        assert_eq!(replay.result, original.result);
        let barrier = s.clients[0].keepalive().await.unwrap();
        loop {
            let frame = s.clients[0].receive().await.unwrap();
            assert_ne!(
                frame.kind(),
                "dtmf.received",
                "metadata recovery/replay duplicated IO"
            );
            if frame.kind() == "ack" {
                assert_eq!(frame.original()["in_reply_to"], barrier);
                break;
            }
        }
        drop(db);
        drop(s.clients);
        s.f.server.shutdown().await.unwrap();
    }
}
#[tokio::test]
async fn production_native_dtmf_signed_sdk_delivery_and_replay_preserve_original_effects() {
    for algorithm in ["EdDSA", "ES256"] {
        let mut s = fixture(algorithm, true).await;
        let scope = capabilities::scope(&s.cid, "signal-original", 2);
        let receipt = s.clients[1]
            .send_native_dtmf(scope.clone(), "12aB#", 100, "info")
            .await
            .unwrap();
        assert!(matches!(
            receipt.state,
            OperationState::Preparing | OperationState::Completed
        ));
        let frame = received(&mut s.clients[0], "signal-original").await;
        assert_eq!(frame.original()["cid"], s.cid);
        assert_eq!(frame.original()["sid"], "native-session");
        assert_eq!(
            frame.scope(rvoip_uctp::v2::Scope::Connection),
            Some("codec-source")
        );
        assert_eq!(frame.original()["payload"]["digits"], "12aB#");
        assert_eq!(frame.original()["payload"]["duration_ms"], 100);
        assert_eq!(frame.original()["payload"]["source"], "uctp-native");
        let observed_at = chrono::DateTime::parse_from_rfc3339(
            frame.original()["payload"]["received_at"].as_str().unwrap(),
        )
        .unwrap();
        let published_at =
            chrono::DateTime::parse_from_rfc3339(frame.original()["ts"].as_str().unwrap()).unwrap();
        assert!(observed_at <= published_at);
        let original = complete(&s, "signal-original").await;
        let summary = original.result.as_ref().unwrap();
        assert_eq!(summary["recipients"][0]["frame_id"], frame.id());
        assert_eq!(summary["recipients"][0]["disposition"], "local_submitted");
        assert_eq!(
            summary["received_at"],
            frame.original()["payload"]["received_at"]
        );
        assert_eq!(summary["local_submission_only"], true);
        assert!(summary.get("digits").is_none());
        let replay = s.clients[1]
            .send_native_dtmf(scope.clone(), "12aB#", 100, "info")
            .await
            .unwrap();
        assert_eq!(replay.state, OperationState::Completed);
        assert_eq!(replay.result, original.result);
        let conflict = s.clients[1].send_native_dtmf(scope, "6", 100, "info").await;
        assert!(conflict.is_err());
        let id = s.clients[0].keepalive().await.unwrap();
        loop {
            let f = s.clients[0].receive().await.unwrap();
            assert_ne!(f.kind(), "dtmf.received", "replay produced another signal");
            if f.kind() == "ack" {
                assert_eq!(f.original()["in_reply_to"], id);
                break;
            }
        }
        let count: i64 = rusqlite::Connection::open(&s.f.config.database)
            .unwrap()
            .query_row("SELECT count(*) FROM native_dtmf_recipients", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);
        drop(s.clients);
        s.f.server.shutdown().await.unwrap();
    }
}
#[tokio::test]
async fn production_native_dtmf_requires_named_receive_permission_without_borrowing_subscription() {
    let mut s = fixture("EdDSA", false).await;
    let reply = s.clients[1]
        .send_native_dtmf(
            capabilities::scope(&s.cid, "signal-no-receive", 2),
            "5",
            100,
            "rfc4733",
        )
        .await
        .unwrap();
    assert_eq!(reply.state, OperationState::Failed);
    assert_eq!(reply.problem.as_ref().unwrap()["code"], "not-recipient");
    assert!(record(&s, "signal-no-receive").result.is_none());
    let db = rusqlite::Connection::open(&s.f.config.database).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM native_dtmf_sources", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(db);
    drop(s.clients);
    s.f.server.shutdown().await.unwrap();
}
#[tokio::test]
async fn production_native_dtmf_writer_fact_storage_failure_recovers_without_another_delivery() {
    let mut s = fixture("ES256", true).await;
    let db = rusqlite::Connection::open(&s.f.config.database).unwrap();
    db.execute_batch("CREATE TRIGGER fail_signal_fact BEFORE UPDATE OF disposition ON native_dtmf_recipients BEGIN SELECT RAISE(ABORT,'controlled signal fact failure'); END;").unwrap();
    s.clients[1]
        .send_native_dtmf(
            capabilities::scope(&s.cid, "signal-storage", 2),
            "7#",
            100,
            "info",
        )
        .await
        .unwrap();
    let frame = received(&mut s.clients[0], "signal-storage").await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        record(&s, "signal-storage").state,
        connect_protocol::transitions::OperationState::Preparing
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM native_dtmf_recipients WHERE disposition IS NOT NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    db.execute_batch("DROP TRIGGER fail_signal_fact").unwrap();
    let result = complete(&s, "signal-storage").await;
    assert_eq!(
        result.result.unwrap()["recipients"][0]["frame_id"],
        frame.id()
    );
    drop(db);
    drop(s.clients);
    s.f.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn production_native_dtmf_queued_during_another_sdk_command_keeps_actual_provenance_and_order(
) {
    let mut s = fixture("EdDSA", true).await;
    let payload = json!({"digits":"9#","duration_ms":100,"method":"info","source":"sip-info","observation_id":"invented-payload-observation","future":{"exact":true}});
    let batch = s.clients[1]
        .request_value(
            "dtmf.send",
            payload,
            capabilities::scope(&s.cid, "signal-queued", 2),
        )
        .await
        .unwrap();
    let mut back = capabilities::scope(&s.cid, "signal-back", 2);
    back.connection_id = Some("codec-source".into());
    s.clients[0]
        .send_native_dtmf(back, "3", 100, "rfc4733")
        .await
        .unwrap();
    let forward = received(&mut s.clients[0], "signal-queued").await;
    let backward = received(&mut s.clients[1], "signal-back").await;
    assert_eq!(forward.original()["payload"]["source"], "uctp-native");
    assert_eq!(forward.original()["payload"]["digits"], "9#");
    assert_eq!(backward.original()["payload"]["digits"], "3");
    let outcome = complete(&s, "signal-queued").await;
    let result = outcome.result.unwrap();
    assert_eq!(result["source_observation_id"], batch.request_id);
    assert_ne!(
        result["source_observation_id"],
        "invented-payload-observation"
    );
    assert_eq!(result["recipients"][0]["frame_id"], forward.id());
    drop(s.clients);
    s.f.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn production_native_dtmf_http_cannot_borrow_native_source_custody() {
    let s = fixture("EdDSA", true).await;
    let mut http = CanonicalHttpClient::open(canonical_http_config(&s.f))
        .await
        .unwrap();
    let response = http
        .request_value(
            "dtmf.send",
            json!({"digits":"5","duration_ms":100,"method":"info"}),
            capabilities::scope(&s.cid, "signal-http-refused", 2),
        )
        .await;
    let error = match response {
        Ok(batch) => batch.ack().unwrap_err(),
        Err(e) => e,
    };
    assert!(
        matches!(error,ClientError::Application(e) if matches!(e.code.as_str(),"unsupported-media-profile"|"unsupported-command"))
    );
    let db = rusqlite::Connection::open(&s.f.config.database).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM operations WHERE operation_id='signal-http-refused'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(db);
    drop(s.clients);
    s.f.server.shutdown().await.unwrap();
}

pub(super) fn signal_scope() -> Scope {
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope
        .operations
        .extend(["dtmf.send".into(), "dtmf.receive".into()]);
    scope
}
fn stream_revision(f: &Fixture, connection: &str) -> u64 {
    rusqlite::Connection::open(&f.config.database)
        .unwrap()
        .query_row(
            "SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1",
            [connection],
            |r| r.get(0),
        )
        .unwrap()
}
async fn http_settled(
    http: &mut CanonicalHttpClient,
    cid: &str,
    operation: &str,
) -> rvoip_uctp::v2::payloads::OperationUpdated {
    let end = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let r = http
            .operation_get(
                operation,
                CommandScope {
                    conversation_id: Some(cid.into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        if matches!(
            r.state,
            OperationState::Completed
                | OperationState::Failed
                | OperationState::Cancelled
                | OperationState::Unknown
        ) && (r.state != OperationState::Unknown
            || r.problem
                .as_ref()
                .is_some_and(|p| p["signal"]["local_work_complete"] == true))
        {
            return r;
        }
        assert!(
            tokio::time::Instant::now() < end,
            "signal remains in progress: {r:?}"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}
fn public_offer(label: &str) -> Value {
    json!({"substrate":"quic","capabilities":{"audio_codecs":[{"name":"pcmu","params":{"sample_rate":8000,"channels":1}}],"max_streams_per_connection":8,"identity_assurance_offered":"identified"},"streams_offered":[{"id":label,"kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}]})
}
pub(super) async fn owned(
    config: CanonicalClientConfig,
    cid: &str,
    connection: &str,
    operation: &str,
) -> connect_client::canonical::PublicNativeAudio {
    CanonicalClient::open(config)
        .await
        .unwrap()
        .prepare_public_native_audio(
            CommandScope {
                conversation_id: Some(cid.into()),
                session_id: Some("native-session".into()),
                connection_id: Some(connection.into()),
                operation_id: Some(operation.into()),
                expected_revision: Some(0),
            },
            public_offer(&format!("{connection}-voice")),
        )
        .await
        .unwrap()
}
#[tokio::test]
async fn production_native_dtmf_http_status_projects_actual_local_frame_summary() {
    let mut samples = Vec::new();
    for algorithm in ["EdDSA", "ES256"] {
        let mut s = fixture(algorithm, true).await;
        s.clients[1]
            .send_native_dtmf(
                capabilities::scope(&s.cid, "signal-http-summary", 2),
                "8#",
                100,
                "info",
            )
            .await
            .unwrap();
        let frame = received(&mut s.clients[0], "signal-http-summary").await;
        let mut http = CanonicalHttpClient::open(canonical_http_config(&s.f))
            .await
            .unwrap();
        let result = http_settled(&mut http, &s.cid, "signal-http-summary").await;
        assert_eq!(result.state, OperationState::Completed);
        assert_eq!(result.result, record(&s, "signal-http-summary").result);
        assert_eq!(
            result.result.as_ref().unwrap()["recipients"][0]["frame_id"],
            frame.id()
        );
        samples.push(json!({"algorithm":algorithm,"http_operation":result,"actual_received":frame.original()}));
        drop(s.clients);
        s.f.server.shutdown().await.unwrap();
    }
    if let Ok(path) = std::env::var("CONNECT_G6_DTMF_HTTP_SAMPLE") {
        std::fs::write(path, serde_json::to_vec_pretty(&samples).unwrap()).unwrap();
    }
}
#[tokio::test]
async fn production_owned_audio_dtmf_uses_its_exact_source_and_preserves_queued_received_signals() {
    for algorithm in ["EdDSA", "ES256"] {
        let mut s = fixture(algorithm, true).await;
        let config = party_with_scope(
            &s.f,
            &s.f.peer.identity_id,
            107,
            vec!["audio".into()],
            signal_scope(),
        );
        let mut http_config = canonical_http_config(&s.f);
        http_config.credential = config.credential.clone();
        let mut audio = owned(config, &s.cid, "signal-owned", "signal-owned-offer").await;
        audio
            .send_dtmf("signal-owned-out", Some(1), "4#", 100, "info")
            .await
            .unwrap();
        received(&mut s.clients[0], "signal-owned-out").await;
        received(&mut s.clients[1], "signal-owned-out").await;
        let mut http = CanonicalHttpClient::open(http_config).await.unwrap();
        let original = http_settled(&mut http, &s.cid, "signal-owned-out").await;
        assert_eq!(original.state, OperationState::Completed);
        assert_eq!(
            original.result.as_ref().unwrap()["connection_id"],
            "signal-owned"
        );
        s.clients[1]
            .send_native_dtmf(
                capabilities::scope(&s.cid, "signal-owned-in", 2),
                "6",
                100,
                "rfc4733",
            )
            .await
            .unwrap();
        let replay = audio
            .send_dtmf("signal-owned-out", Some(1), "4#", 100, "info")
            .await
            .unwrap();
        assert_eq!(replay.result, original.result);
        let mut found = false;
        for _ in 0..128 {
            let frame = audio.receive_signaling().await.unwrap();
            if frame.kind() == "dtmf.received" {
                assert_eq!(
                    frame.scope(rvoip_uctp::v2::Scope::Operation),
                    Some("signal-owned-in")
                );
                assert_eq!(
                    frame.scope(rvoip_uctp::v2::Scope::Connection),
                    Some("signal-owned")
                );
                found = true;
                break;
            }
        }
        assert!(found);
        audio.shutdown().await.unwrap();
        drop(s.clients);
        s.f.server.shutdown().await.unwrap();
    }
}

#[derive(Clone, Copy)]
enum CreditFault {
    Deadline,
    RecipientRevoke,
    SourceRevoke,
    Cancel,
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_native_dtmf_partial_credit_write_records_unknown_without_replay_and_releases_control(
) {
    credit_fault(CreditFault::Deadline).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_native_dtmf_recipient_revoke_waits_for_partial_io_and_preserves_other_recipient(
) {
    credit_fault(CreditFault::RecipientRevoke).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_native_dtmf_source_revoke_waits_for_partial_io_and_never_revives_original_work()
{
    credit_fault(CreditFault::SourceRevoke).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_native_dtmf_cancel_after_partial_io_preserves_uncertainty_and_stops_remaining_work(
) {
    credit_fault(CreditFault::Cancel).await;
}
async fn credit_fault(mode: CreditFault) {
    use connect_store::{AuthorityMode, AuthorityScope};
    for algorithm in ["EdDSA", "ES256"] {
        let mut s = fixture(algorithm, true).await;
        let config = party_with_scope(
            &s.f,
            &s.f.peer.identity_id,
            108,
            vec!["audio".into()],
            signal_scope(),
        );
        // A signed DTMF frame exceeds this window. Finish the actual handshake
        // and barrier while reading, then stop reads so the first signal must
        // retain its physical sink across a real partial QUIC write.
        let mut peer = super::capability_quic::NativePeer::with_receive_window(&config, 256).await;
        peer.native(&s.cid).await;
        wait_native_operation(&s.f, &config.credential, &s.cid, "capability-slow-offer").await;
        peer.barrier().await;
        // Drain competing control/lifecycle frames, then stop within the exact
        // first DTMF frame. An unrelated queued writer must not be the fault.
        let mut pausing = Some(tokio::spawn(peer.pause_on_dtmf()));
        let mut peer = None;
        let mut http = CanonicalHttpClient::open(canonical_http_config(&s.f))
            .await
            .unwrap();
        let db = rusqlite::Connection::open(&s.f.config.database).unwrap();
        let mut uncertain = None;
        let mut injected = false;
        for index in 0..1 {
            let operation = format!("signal-credit-{index}");
            let scope =
                capabilities::scope(&s.cid, &operation, stream_revision(&s.f, "codec-target"));
            s.clients[1]
                .send_native_dtmf(
                    scope.clone(),
                    "1234567890ABCD*#1234567890abcd*#",
                    100,
                    "info",
                )
                .await
                .unwrap();
            let (paused, frame) = pausing.take().unwrap().await.unwrap();
            peer = Some(paused);
            let blocked_frame = frame;
            let scopes = [&s.f.client.identity_id, &config.identity_id]
                .into_iter()
                .map(|id| AuthorityScope::identity(id, AuthorityMode::Write).unwrap())
                .collect::<Vec<_>>();
            let observe_until = tokio::time::Instant::now() + Duration::from_secs(5);
            let mut stable_since = None;
            let io_held = if matches!(mode, CreditFault::Deadline) {
                false
            } else {
                loop {
                    let pending:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM native_dtmf_recipients r JOIN operations o ON o.record_id=r.operation_record_id JOIN native_admission_sources n ON n.operation_record_id=r.target_record_id WHERE o.operation_id=?1 AND n.connection_id='capability-slow' AND r.attempted=1 AND r.disposition IS NULL)",[&operation],|r|r.get(0)).unwrap();
                    let held=pending&&scopes.iter().all(|scope|matches!(s.f.server.store.try_acquire_authority(std::slice::from_ref(scope)),Err(e) if e.code=="authority-busy"));
                    if held {
                        let at = stable_since.get_or_insert_with(tokio::time::Instant::now);
                        if at.elapsed() >= Duration::from_millis(25) {
                            break true;
                        }
                    } else {
                        stable_since = None;
                    }
                    let state: String = db
                        .query_row(
                            "SELECT state FROM operations WHERE operation_id=?1",
                            [&operation],
                            |r| r.get(0),
                        )
                        .unwrap();
                    if matches!(
                        state.as_str(),
                        "completed" | "failed" | "cancelled" | "unknown"
                    ) {
                        break false;
                    }
                    assert!(
                        tokio::time::Instant::now() < observe_until,
                        "signal never reached a stable IO or outcome boundary"
                    );
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
            };
            if io_held && !matches!(mode, CreditFault::Deadline) {
                let source_scope =
                    AuthorityScope::identity(&s.f.client.identity_id, AuthorityMode::Write)
                        .unwrap();
                let target_scope =
                    AuthorityScope::identity(&config.identity_id, AuthorityMode::Write).unwrap();
                for scope in [&source_scope, &target_scope] {
                    assert!(
                        matches!(s.f.server.store.try_acquire_authority(std::slice::from_ref(scope)),Err(e) if e.code=="authority-busy"),
                        "actual partial IO lost current authority"
                    );
                }
                if matches!(mode, CreditFault::Cancel) {
                    let batch = http
                        .request_value(
                            "operation.cancel",
                            json!({"operation_id":operation}),
                            CommandScope {
                                conversation_id: Some(s.cid.clone()),
                                operation_id: Some(format!("cancel-{operation}")),
                                ..Default::default()
                            },
                        )
                        .await
                        .unwrap();
                    batch.ack().unwrap();
                } else {
                    let store = s.f.server.store.clone();
                    let admin = store.authenticate(&s.f.admin).unwrap();
                    let credential = store
                        .authenticate(if matches!(mode, CreditFault::SourceRevoke) {
                            &s.f.client.credential
                        } else {
                            &config.credential
                        })
                        .unwrap()
                        .credential_id;
                    let revoked = credential.clone();
                    let writer = tokio::task::spawn_blocking(move || {
                        store.revoke(&admin, &revoked).unwrap()
                    });
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    assert!(
                        !writer.is_finished(),
                        "revocation overtook the actual partial sink"
                    );
                    tokio::time::timeout(Duration::from_secs(3), writer)
                        .await
                        .unwrap()
                        .unwrap();
                    assert!(s.f.server.store.revalidate(&credential).is_err());
                    if matches!(mode, CreditFault::SourceRevoke) {
                        let mut hc = canonical_http_config(&s.f);
                        hc.credential =
                            s.f.server
                                .store
                                .issue(
                                    &s.f.client.identity_id,
                                    signal_scope(),
                                    chrono::Utc::now().timestamp() + 600,
                                )
                                .unwrap()
                                .token;
                        http = CanonicalHttpClient::open(hc).await.unwrap();
                    }
                }
                injected = true;
            }
            let result = http_settled(&mut http, &s.cid, &operation).await;
            let summary = if result.state == OperationState::Completed {
                result.result.as_ref().unwrap()
            } else {
                &result.problem.as_ref().unwrap()["signal"]
            };
            let normal = summary["recipients"]
                .as_array()
                .unwrap_or_else(|| {
                    panic!(
                        "missing local summary: {}",
                        serde_json::to_value(&result).unwrap()
                    )
                })
                .iter()
                .find(|r| r["connection_id"] == "codec-source")
                .unwrap();
            if normal["disposition"] == "local_submitted" {
                received(&mut s.clients[0], &operation).await;
            }
            if result.state == OperationState::Unknown {
                let slow = summary["recipients"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|r| r["connection_id"] == "capability-slow")
                    .unwrap();
                assert_eq!(slow["disposition"], "outcome_unknown");
                assert_eq!(slow["frame_id"].as_str(), Some(blocked_frame.as_str()));
                assert_eq!(summary["local_work_complete"], true);
                uncertain = Some((operation, scope, result));
                break;
            }
            assert_eq!(
                result.state,
                OperationState::Completed,
                "credit refusal must retain actual writer outcome: {}",
                serde_json::to_value(&result).unwrap()
            );
        }
        assert!(
            matches!(mode, CreditFault::Deadline) || injected,
            "fault did not reach actual pending IO"
        );
        let (operation, scope, original) =
            uncertain.expect("signed signal must exceed the fixed256-byte receive credit");
        let count:i64=db.query_row("SELECT count(*) FROM native_dtmf_recipients r JOIN operations o ON o.record_id=r.operation_record_id WHERE o.operation_id=?1",[&operation],|r|r.get(0)).unwrap();
        if !matches!(mode, CreditFault::SourceRevoke) {
            let replay = s.clients[1]
                .send_native_dtmf(scope, "1234567890ABCD*#1234567890abcd*#", 100, "info")
                .await
                .unwrap();
            assert_eq!(replay.state, OperationState::Unknown);
            assert_eq!(replay.problem, original.problem);
            s.clients[1]
                .send_native_dtmf(
                    capabilities::scope(
                        &s.cid,
                        "signal-after-stopped-unknown",
                        stream_revision(&s.f, "codec-target"),
                    ),
                    "5",
                    100,
                    "info",
                )
                .await
                .unwrap();
            let next = http_settled(&mut http, &s.cid, "signal-after-stopped-unknown").await;
            assert_eq!(
                next.state,
                OperationState::Completed,
                "fresh signal after retired partial writer: {}",
                serde_json::to_value(&next).unwrap()
            );
            assert_eq!(
                next.result.as_ref().unwrap()["recipients"]
                    .as_array()
                    .unwrap()
                    .len(),
                1
            );
            received(&mut s.clients[0], "signal-after-stopped-unknown").await;
        } else {
            assert!(s.clients[1]
                .send_native_dtmf(scope, "1234567890ABCD*#1234567890abcd*#", 100, "info")
                .await
                .is_err());
        }
        let after:i64=db.query_row("SELECT count(*) FROM native_dtmf_recipients r JOIN operations o ON o.record_id=r.operation_record_id WHERE o.operation_id=?1",[&operation],|r|r.get(0)).unwrap();
        assert_eq!(after, count);
        drop(db);
        drop(peer);
        drop(s.clients);
        s.f.server.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn production_native_dtmf_restart_keeps_unrecorded_actual_submission_unknown_and_never_redelivers(
) {
    for algorithm in ["EdDSA", "ES256"] {
        let mut s = fixture(algorithm, true).await;
        let db = rusqlite::Connection::open(&s.f.config.database).unwrap();
        db.execute_batch("CREATE TRIGGER stop_signal_fact BEFORE UPDATE OF disposition ON native_dtmf_recipients BEGIN SELECT RAISE(ABORT,'controlled restart cutoff'); END;").unwrap();
        let original_scope = capabilities::scope(&s.cid, "signal-restart", 2);
        s.clients[1]
            .send_native_dtmf(original_scope.clone(), "7#", 100, "info")
            .await
            .unwrap();
        let received_before = received(&mut s.clients[0], "signal-restart").await;
        let before = record(&s, "signal-restart");
        assert_eq!(
            before.state,
            connect_protocol::transitions::OperationState::Preparing
        );
        let codecs::Running {
            mut f,
            clients,
            cid,
            ..
        } = s;
        drop(clients);
        f.server.shutdown().await.unwrap();
        // The old owned callback has stopped. Only the successor may recover
        // the durable attempted-frame reservation; no original fact survived.
        db.execute_batch("DROP TRIGGER stop_signal_fact").unwrap();
        f.server = RunningCanonicalServer::start(f.config.clone())
            .await
            .unwrap();
        f.client.endpoint = f.server.address;
        let mut http = CanonicalHttpClient::open(canonical_http_config(&f))
            .await
            .unwrap();
        let recovered = http_settled(&mut http, &cid, "signal-restart").await;
        assert_eq!(recovered.state, OperationState::Unknown);
        assert_eq!(
            recovered.record_id.as_ref().map(|v| v.as_str()),
            Some(before.record_id.as_str())
        );
        let summary = &recovered.problem.as_ref().unwrap()["signal"];
        assert_eq!(summary["recipients"][0]["disposition"], "outcome_unknown");
        assert_eq!(summary["recipients"][0]["frame_id"], received_before.id());
        let mut other_physical = CanonicalClient::open(f.client.clone()).await.unwrap();
        let replay = other_physical
            .send_native_dtmf(original_scope, "7#", 100, "info")
            .await
            .unwrap();
        assert_eq!(replay.state, OperationState::Unknown);
        assert_eq!(replay.problem, recovered.problem);
        drop(other_physical);
        let config = party_with_scope(
            &f,
            &f.peer.identity_id,
            109,
            vec!["audio".into()],
            signal_scope(),
        );
        let mut target = owned(
            config,
            &cid,
            "signal-restarted-recipient",
            "signal-restarted-recipient-offer",
        )
        .await;
        let mut source = owned(
            f.client.clone(),
            &cid,
            "signal-restarted-source",
            "signal-restarted-source-offer",
        )
        .await;
        source
            .send_dtmf("signal-after-restart", Some(1), "2#", 100, "info")
            .await
            .unwrap();
        let mut found = false;
        for _ in 0..128 {
            let frame = target.receive_signaling().await.unwrap();
            assert_ne!(
                frame.scope(rvoip_uctp::v2::Scope::Operation),
                Some("signal-restart"),
                "successor redelivered original signal"
            );
            if frame.kind() == "dtmf.received" {
                assert_eq!(
                    frame.scope(rvoip_uctp::v2::Scope::Operation),
                    Some("signal-after-restart")
                );
                found = true;
                break;
            }
        }
        assert!(found);
        assert_eq!(
            http_settled(&mut http, &cid, "signal-after-restart")
                .await
                .state,
            OperationState::Completed
        );
        let still = http_settled(&mut http, &cid, "signal-restart").await;
        assert_eq!(still.state, OperationState::Unknown);
        assert_eq!(still.problem, recovered.problem);
        source.shutdown().await.unwrap();
        target.shutdown().await.unwrap();
        drop(db);
        f.server.shutdown().await.unwrap();
    }
}

async fn barrier_without_signal(client: &mut CanonicalClient) {
    let id = client.keepalive().await.unwrap();
    for _ in 0..128 {
        let frame = client.receive().await.unwrap();
        if lifecycle::session_start(&frame) {
            continue;
        }
        assert_ne!(
            frame.kind(),
            "dtmf.received",
            "signal escaped the actual private Connection audience"
        );
        if frame.kind() == "ack" {
            assert_eq!(frame.original()["in_reply_to"], id);
            return;
        }
        assert!(matches!(
            frame.kind(),
            "connection.ready" | "stream.opened" | "stream.closed" | "connection.quality"
        ));
    }
    panic!("missing private signal barrier")
}
pub(super) async fn private(
    f: &Fixture,
    probes: &mut [CanonicalClient],
    cid: &str,
) -> (CommandScope, Value) {
    // Private consultant, primary, retained remote, public consultant. The two
    // consultant connections share the same Participant and named signal grant.
    let original = capabilities::scope(
        cid,
        "signal-private-primary",
        stream_revision(f, "codec-target"),
    );
    probes[1]
        .send_native_dtmf(original.clone(), "91#", 100, "info")
        .await
        .unwrap();
    let first = received(&mut probes[0], "signal-private-primary").await;
    let mut http = CanonicalHttpClient::open(canonical_http_config(f))
        .await
        .unwrap();
    let outcome = http_settled(&mut http, cid, "signal-private-primary").await;
    assert_eq!(outcome.state, OperationState::Completed);
    let summary = outcome.result.unwrap();
    assert_eq!(summary["recipients"].as_array().unwrap().len(), 1);
    assert_eq!(
        summary["recipients"][0]["connection_id"],
        "private-consultant"
    );
    assert_eq!(summary["recipients"][0]["frame_id"], first.id());
    let mut back = capabilities::scope(
        cid,
        "signal-private-consultant",
        stream_revision(f, "private-consultant"),
    );
    back.connection_id = Some("private-consultant".into());
    probes[0]
        .send_native_dtmf(back, "82#", 100, "info")
        .await
        .unwrap();
    received(&mut probes[1], "signal-private-consultant").await;
    for _ in 0..3 {
        barrier_without_signal(&mut probes[2]).await;
        barrier_without_signal(&mut probes[3]).await;
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    (original, summary)
}
pub(super) async fn returned(
    f: &Fixture,
    probes: &mut [CanonicalClient],
    cid: &str,
    original: (CommandScope, Value),
) {
    // Retained remote, primary, retired private consultant, retired public peer.
    let replay = probes[1]
        .send_native_dtmf(original.0, "91#", 100, "info")
        .await
        .unwrap();
    assert_eq!(replay.state, OperationState::Completed);
    assert_eq!(replay.result, Some(original.1));
    barrier_without_signal(&mut probes[0]).await;
    probes[1]
        .send_native_dtmf(
            capabilities::scope(
                cid,
                "signal-returned-primary",
                stream_revision(f, "codec-target"),
            ),
            "63#",
            100,
            "info",
        )
        .await
        .unwrap();
    let frame = received(&mut probes[0], "signal-returned-primary").await;
    let mut http = CanonicalHttpClient::open(canonical_http_config(f))
        .await
        .unwrap();
    let result = http_settled(&mut http, cid, "signal-returned-primary").await;
    assert_eq!(result.state, OperationState::Completed);
    let summary = result.result.unwrap();
    assert_eq!(summary["recipients"].as_array().unwrap().len(), 1);
    assert_eq!(summary["recipients"][0]["connection_id"], "codec-source");
    assert_eq!(summary["recipients"][0]["frame_id"], frame.id());
}

fn bind_owned_successor(
    s: &codecs::Running,
    config: &CanonicalClientConfig,
    credential: &str,
    operation: &str,
) {
    bind_fixture_successor(&s.f, config, credential, operation)
}
pub(super) fn bind_fixture_successor(
    f: &Fixture,
    config: &CanonicalClientConfig,
    credential: &str,
    operation: &str,
) {
    let actor = f.server.store.authenticate(&config.credential).unwrap();
    let bound = f
        .server
        .store
        .resolve_peer_signing_key(
            &actor.credential_id,
            "tnt_default",
            &config.signing_key_id,
            &config.signing_algorithm,
        )
        .unwrap();
    let db = rusqlite::Connection::open(&f.config.database).unwrap();
    let revision: u64 = db
        .query_row(
            "SELECT revision FROM peer_signing_keys WHERE key_id=?1",
            [&config.signing_key_id],
            |r| r.get(0),
        )
        .unwrap();
    drop(db);
    let admin = f.server.store.authenticate(&f.admin).unwrap();
    f.server
        .store
        .execute_provisioning(
            &admin.credential_id,
            ProvisioningCommand::SigningKey {
                operation_id: operation.into(),
                identity_id: actor.identity_id.clone(),
                credential_id: credential.into(),
                device_id: config.device_id.clone(),
                key_id: config.signing_key_id.clone(),
                algorithm: if config.signing_algorithm == "ES256" {
                    SigningAlgorithm::ES256
                } else {
                    SigningAlgorithm::EdDSA
                },
                public_key: URL_SAFE_NO_PAD.encode(bound.public_key),
                not_before: bound.not_before,
                expires_at: bound.expires_at,
                expected_revision: Some(revision),
            },
        )
        .unwrap();
}

#[tokio::test]
async fn production_owned_audio_refresh_keeps_signed_reader_and_current_signal_authority() {
    for algorithm in ["EdDSA", "ES256"] {
        let mut s = fixture(algorithm, true).await;
        let config = party_with_scope(
            &s.f,
            &s.f.peer.identity_id,
            109,
            vec!["audio".into()],
            signal_scope(),
        );
        let actor = s.f.server.store.authenticate(&config.credential).unwrap();
        let mut audio = owned(
            config.clone(),
            &s.cid,
            "signal-refresh-owned",
            "signal-refresh-owned-offer",
        )
        .await;
        assert!(audio
            .refresh_credential("unissued-owned-successor".into())
            .await
            .is_err());
        let issued =
            s.f.server
                .store
                .issue(&actor.identity_id, actor.scope.clone(), actor.expires_at)
                .unwrap();
        bind_owned_successor(&s, &config, &issued.id, "bind-owned-refresh-successor");
        let next = audio
            .refresh_credential(issued.token.clone())
            .await
            .unwrap();
        assert_eq!(next.identity_id, actor.identity_id);
        audio
            .send_dtmf("signal-after-refresh", Some(1), "9#", 100, "info")
            .await
            .unwrap();
        received(&mut s.clients[0], "signal-after-refresh").await;
        received(&mut s.clients[1], "signal-after-refresh").await;
        // The recipient registration moves to the verified successor dispatch;
        // old queued deliveries keep their original credentials/incarnation.
        s.clients[1]
            .send_native_dtmf(
                capabilities::scope(&s.cid, "signal-to-refreshed-owner", 2),
                "4",
                100,
                "info",
            )
            .await
            .unwrap();
        received(&mut s.clients[0], "signal-to-refreshed-owner").await;
        let mut delivered = false;
        for _ in 0..128 {
            let frame = audio.receive_signaling().await.unwrap();
            if frame.kind() == "dtmf.received" {
                assert_eq!(
                    frame.scope(rvoip_uctp::v2::Scope::Operation),
                    Some("signal-to-refreshed-owner")
                );
                delivered = true;
                break;
            }
        }
        assert!(delivered, "refreshed recipient did not receive its signal");
        // Reusing that same credential changes only dispatch metadata. It must
        // neither retire the media origin nor increment the Core epoch again.
        audio.refresh_credential(issued.token).await.unwrap();
        audio
            .send_dtmf("signal-after-repeat-refresh", Some(1), "5", 100, "info")
            .await
            .unwrap();
        received(&mut s.clients[0], "signal-after-repeat-refresh").await;
        received(&mut s.clients[1], "signal-after-repeat-refresh").await;
        let sql = rusqlite::Connection::open(&s.f.config.database).unwrap();
        let observed:(String,u64,u64,String,String,u64)=sql.query_row("SELECT b.state,b.epoch,b.core_credential_revision,b.previous_credential_id,b.credential_id,b.stream_revision FROM native_media_credential_bindings b JOIN operations o ON o.record_id=b.parent_record_id WHERE o.connection_id='signal-refresh-owned'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).unwrap();
        assert_eq!(
            observed,
            ("applied".into(), 1, 2, actor.credential_id, issued.id, 1)
        );
        let epochs: u64 = sql
            .query_row(
                "SELECT count(*) FROM native_media_credential_bindings",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(epochs, 1);
        drop(sql);
        audio.shutdown().await.unwrap();
        drop(s.clients);
        s.f.server.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn production_owned_audio_signaling_refresh_survives_original_credential_expiry() {
    for algorithm in ["EdDSA", "ES256"] {
        let mut s = fixture(algorithm, true).await;
        let mut config = party_with_scope(
            &s.f,
            &s.f.peer.identity_id,
            110,
            vec!["audio".into()],
            signal_scope(),
        );
        let actor = s.f.server.store.authenticate(&config.credential).unwrap();
        let short =
            s.f.server
                .store
                .issue(
                    &actor.identity_id,
                    actor.scope.clone(),
                    chrono::Utc::now().timestamp() + 7,
                )
                .unwrap();
        bind_owned_successor(&s, &config, &short.id, "bind-short-owned-signaling");
        config.credential = short.token;
        let expiry =
            s.f.server
                .store
                .credential_metadata(&short.id)
                .unwrap()
                .actor
                .expires_at;
        let mut audio = owned(
            config.clone(),
            &s.cid,
            "signal-expiry-owned",
            "signal-expiry-owned-offer",
        )
        .await;
        let next =
            s.f.server
                .store
                .issue(
                    &actor.identity_id,
                    actor.scope.clone(),
                    chrono::Utc::now().timestamp() + 300,
                )
                .unwrap();
        bind_owned_successor(&s, &config, &next.id, "bind-long-owned-signaling");
        let authentication = audio.refresh_credential(next.token.clone()).await.unwrap();
        assert!(authentication.expires_at.timestamp() > expiry);
        let after = chrono::DateTime::from_timestamp(expiry, 0).unwrap()
            + chrono::Duration::milliseconds(200);
        if let Ok(wait) = (after - chrono::Utc::now()).to_std() {
            tokio::time::sleep(wait).await;
        }
        assert!(s.f.server.store.authenticate(&config.credential).is_err());
        audio
            .send_dtmf("signal-after-predecessor-expiry", Some(1), "7", 100, "info")
            .await
            .unwrap();
        received(&mut s.clients[0], "signal-after-predecessor-expiry").await;
        received(&mut s.clients[1], "signal-after-predecessor-expiry").await;
        // The same owned reader and already-rotated media remain usable after
        // another refresh with the unchanged successor credential.
        let again = audio.refresh_credential(next.token).await.unwrap();
        assert_eq!(again.identity_id, actor.identity_id);
        assert!(again.expires_at.timestamp() > expiry);
        audio.shutdown().await.unwrap();
        drop(s.clients);
        s.f.server.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn production_native_refresh_retains_actual_decoded_rtp_after_original_expiry() {
    for algorithm in ["EdDSA", "ES256"] {
        let mut f = Fixture::with_native_dtmf_scope(algorithm, 12).await;
        let actor = f.server.store.authenticate(&f.client.credential).unwrap();
        let short = f
            .server
            .store
            .issue(
                &actor.identity_id,
                actor.scope.clone(),
                chrono::Utc::now().timestamp() + 7,
            )
            .unwrap();
        bind_fixture_successor(&f, &f.client, &short.id, "bind-short-rtp-refresh");
        let expiry = f
            .server
            .store
            .credential_metadata(&short.id)
            .unwrap()
            .actor
            .expires_at;
        f.client.credential = short.token.clone();
        let mut s = codecs::setup_started_dtmf(f, true).await;
        let next =
            s.f.server
                .store
                .issue(
                    &actor.identity_id,
                    actor.scope.clone(),
                    chrono::Utc::now().timestamp() + 300,
                )
                .unwrap();
        bind_fixture_successor(&s.f, &s.f.client, &next.id, "bind-long-rtp-refresh");
        let facts = || {
            rusqlite::Connection::open(&s.f.config.database).unwrap().query_row(
                "SELECT generation,endpoint_id,participant_id,membership_epoch,json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id='codec-target'",[],
                |r|Ok((r.get::<_,u64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,u64>(3)?,r.get::<_,u64>(4)?))
            ).unwrap()
        };
        let before = facts();
        assert_eq!(before.4, 2);
        {
            let (source, target) = s.clients.split_at_mut(1);
            codecs::decoded_audio(&source[0], &mut target[0], s.sh, 0, s.th, 0, 120_000).await;
        }
        let authentication = s.clients[1].refresh_credential(next.token).await.unwrap();
        assert_eq!(authentication.identity_id, actor.identity_id);
        let after = chrono::DateTime::from_timestamp(expiry, 0).unwrap()
            + chrono::Duration::milliseconds(200);
        if let Ok(wait) = (after - chrono::Utc::now()).to_std() {
            tokio::time::sleep(wait).await;
        }
        assert!(s.f.server.store.authenticate(&short.token).is_err());
        // Remove unread historical packets. The second independent FFmpeg
        // decode must observe newly transmitted audio through the same handles.
        for _ in 0..256 {
            if tokio::time::timeout(
                Duration::from_millis(5),
                s.clients[1].receive_native_media_datagram(),
            )
            .await
            .is_err()
            {
                break;
            }
        }
        {
            let (source, target) = s.clients.split_at_mut(1);
            codecs::decoded_audio(&source[0], &mut target[0], s.sh, 0, s.th, 0, 240_000).await;
        }
        assert_eq!(
            facts(),
            before,
            "credential refresh changed the logical media model"
        );
        let sql = rusqlite::Connection::open(&s.f.config.database).unwrap();
        let observed:(String,u64,u64,u64)=sql.query_row("SELECT state,epoch,core_credential_revision,stream_revision FROM native_media_credential_bindings",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(observed, ("applied".into(), 1, 2, 2));
        drop(sql);
        drop(s.clients);
        s.f.server.shutdown().await.unwrap();
    }
}

#[cfg(feature = "test-support")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::await_holding_lock)] // Actual Core metadata remains busy through admission.
async fn production_native_dtmf_metadata_deadline_and_cancel_refuse_before_io_and_replay_no_effect()
{
    for algorithm in ["EdDSA", "ES256"] {
        for mode in ["deadline", "cancel"] {
            let mut s = fixture(algorithm, true).await;
            let session =
                s.f.server
                    .service
                    .canonical_session_metadata_for_test("native-session")
                    .unwrap();
            let metadata = session.write().unwrap();
            let scope = capabilities::scope(&s.cid, "signal-metadata-stop", 2);
            s.clients[1]
                .send_native_dtmf(scope.clone(), "7", 100, "info")
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(100)).await;
            let pending = record(&s, "signal-metadata-stop");
            if pending.state != connect_protocol::transitions::OperationState::Preparing {
                drop(metadata);
                panic!(
                    "metadata wait ended before bounded stop: {}",
                    serde_json::to_value(&pending).unwrap()
                );
            }
            let mut http = CanonicalHttpClient::open(canonical_http_config(&s.f))
                .await
                .unwrap();
            let result = if mode == "cancel" {
                http.request_value(
                    "operation.cancel",
                    json!({"operation_id":"signal-metadata-stop"}),
                    CommandScope {
                        conversation_id: Some(s.cid.clone()),
                        operation_id: Some("cancel-metadata-stop".into()),
                        ..Default::default()
                    },
                )
                .await
                .unwrap()
                .ack()
                .unwrap();
                drop(metadata);
                http_settled(&mut http, &s.cid, "signal-metadata-stop").await
            } else {
                let result = http_settled(&mut http, &s.cid, "signal-metadata-stop").await;
                drop(metadata);
                result
            };
            assert_eq!(
                result.state,
                if mode == "cancel" {
                    OperationState::Cancelled
                } else {
                    OperationState::Failed
                }
            );
            let recipients = result.problem.as_ref().unwrap()["signal"]["recipients"]
                .as_array()
                .unwrap();
            assert_eq!(recipients.len(), 1);
            assert_eq!(recipients[0]["disposition"], "not_started");
            assert!(recipients[0]["frame_id"].is_null());
            let replay = s.clients[1]
                .send_native_dtmf(scope, "7", 100, "info")
                .await
                .unwrap();
            assert_eq!(replay.state, result.state);
            assert_eq!(replay.problem, result.problem);
            s.clients[1]
                .send_native_dtmf(
                    capabilities::scope(&s.cid, "signal-metadata-recovery", 2),
                    "6",
                    100,
                    "info",
                )
                .await
                .unwrap();
            received(&mut s.clients[0], "signal-metadata-recovery").await;
            complete(&s, "signal-metadata-recovery").await;
            drop(s.clients);
            s.f.server.shutdown().await.unwrap();
        }
    }
}
