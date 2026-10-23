//! Actual consent, same physical native source and unchanged held signal intent.
use super::*;
use connect_client::application::ApplicationClient;
use connect_client::canonical::PublicNativeAudio;
use connect_protocol::user_delegation::*;

async fn setup(algorithm: &str) -> (Fixture, String, String, String, CanonicalClientConfig) {
    let s = codecs::setup_started_dtmf(Fixture::with_native_dtmf_scope(algorithm, 12).await, true)
        .await;
    let codecs::Running {
        mut f,
        clients,
        cid,
        ..
    } = s;
    for client in &clients {
        client.close();
    }
    drop(clients);
    let user = f
        .server
        .store
        .create_identity("native-signal-human-consenter")
        .unwrap()
        .id;
    let actor = f.server.store.authenticate(&f.client.credential).unwrap();
    let mut operations = actor.scope.operations.clone();
    operations.push("credential.delegate".into());
    let human = f
        .server
        .store
        .issue(
            &user,
            Scope {
                operations,
                recipients: vec![f.client.identity_id.clone(), f.peer.identity_id.clone()],
            },
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    f.config.bind = f.server.address;
    f.config.http_bind = f.server.http_address;
    f.config.user_assurance = vec![UserAssuranceRule {
        conversation_id: cid.clone(),
        command: "dtmf.send".into(),
        user_id: user.clone(),
    }];
    f.server.shutdown().await.unwrap();
    let path = f.temp.path().join("native-step-up-server.toml");
    std::fs::write(&path, toml::to_string(&f.config).unwrap()).unwrap();
    f.config = CanonicalServerConfig::load(&path).unwrap();
    f.server = RunningCanonicalServer::start(f.config.clone())
        .await
        .unwrap();
    f.client.endpoint = f.server.address;
    let mut target_scope = dtmf::signal_scope();
    target_scope.operations.push("stream.subscribe".into());
    let target = party_with_scope(
        &f,
        &f.peer.identity_id,
        121,
        vec!["audio".into()],
        target_scope,
    );
    (f, cid, user, human.token, target)
}
async fn consent(f: &Fixture, user: &str, human: &str) -> UserDelegationOutcome {
    consent_with_operations(f, user, human, None).await
}
async fn consent_with_operations(
    f: &Fixture,
    user: &str,
    human: &str,
    operations: Option<Vec<String>>,
) -> UserDelegationOutcome {
    let actor = f.server.store.authenticate(&f.client.credential).unwrap();
    let mut config = canonical_http_config(f);
    config.credential = human.into();
    let mut app = ApplicationClient::open(config).await.unwrap();
    let request = UserDelegationRequest {
        operation_id: "native-signal-user-consent".into(),
        identity_id: f.client.identity_id.clone(),
        operations: operations.unwrap_or(actor.scope.operations),
        recipients: vec![f.client.identity_id.clone(), f.peer.identity_id.clone()],
        expires_at: chrono::Utc::now().timestamp() + 300,
    };
    let issued = app
        .issue_user_delegation(user, request.clone())
        .await
        .unwrap();
    let replay = app.issue_user_delegation(user, request).await.unwrap();
    assert_eq!(issued.credential_id, replay.credential_id);
    assert_eq!(issued.credential, replay.credential);
    issued
}
async fn receive_signal(audio: &mut PublicNativeAudio, operation: &str) -> Envelope {
    for _ in 0..128 {
        let e = audio.receive_signaling().await.unwrap();
        if e.kind() == "dtmf.received" {
            assert_eq!(e.scope(rvoip_uctp::v2::Scope::Operation), Some(operation));
            return e;
        }
    }
    panic!("missing signal after native step-up")
}
#[tokio::test]
async fn production_owned_native_step_up_resumes_original_dtmf_after_current_user_consent_and_media_rotation(
) {
    step_up_signal(false, false).await;
}
#[tokio::test]
async fn production_owned_native_step_up_accepts_signal_only_consent_without_audio_control() {
    step_up_signal(true, false).await;
}
#[tokio::test]
async fn production_owned_native_signal_only_refuses_new_rtp_on_the_original_source() {
    step_up_signal(true, true).await;
}
async fn step_up_signal(signal_only: bool, forbid_rtp: bool) {
    for algorithm in ["EdDSA", "ES256"] {
        let (f, cid, user, human, target_config) = setup(algorithm).await;
        let issued = consent_with_operations(
            &f,
            &user,
            &human,
            signal_only.then(|| vec!["dtmf.send".into()]),
        )
        .await;
        let mut source = dtmf::owned(
            f.client.clone(),
            &cid,
            "step-up-source",
            "step-up-source-offer",
        )
        .await;
        let mut target = dtmf::owned(
            target_config.clone(),
            &cid,
            "step-up-target",
            "step-up-target-offer",
        )
        .await;
        if forbid_rtp {
            let selection =
                vec![serde_json::from_value(json!({"strm_id":"step-up-source-voice"})).unwrap()];
            assert_eq!(
                target
                    .subscribe_audio("signal-only-baseline-subscription", 1, selection)
                    .await
                    .unwrap(),
                2
            );
            for _ in 0..16 {
                source.send_pcmu(&[0x44; 160]).unwrap();
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            let baseline = tokio::time::timeout(Duration::from_secs(2), target.receive_pcmu())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                baseline,
                vec![0x44; 160],
                "baseline route did not carry actual audio"
            );
        }
        assert!(
            matches!(source.send_dtmf("step-up-original-signal",Some(1),"9#",100,"info").await,Err(ClientError::Application(error)) if error.code=="step-up-required")
        );
        assert_eq!(source.step_up_required(), Some("user-authorized"));
        // Consent is not Device/key enrollment. This validation failure must
        // consume the challenge without rotating media or executing the signal.
        assert!(source
            .complete_step_up(issued.credential.clone())
            .await
            .is_err());
        assert_eq!(source.step_up_required(), None);
        let db = rusqlite::Connection::open(&f.config.database).unwrap();
        let before:(u64,u64)=db.query_row("SELECT (SELECT count(*) FROM native_media_credential_bindings),(SELECT count(*) FROM operations WHERE operation_id='step-up-original-signal')",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(before, (0, 0));
        drop(db);
        dtmf::bind_fixture_successor(
            &f,
            &f.client,
            &issued.credential_id,
            "enroll-native-step-up-consent",
        );
        assert!(
            matches!(source.send_dtmf("step-up-original-signal",Some(1),"9#",100,"info").await,Err(ClientError::Application(error)) if error.code=="step-up-required")
        );
        let completed = source
            .complete_step_up(issued.credential.clone())
            .await
            .unwrap();
        assert_eq!(completed.authentication.assurance, "user-authorized");
        assert_eq!(completed.authentication.identity_id, f.client.identity_id);
        completed.replies.ack().unwrap();
        assert_eq!(source.step_up_required(), None);
        let frame = receive_signal(&mut target, "step-up-original-signal").await;
        assert_eq!(frame.original()["payload"]["digits"], "9#");
        assert_eq!(source.stream_revision(), 1);
        let db = rusqlite::Connection::open(&f.config.database).unwrap();
        let original:(String,u64)=db.query_row("SELECT record_id,revision FROM operations WHERE operation_id='step-up-original-signal'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        let epoch:(String,u64,u64,String,u64)=db.query_row("SELECT b.state,b.epoch,b.core_credential_revision,b.credential_id,b.stream_revision FROM native_media_credential_bindings b JOIN operations o ON o.record_id=b.parent_record_id WHERE o.connection_id='step-up-source'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
        assert_eq!(epoch, ("applied".into(), 1, 2, issued.credential_id, 1));
        let observed:(String,u64)=db.query_row("SELECT json_extract(observation_json,'$.envelope_id'),json_extract(observation_json,'$.credential_revision') FROM native_dtmf_sources WHERE operation_record_id=?1",[&original.0],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(
            observed,
            (completed.replies.request_id, 2),
            "step-up replaced the original held envelope identity"
        );

        drop(db);
        source
            .send_dtmf("step-up-original-signal", Some(1), "9#", 100, "info")
            .await
            .unwrap();
        let db = rusqlite::Connection::open(&f.config.database).unwrap();
        let repeated:(String,u64)=db.query_row("SELECT record_id,revision FROM operations WHERE operation_id='step-up-original-signal'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(repeated.0, original.0);
        let effects: u64 = db
            .query_row(
                "SELECT count(*) FROM operation_effects WHERE operation_record_id=?1",
                [&original.0],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(effects, 1);
        let deadline = tokio::time::Instant::now() + Duration::from_millis(100);
        while let Ok(Ok(e)) = tokio::time::timeout_at(deadline, target.receive_signaling()).await {
            assert_ne!(
                e.kind(),
                "dtmf.received",
                "logical replay emitted a second signal"
            );
        }
        drop(db);
        if signal_only {
            let current = f.server.store.authenticate(&issued.credential).unwrap();
            assert!(!current.permits("connection.offer"));
            assert!(!current.permits("stream.subscribe"));
            assert!(!current.permits("dtmf.receive"));
            if forbid_rtp {
                // Verify fresh signaling before the terminal media-abuse probe.
                source
                    .send_dtmf("signal-only-before-rtp-refusal", Some(1), "7", 100, "info")
                    .await
                    .unwrap();
                receive_signal(&mut target, "signal-only-before-rtp-refusal").await;
                // Drain only historical packets, then send new RTP on the exact
                // original source handle after the signal-only epoch was applied.
                for _ in 0..256 {
                    if tokio::time::timeout(Duration::from_millis(5), target.receive_pcmu())
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                for _ in 0..16 {
                    source.send_pcmu(&[0x55; 160]).unwrap();
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                assert!(
                    tokio::time::timeout(Duration::from_millis(250), target.receive_pcmu())
                        .await
                        .is_err(),
                    "signal-only consent inherited RTP publication"
                );
            } else {
                let receiver = f
                    .server
                    .store
                    .issue(
                        &target_config.identity_id,
                        Scope {
                            operations: vec!["dtmf.receive".into()],
                            recipients: vec![f.client.identity_id.clone()],
                        },
                        chrono::Utc::now().timestamp() + 300,
                    )
                    .unwrap();
                dtmf::bind_fixture_successor(
                    &f,
                    &target_config,
                    &receiver.id,
                    "enroll-signal-only-receiver",
                );
                target.refresh_credential(receiver.token).await.unwrap();
                source
                    .send_dtmf("signal-only-refreshed-receiver", Some(1), "8", 100, "info")
                    .await
                    .unwrap();
                let delivered = receive_signal(&mut target, "signal-only-refreshed-receiver").await;
                assert_eq!(delivered.original()["payload"]["digits"], "8");
                let denied = source
                    .subscribe_audio(
                        "signal-only-audio-refused",
                        1,
                        vec![
                            serde_json::from_value(json!({"strm_id":"step-up-target-voice"}))
                                .unwrap(),
                        ],
                    )
                    .await;
                assert!(
                matches!(denied, Err(ClientError::Application(ref error)) if error.code == "forbidden"),
                "signal consent did not produce an explicit audio permission refusal: {denied:?}"
            );
                let db = rusqlite::Connection::open(&f.config.database).unwrap();
                let count:u64 = db.query_row("SELECT count(*) FROM operations WHERE operation_id='signal-only-audio-refused'", [], |r|r.get(0)).unwrap();
                assert_eq!(count, 0);
            }
            source.close();
        } else {
            source.shutdown().await.unwrap();
        }
        target.shutdown().await.unwrap();
        f.server.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn production_owned_native_step_up_keeps_authentication_separate_from_original_sql_refusal_and_retry(
) {
    for algorithm in ["EdDSA", "ES256"] {
        let (f, cid, user, human, target_config) = setup(algorithm).await;
        let issued = consent(&f, &user, &human).await;
        dtmf::bind_fixture_successor(
            &f,
            &f.client,
            &issued.credential_id,
            "enroll-refused-native-step-up",
        );
        let mut source = dtmf::owned(
            f.client.clone(),
            &cid,
            "step-up-refused-source",
            "step-up-refused-source-offer",
        )
        .await;
        let mut target = dtmf::owned(
            target_config,
            &cid,
            "step-up-refused-target",
            "step-up-refused-target-offer",
        )
        .await;
        let db = rusqlite::Connection::open(&f.config.database).unwrap();
        db.execute_batch("CREATE TRIGGER refuse_held_signal BEFORE INSERT ON operations WHEN NEW.operation_id='step-up-refused-signal' BEGIN SELECT RAISE(ABORT,'controlled original signal refusal'); END;").unwrap();
        assert!(
            matches!(source.send_dtmf("step-up-refused-signal",Some(1),"2",100,"info").await,Err(ClientError::Application(error)) if error.code=="step-up-required")
        );
        let completion = source.complete_step_up(issued.credential).await.unwrap();
        assert_eq!(completion.authentication.assurance, "user-authorized");
        assert!(
            completion.replies.ack().is_err(),
            "authentication is not the original mutation outcome"
        );
        assert_eq!(source.step_up_required(), None);
        let facts:(u64,u64)=db.query_row("SELECT (SELECT count(*) FROM native_media_credential_bindings WHERE state='applied' AND epoch=1 AND core_credential_revision=2),(SELECT count(*) FROM operations WHERE operation_id='step-up-refused-signal')",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(facts, (1, 0));
        db.execute_batch("DROP TRIGGER refuse_held_signal").unwrap();
        // Explicit fresh-envelope retry keeps the caller's logical mutation ID.
        // It uses the committed assurance and never repeats media rotation.
        source
            .send_dtmf("step-up-refused-signal", Some(1), "2", 100, "info")
            .await
            .unwrap();
        receive_signal(&mut target, "step-up-refused-signal").await;
        let facts:(u64,u64,u64)=db.query_row("SELECT (SELECT count(*) FROM native_media_credential_bindings),(SELECT count(*) FROM operations WHERE operation_id='step-up-refused-signal'),(SELECT count(*) FROM operation_effects e JOIN operations o ON o.record_id=e.operation_record_id WHERE o.operation_id='step-up-refused-signal')",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(facts, (1, 1, 1));
        drop(db);
        source.shutdown().await.unwrap();
        target.shutdown().await.unwrap();
        f.server.shutdown().await.unwrap();
    }
}
