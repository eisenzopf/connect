//! Actual signed receipt of peer descriptors; listener fanout is separate.
use super::*;
use rvoip_uctp::v2::payloads::OperationState;
pub(super) fn scope(cid: &str, operation: &str, revision: u64) -> CommandScope {
    CommandScope {
        conversation_id: Some(cid.into()),
        session_id: Some("native-session".into()),
        connection_id: Some("codec-target".into()),
        operation_id: Some(operation.into()),
        expected_revision: Some(revision),
    }
}
pub(super) fn participant(f: &Fixture, connection: &str) -> String {
    rusqlite::Connection::open(&f.config.database).unwrap().query_row("SELECT n.participant_id FROM native_admission_sources n JOIN operations o ON o.record_id=n.operation_record_id WHERE o.connection_id=?1 AND o.command_type='connection.offer' AND o.state='completed'",[connection],|r|r.get(0)).unwrap()
}
fn readiness(f: &Fixture) -> Value {
    let raw: String = rusqlite::Connection::open(&f.config.database)
        .unwrap()
        .query_row(
            "SELECT readiness_json FROM connections WHERE id='codec-target'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    serde_json::from_str(&raw).unwrap()
}
pub(super) async fn next_advertisement(client: &mut CanonicalClient, algorithm: &str) -> Envelope {
    for _ in 0..128 {
        let frame = client.receive().await.unwrap();
        if frame.kind() == "capability.advertise" {
            lifecycle::observe(algorithm, &frame);
            return frame;
        }
        assert!(
            matches!(
                frame.kind(),
                "connection.ready" | "stream.opened" | "stream.closed" | "connection.quality"
            ),
            "unexpected frame: {:?}",
            frame.original()
        );
    }
    panic!("advertisement missing behind bounded lifecycle hints");
}
pub(super) async fn barrier_without_advertisement(client: &mut CanonicalClient) {
    let id = client.keepalive().await.unwrap();
    loop {
        let frame = client.receive().await.unwrap();
        if frame.kind() == "ack" {
            assert_eq!(frame.original()["in_reply_to"], id);
            return;
        }
        assert!(
            matches!(
                frame.kind(),
                "connection.ready" | "stream.opened" | "stream.closed" | "connection.quality"
            ),
            "unexpected publication: {:?}",
            frame.original()
        );
    }
}
#[tokio::test]
async fn production_native_capability_delivery_uses_the_recipient_physical_writer_and_latest_source_descriptor_without_republishing_replay(
) {
    for algorithm in ["EdDSA", "ES256"] {
        let mut f = Fixture::with_native_end_scope(algorithm, 12).await;
        f.client.timeout_ms = 8000;
        let (mut scene, ()) =
            codecs::setup_started(f, (), "delivery-target-offer", "delivery-source-offer").await;
        let pid = participant(&scene.f, "codec-target");
        let first = json!({"audio_codecs":[{"name":"pcmu","params":{}}],"future":{"version":1,"exact":[true,17]}});
        let latest = json!({"audio_codecs":[],"future":{"version":2,"exact":[false,29]}});
        let original = scope(&scene.cid, "delivery-first", 2);
        let receipt = scene.clients[1]
            .advertise_native_capabilities(original.clone(), &pid, first.clone(), "endpoint-change")
            .await
            .unwrap();
        assert_eq!(receipt.state, OperationState::Completed);
        assert_eq!(
            scene.clients[1]
                .advertise_native_capabilities(
                    scope(&scene.cid, "delivery-latest", 2),
                    &pid,
                    latest.clone(),
                    "network-adapt"
                )
                .await
                .unwrap()
                .state,
            OperationState::Completed
        );
        let own_operation = scene.clients[0]
            .operation_get(
                "delivery-source-offer",
                CommandScope {
                    conversation_id: Some(scene.cid.clone()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(own_operation.state, OperationState::Completed);
        let mut previous = 0;
        loop {
            // This peer owns codec-source. The signed fact names codec-target;
            // source wire scope must not be mistaken for recipient routing.
            let frame = next_advertisement(&mut scene.clients[0], algorithm).await;
            assert_eq!(frame.original()["cid"], scene.cid);
            assert_eq!(frame.original()["sid"], "native-session");
            assert_eq!(frame.original()["connid"], "codec-target");
            assert!(frame.original().get("in_reply_to").is_none());
            assert!(frame.original().get("operation_id").is_none());
            assert_eq!(frame.original()["payload"]["by_participant"], pid);
            let version = frame.original()["payload"]["capabilities"]["future"]["version"]
                .as_u64()
                .unwrap();
            assert!(version >= previous);
            previous = version;
            if version == 2 {
                assert_eq!(frame.original()["payload"]["capabilities"], latest);
                break;
            }
            assert_eq!(version, 1);
            assert_eq!(frame.original()["payload"]["capabilities"], first);
        }
        assert_eq!(
            scene.clients[1]
                .advertise_native_capabilities(original, &pid, first, "endpoint-change")
                .await
                .unwrap()
                .result,
            receipt.result
        );
        for _ in 0..3 {
            tokio::time::sleep(Duration::from_millis(80)).await;
            barrier_without_advertisement(&mut scene.clients[0]).await;
        }
        assert_eq!(
            readiness(&scene.f)["advertised_capabilities"]["capabilities"],
            latest
        );
        assert_audio_reaches_other_participant(&mut scene.clients, scene.sh, &[scene.th], 91000)
            .await;
        scene.clients.swap(0, 1);
        assert_audio_reaches_other_participant(&mut scene.clients, scene.th, &[scene.sh], 92000)
            .await;
        for client in scene.clients {
            client.shutdown().await.unwrap();
        }
        scene.f.server.shutdown().await.unwrap();
    }
}
#[tokio::test]
async fn production_native_capability_gain_negotiates_new_codec_and_withdrawal_preserves_existing_audio_and_selectors(
) {
    for algorithm in ["EdDSA", "ES256"] {
        let mut f = Fixture::with_native_end_scope(algorithm, 12).await;
        f.client.timeout_ms = 8000;
        let pcmu = json!({"audio_codecs":[{"name":"pcmu","params":{"sample_rate":8000,"channels":1}}],"max_streams_per_connection":8,"identity_assurance_offered":"identified"});
        let pcma = json!({"audio_codecs":[{"name":"pcma","params":{"sample_rate":8000,"channels":1}}],"max_streams_per_connection":8,"identity_assurance_offered":"identified"});
        let (mut s, ()) = codecs::setup_started_capabilities(
            f,
            (),
            "caps-gain-target-offer",
            "caps-gain-source-offer",
            pcmu.clone(),
        )
        .await;
        let pid = participant(&s.f, "codec-target");
        let change = json!({"action":"codec-renegotiate","streams":["codec-monitor"],"codec_preferences":["pcma"]});
        let unavailable = s.clients[1]
            .request_value(
                "connection.update",
                change.clone(),
                scope(&s.cid, "caps-before-gain", 2),
            )
            .await
            .unwrap();
        unavailable.ack().unwrap();
        assert_eq!(
            unavailable
                .frames
                .iter()
                .find(|e| e.kind() == "operation.updated")
                .unwrap()
                .original()["payload"]["state"],
            "failed"
        );
        let first_scope = scope(&s.cid, "caps-declared-pcmu", 2);
        let first = s.clients[1]
            .advertise_native_capabilities(
                first_scope.clone(),
                &pid,
                pcmu.clone(),
                "endpoint-change",
            )
            .await
            .unwrap();
        assert_eq!(first.state, OperationState::Completed, "{first:?}");
        let gained_scope = scope(&s.cid, "caps-declared-pcma", 2);
        assert_eq!(
            s.clients[1]
                .advertise_native_capabilities(
                    gained_scope.clone(),
                    &pid,
                    pcma.clone(),
                    "endpoint-change"
                )
                .await
                .unwrap()
                .state,
            OperationState::Completed
        );
        assert_eq!(
            s.clients[1]
                .advertise_native_capabilities(first_scope, &pid, pcmu, "endpoint-change")
                .await
                .unwrap()
                .result,
            first.result
        );
        assert_eq!(
            readiness(&s.f)["advertised_capabilities"]["capabilities"],
            pcma
        );
        // Declaration and historical replay do not alter the original RTP path.
        assert_audio_reaches_other_participant(&mut s.clients, s.sh, &[s.th], 86000).await;
        let changed_scope = scope(&s.cid, "caps-gained-pcma", 2);
        s.clients[1]
            .send_value("connection.update", change.clone(), changed_scope.clone())
            .await
            .unwrap();
        let callback = s.clients[1].receive_control().await.unwrap();
        assert_eq!(callback.kind(), "connection.offer");
        let selected = &callback.original()["payload"]["substrate_setup"]["streams"][0];
        assert_eq!(selected["codec"]["name"], "pcma");
        let changed = selected["stream_local_id"].as_u64().unwrap() as u16;
        assert_ne!(changed, s.th);
        codecs::answer(&mut s.clients[1], &callback, &changed_scope, false).await;
        let done = codecs::completed(&mut s.clients[1], "completed").await;
        assert_eq!(done.original()["payload"]["result"]["stream_revision"], 3);
        codecs::decoded_pcma(&mut s.clients, s.sh, changed, 87000).await;
        codecs::decoded_return(&mut s.clients, s.sh, changed, 88000).await;
        assert_eq!(
            s.clients[1]
                .advertise_native_capabilities(
                    scope(&s.cid, "caps-withdraw", 3),
                    &pid,
                    json!({}),
                    "network-adapt"
                )
                .await
                .unwrap()
                .state,
            OperationState::Completed
        );
        // A late replay must not restore the old future-negotiation snapshot.
        assert_eq!(
            s.clients[1]
                .advertise_native_capabilities(gained_scope, &pid, pcma, "endpoint-change")
                .await
                .unwrap()
                .state,
            OperationState::Completed
        );
        assert_eq!(
            readiness(&s.f)["advertised_capabilities"]["capabilities"],
            json!({})
        );
        let refused = s.clients[1]
            .request_value(
                "connection.update",
                change,
                scope(&s.cid, "caps-after-withdraw", 3),
            )
            .await
            .unwrap();
        refused.ack().unwrap();
        assert_eq!(
            refused
                .frames
                .iter()
                .find(|e| e.kind() == "operation.updated")
                .unwrap()
                .original()["payload"]["state"],
            "failed"
        );
        // Existing port selectors remain usable without new parameter exchange.
        let retained = s.clients[1]
            .request_value(
                "stream.subscribe",
                json!({"subscriptions":[{"strm_id":"codec-voice"}]}),
                scope(&s.cid, "caps-retain-selector", 3),
            )
            .await
            .unwrap();
        retained.ack().unwrap();
        assert_eq!(
            retained
                .frames
                .iter()
                .find(|e| e.kind() == "operation.updated")
                .unwrap()
                .original()["payload"]["state"],
            "completed"
        );
        codecs::decoded_pcma(&mut s.clients, s.sh, changed, 89000).await;
        codecs::decoded_return(&mut s.clients, s.sh, changed, 90000).await;
        for client in s.clients {
            client.shutdown().await.unwrap();
        }
        s.f.server.shutdown().await.unwrap();
    }
}
#[tokio::test]
async fn production_native_capability_receipt_replay_preserves_newer_descriptor_without_changing_actual_audio(
) {
    for algorithm in ["EdDSA", "ES256"] {
        let mut f = Fixture::with_native_end_scope(algorithm, 12).await;
        f.client.timeout_ms = 8000;
        let (mut s, ()) =
            codecs::setup_started(f, (), "capability-target-offer", "capability-source-offer")
                .await;
        let pid = participant(&s.f, "codec-target");
        let before = readiness(&s.f);
        let first_scope = scope(&s.cid, "capability-first", 2);
        let descriptor = json!({"audio_codecs":[{"name":"g.711-mu","params":{"sample_rate":8000}}],"identity_assurance_offered":"identified","future":{"exact":[1,true]}});
        let first = s.clients[1]
            .advertise_native_capabilities(
                first_scope.clone(),
                &pid,
                descriptor.clone(),
                "endpoint-change",
            )
            .await
            .unwrap();
        assert_eq!(first.state, OperationState::Completed);
        assert_eq!(first.result.as_ref().unwrap()["connection_generation"], 1);
        let latest = json!({"audio_codecs":[],"data_protocols":["future-protocol"],"future":{"latest":"exact"}});
        assert_eq!(
            s.clients[1]
                .advertise_native_capabilities(
                    scope(&s.cid, "capability-latest", 2),
                    &pid,
                    latest.clone(),
                    "network-adapt"
                )
                .await
                .unwrap()
                .state,
            OperationState::Completed
        );
        let replay = s.clients[1]
            .advertise_native_capabilities(
                first_scope.clone(),
                &pid,
                descriptor.clone(),
                "endpoint-change",
            )
            .await
            .unwrap();
        assert_eq!(first.result, replay.result);
        let mut after = readiness(&s.f);
        let stored = after
            .as_object_mut()
            .unwrap()
            .remove("advertised_capabilities")
            .unwrap();
        assert_eq!(stored["capabilities"], latest);
        assert_eq!(stored["by_participant"], pid);
        assert_eq!(after, before);
        let changed = s.clients[1]
            .advertise_native_capabilities(first_scope, &pid, descriptor, "operator-policy")
            .await;
        assert!(matches!(changed,Err(ClientError::Application(e)) if e.code=="conflict"));
        let false_assurance = s.clients[1]
            .advertise_native_capabilities(
                scope(&s.cid, "capability-false-assurance", 2),
                &pid,
                json!({"identity_assurance_offered":"user-authorized"}),
                "endpoint-change",
            )
            .await
            .unwrap();
        assert_eq!(false_assurance.state, OperationState::Failed);
        assert_eq!(
            readiness(&s.f)["advertised_capabilities"]["capabilities"],
            latest
        );
        let wrong = s.clients[1]
            .advertise_native_capabilities(
                scope(&s.cid, "capability-wrong-member", 2),
                "foreign-participant",
                json!({}),
                "endpoint-change",
            )
            .await;
        assert!(matches!(wrong,Err(ClientError::Application(e)) if e.code=="forbidden"));
        let mut foreign = CanonicalClient::open(s.f.client.clone()).await.unwrap();
        let denied = foreign
            .advertise_native_capabilities(
                scope(&s.cid, "capability-foreign", 2),
                &pid,
                json!({}),
                "endpoint-change",
            )
            .await;
        assert!(matches!(denied,Err(ClientError::Application(e)) if e.code=="forbidden"));
        foreign.shutdown().await.unwrap();
        let stale = s.clients[1]
            .advertise_native_capabilities(
                scope(&s.cid, "capability-stale", 1),
                &pid,
                json!({}),
                "endpoint-change",
            )
            .await
            .unwrap();
        assert_eq!(stale.state, OperationState::Failed);
        let mut http = CanonicalHttpClient::open(canonical_http_config(&s.f))
            .await
            .unwrap();
        let response = http
            .request_value(
                "capability.advertise",
                json!({"by_participant":pid,"capabilities":{},"trigger":"endpoint-change"}),
                scope(&s.cid, "capability-http", 2),
            )
            .await;
        let error = match response {
            Ok(batch) => batch.ack().unwrap_err(),
            Err(e) => e,
        };
        assert!(
            matches!(error,ClientError::Application(e) if matches!(e.code.as_str(),"unsupported-media-profile"|"unsupported-command"))
        );
        assert_eq!(
            readiness(&s.f)["advertised_capabilities"]["capabilities"],
            latest
        );
        assert_audio_reaches_other_participant(&mut s.clients, s.sh, &[s.th], 83000).await;
        s.clients.swap(0, 1);
        assert_audio_reaches_other_participant(&mut s.clients, s.th, &[s.sh], 84000).await;
        for client in s.clients {
            client.shutdown().await.unwrap();
        }
        s.f.server.shutdown().await.unwrap();
    }
}
#[tokio::test]
async fn production_native_capability_journal_failure_keeps_metadata_atomic_and_recovery_releases_source_readers(
) {
    let mut s = codecs::setup().await;
    let pid = participant(&s.f, "codec-target");
    let c = rusqlite::Connection::open(&s.f.config.database).unwrap();
    c.execute_batch("CREATE TRIGGER controlled_capability_record_failure BEFORE INSERT ON journal WHEN json_extract(NEW.data_json,'$.operation_id')='capability-recover' AND json_extract(NEW.data_json,'$.state')='completed' BEGIN SELECT RAISE(ABORT,'controlled capability recording failure'); END;").unwrap();
    let command = scope(&s.cid, "capability-recover", 2);
    let descriptor = json!({"audio_codecs":[{"name":"pcmu","params":{}}]});
    let pending = s.clients[1]
        .advertise_native_capabilities(command.clone(), &pid, descriptor.clone(), "endpoint-change")
        .await
        .unwrap();
    assert_eq!(pending.state, OperationState::Preparing);
    assert!(readiness(&s.f).get("advertised_capabilities").is_none());
    assert_audio_reaches_other_participant(&mut s.clients, s.sh, &[s.th], 85000).await;
    barrier_without_advertisement(&mut s.clients[0]).await;
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
    c.execute_batch("DROP TRIGGER controlled_capability_record_failure")
        .unwrap();
    drop(c);
    let until = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let value = s.clients[1]
            .operation_get(
                "capability-recover",
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
            .advertise_native_capabilities(command, &pid, descriptor.clone(), "endpoint-change")
            .await
            .unwrap()
            .state,
        OperationState::Completed
    );
    assert_eq!(
        readiness(&s.f)["advertised_capabilities"]["capabilities"],
        descriptor
    );
    let delivered = next_advertisement(&mut s.clients[0], "EdDSA").await;
    assert_eq!(delivered.original()["payload"]["capabilities"], descriptor);
    tokio::time::sleep(Duration::from_millis(100)).await;
    barrier_without_advertisement(&mut s.clients[0]).await;
    let effects:u64=rusqlite::Connection::open(&s.f.config.database).unwrap().query_row("SELECT COUNT(*) FROM operation_effects e JOIN operations o ON o.record_id=e.operation_record_id WHERE o.operation_id='capability-recover'",[],|r|r.get(0)).unwrap();
    assert_eq!(effects, 1);
    for client in s.clients {
        client.shutdown().await.unwrap();
    }
    s.f.server.shutdown().await.unwrap();
}
#[tokio::test]
async fn production_native_capability_advertisement_requires_its_named_permission_beyond_connection_creation(
) {
    let f = Fixture::with_native_end_scope("EdDSA", 8).await;
    ready_endpoint(&f).await;
    let mut limited = Scope::collaboration(vec!["*".into()]);
    limited.operations.retain(|p| p != "capability.advertise");
    let config = party_with_scope(
        &f,
        &f.client.identity_id,
        108,
        vec!["audio".into()],
        limited,
    );
    let mut owner = CanonicalClient::open(f.client.clone()).await.unwrap();
    let cid = session(&f, &mut owner).await;
    let mut peer = CanonicalClient::open(config.clone()).await.unwrap();
    negotiate(
        &mut peer,
        &cid,
        "capability-limited-offer",
        "capability-limited-slot",
    )
    .await;
    wait_native_operation(&f, &config.credential, &cid, "capability-limited-offer").await;
    let pid = participant(&f, "capability-limited-slot");
    let scope = CommandScope {
        conversation_id: Some(cid),
        session_id: Some("native-session".into()),
        connection_id: Some("capability-limited-slot".into()),
        operation_id: Some("capability-limited".into()),
        expected_revision: Some(1),
    };
    let denied = peer
        .advertise_native_capabilities(scope, &pid, json!({}), "endpoint-change")
        .await;
    assert!(matches!(denied,Err(ClientError::Application(e)) if e.code=="forbidden"));
    let count: u64 = rusqlite::Connection::open(&f.config.database)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM operations WHERE operation_id='capability-limited'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    assert!(f.server.store.authenticate(&config.credential).is_ok());
    let id = peer.keepalive().await.unwrap();
    reply(&mut peer, &id, "ack").await;
    peer.shutdown().await.unwrap();
    owner.shutdown().await.unwrap();
    f.server.shutdown().await.unwrap();
}
