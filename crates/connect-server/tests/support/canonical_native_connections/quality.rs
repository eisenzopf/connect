//! Actual native cadence, original operation replay and captured-result recovery.
use super::*;
use rvoip_uctp::v2::payloads::OperationState;
fn scope(cid: &str, operation: &str, revision: u64) -> CommandScope {
    CommandScope {
        conversation_id: Some(cid.into()),
        session_id: Some("native-session".into()),
        connection_id: Some("codec-target".into()),
        operation_id: Some(operation.into()),
        expected_revision: Some(revision),
    }
}
fn readiness(f: &Fixture) -> Value {
    let text: String = rusqlite::Connection::open(&f.config.database)
        .unwrap()
        .query_row(
            "SELECT readiness_json FROM connections WHERE id='codec-target'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    serde_json::from_str(&text).unwrap()
}
async fn sample(
    client: &mut CanonicalClient,
    cid: &str,
    since: chrono::DateTime<chrono::Utc>,
    interval_ms: u64,
    algorithm: &str,
) -> Envelope {
    let until = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        assert!(
            tokio::time::Instant::now() < until,
            "fresh configured quality sample"
        );
        let frame = client.receive().await.unwrap();
        assert_eq!(frame.original()["cid"], cid);
        assert_eq!(frame.original()["sid"], "native-session");
        if lifecycle::session_start(&frame) {
            continue;
        }
        assert_eq!(frame.original()["connid"], "codec-target");
        assert!(matches!(
            frame.kind(),
            "connection.ready" | "stream.opened" | "stream.closed" | "connection.quality"
        ));
        lifecycle::observe(algorithm, &frame);
        if frame.kind() != "connection.quality" {
            continue;
        }
        let at =
            chrono::DateTime::parse_from_rfc3339(frame.original()["ts"].as_str().unwrap()).unwrap();
        if at < since {
            continue;
        } // Historical bytes already admitted before the update.
        let actual = frame.original()["payload"]["interval_ms"].as_u64().unwrap();
        assert!(
            (interval_ms..interval_ms + 1500).contains(&actual),
            "configured {interval_ms}, measured {actual}"
        );
        return frame;
    }
}
#[tokio::test]
async fn production_native_quality_cadence_replay_preserves_newer_configuration_and_actual_bindings(
) {
    for algorithm in ["EdDSA", "ES256"] {
        let mut f = Fixture::with_native_end_scope(algorithm, 12).await;
        f.client.timeout_ms = 8000;
        let (mut s, ()) =
            codecs::setup_started(f, (), "quality-target-offer", "quality-source-offer").await;
        let before = readiness(&s.f);
        let fast = scope(&s.cid, "quality-fast", 2);
        let first = s.clients[1]
            .configure_native_quality(fast.clone(), 700)
            .await
            .unwrap();
        assert_eq!(first.state, OperationState::Completed);
        assert_eq!(first.result.as_ref().unwrap()["connection_generation"], 1);
        let after = chrono::Utc::now();
        assert_audio_reaches_other_participant(&mut s.clients, s.sh, &[s.th], 80000).await;
        let report = sample(&mut s.clients[1], &s.cid, after, 700, algorithm).await;
        assert!(
            report.original()["payload"]["streams"][0]["packets_sent"]
                .as_u64()
                .unwrap()
                > 0
        );
        let slow = scope(&s.cid, "quality-slow", 2);
        assert_eq!(
            s.clients[1]
                .configure_native_quality(slow, 1800)
                .await
                .unwrap()
                .state,
            OperationState::Completed
        );
        let after = chrono::Utc::now();
        let replay = s.clients[1]
            .configure_native_quality(fast.clone(), 700)
            .await
            .unwrap();
        assert_eq!(replay.result, first.result);
        assert_eq!(readiness(&s.f)["quality_interval_ms"], 1800);
        sample(&mut s.clients[1], &s.cid, after, 1800, algorithm).await;
        let mut ready = readiness(&s.f);
        ready.as_object_mut().unwrap().remove("quality_interval_ms");
        assert_eq!(ready, before);
        // Same logical ID with a changed interval conflicts and cannot retune.
        let changed = s.clients[1].configure_native_quality(fast, 900).await;
        assert!(matches!(changed,Err(ClientError::Application(e)) if e.code=="conflict"));
        // A different physical peer cannot act on the original live slot.
        let mut foreign = CanonicalClient::open(s.f.client.clone()).await.unwrap();
        let denied = foreign
            .configure_native_quality(scope(&s.cid, "quality-foreign", 2), 500)
            .await;
        assert!(matches!(denied,Err(ClientError::Application(e)) if e.code=="forbidden"));
        let foreign_operations: u64 = rusqlite::Connection::open(&s.f.config.database)
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM operations WHERE operation_id='quality-foreign'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(foreign_operations, 0);
        assert_eq!(readiness(&s.f)["quality_interval_ms"], 1800);
        foreign.shutdown().await.unwrap();
        let stale = s.clients[1]
            .configure_native_quality(scope(&s.cid, "quality-stale", 1), 500)
            .await
            .unwrap();
        assert_eq!(stale.state, OperationState::Failed);
        assert_eq!(readiness(&s.f)["quality_interval_ms"], 1800);
        // HTTP never obtains this signed physical-peer control capability.
        let mut http = CanonicalHttpClient::open(canonical_http_config(&s.f))
            .await
            .unwrap();
        let batch = http
            .request_value(
                "connection.update",
                json!({"action":"quality","quality_interval_ms":700}),
                scope(&s.cid, "quality-http", 2),
            )
            .await;
        let error = match batch {
            Ok(batch) => batch.ack().unwrap_err(),
            Err(error) => error,
        };
        assert!(
            matches!(error,ClientError::Application(e) if matches!(e.code.as_str(),"unsupported-media-profile"|"unsupported-command"))
        );
        s.clients.swap(0, 1);
        assert_audio_reaches_other_participant(&mut s.clients, s.th, &[s.sh], 81000).await;
        for client in s.clients {
            client.shutdown().await.unwrap();
        }
        s.f.server.shutdown().await.unwrap();
    }
}
#[tokio::test]
async fn production_native_quality_requires_current_named_permission_without_parent_or_owner_fallback(
) {
    let f = Fixture::with_native_end_scope("EdDSA", 8).await;
    ready_endpoint(&f).await;
    let mut limited = Scope::collaboration(vec!["*".into()]);
    limited.operations.retain(|p| p != "connection.update");
    let config = party_with_scope(
        &f,
        &f.client.identity_id,
        107,
        vec!["audio".into()],
        limited,
    );
    let mut owner = CanonicalClient::open(f.client.clone()).await.unwrap();
    let cid = session(&f, &mut owner).await;
    let mut peer = CanonicalClient::open(config.clone()).await.unwrap();
    negotiate(
        &mut peer,
        &cid,
        "quality-limited-offer",
        "quality-limited-slot",
    )
    .await;
    wait_native_operation(&f, &config.credential, &cid, "quality-limited-offer").await;
    let command = CommandScope {
        conversation_id: Some(cid),
        session_id: Some("native-session".into()),
        connection_id: Some("quality-limited-slot".into()),
        operation_id: Some("quality-limited".into()),
        expected_revision: Some(1),
    };
    assert!(
        matches!(peer.configure_native_quality(command,700).await,Err(ClientError::Application(e)) if e.code=="forbidden")
    );
    assert!(f.server.store.authenticate(&config.credential).is_ok());
    let count: u64 = rusqlite::Connection::open(&f.config.database)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM operations WHERE operation_id='quality-limited'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    let id = peer.keepalive().await.unwrap();
    reply(&mut peer, &id, "ack").await;
    peer.shutdown().await.unwrap();
    owner.shutdown().await.unwrap();
    f.server.shutdown().await.unwrap();
}
#[tokio::test]
async fn production_native_quality_captured_result_recovers_journal_failure_without_retaining_source_readers(
) {
    let mut s = codecs::setup().await;
    let c = rusqlite::Connection::open(&s.f.config.database).unwrap();
    c.execute_batch("CREATE TRIGGER controlled_quality_record_failure BEFORE INSERT ON journal WHEN json_extract(NEW.data_json,'$.operation_id')='quality-recover' AND json_extract(NEW.data_json,'$.state')='completed' BEGIN SELECT RAISE(ABORT,'controlled quality recording failure'); END;").unwrap();
    let command = scope(&s.cid, "quality-recover", 2);
    let pending = s.clients[1]
        .configure_native_quality(command.clone(), 800)
        .await
        .unwrap();
    assert_eq!(pending.state, OperationState::Preparing);
    let after = chrono::Utc::now();
    assert_audio_reaches_other_participant(&mut s.clients, s.sh, &[s.th], 82000).await;
    sample(&mut s.clients[1], &s.cid, after, 800, "EdDSA").await;
    assert!(readiness(&s.f).get("quality_interval_ms").is_none());
    let store = s.f.server.store.clone();
    let identity = s.f.client.identity_id.clone();
    tokio::task::spawn_blocking(move || {
        store
            .acquire_authority(
                &[connect_store::AuthorityScope::identity(
                    &identity,
                    connect_store::AuthorityMode::Write,
                )
                .unwrap()],
                &connect_store::AuthorityWait::new(Duration::from_secs(2)),
            )
            .map(drop)
    })
    .await
    .unwrap()
    .unwrap();
    c.execute_batch("DROP TRIGGER controlled_quality_record_failure")
        .unwrap();
    drop(c);
    let until = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let value = s.clients[1]
            .operation_get(
                "quality-recover",
                CommandScope {
                    conversation_id: Some(s.cid.clone()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        if value.state == OperationState::Completed {
            break;
        }
        assert!(tokio::time::Instant::now() < until);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        s.clients[1]
            .configure_native_quality(command, 800)
            .await
            .unwrap()
            .state,
        OperationState::Completed
    );
    let count:u64=rusqlite::Connection::open(&s.f.config.database).unwrap().query_row("SELECT COUNT(*) FROM operation_effects e JOIN operations o ON o.record_id=e.operation_record_id WHERE o.operation_id='quality-recover'",[],|r|r.get(0)).unwrap();
    assert_eq!(count, 1);
    assert_eq!(readiness(&s.f)["quality_interval_ms"], 800);
    assert_eq!(readiness(&s.f)["stream_revision"], 2);
    for client in s.clients {
        client.shutdown().await.unwrap();
    }
    s.f.server.shutdown().await.unwrap();
}
