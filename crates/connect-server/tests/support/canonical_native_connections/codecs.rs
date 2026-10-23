//! Actual signed native codec changes, decoded media and original operation custody.
use super::*;
fn capabilities() -> Value {
    json!({"audio_codecs":[{"name":"pcmu","params":{"sample_rate":8000,"channels":1}},{"name":"pcma","params":{"sample_rate":8000,"channels":1}}],"max_streams_per_connection":8,"identity_assurance_offered":"identified"})
}
pub(super) struct Running {
    pub(super) f: Fixture,
    pub(super) clients: Vec<CanonicalClient>,
    pub(super) cid: String,
    pub(super) sh: u16,
    pub(super) th: u16,
}
pub(super) async fn setup() -> Running {
    setup_operations("codec-target-offer", "codec-source-offer").await
}
async fn setup_operations(target_operation: &str, source_operation: &str) -> Running {
    setup_configured(target_operation, source_operation, |_| ())
        .await
        .0
}
pub(super) async fn setup_with_host<T>(configure: impl FnOnce(&Fixture) -> T) -> (Running, T) {
    setup_configured("codec-target-offer", "codec-source-offer", configure).await
}
pub(super) async fn setup_with_telephone<T>(
    configure: impl FnOnce(
        &Fixture,
    ) -> (
        T,
        connect_server::canonical_telephone::CanonicalTelephoneConfig,
    ),
) -> (Running, T) {
    setup_with_telephone_profile("EdDSA", None, configure).await
}
pub(super) async fn setup_with_telephone_dtmf<T>(
    algorithm: &str,
    configure: impl FnOnce(
        &Fixture,
    ) -> (
        T,
        connect_server::canonical_telephone::CanonicalTelephoneConfig,
    ),
) -> (Running, T) {
    let mut source_scope = Scope::collaboration(vec!["*".into()]);
    source_scope.operations.extend([
        "stream.subscribe".into(),
        "stream.unsubscribe".into(),
        "dtmf.send".into(),
        "dtmf.receive".into(),
    ]);
    setup_with_telephone_profile(algorithm, Some(source_scope), configure).await
}
async fn setup_with_telephone_profile<T>(
    algorithm: &str,
    source_scope: Option<Scope>,
    configure: impl FnOnce(
        &Fixture,
    ) -> (
        T,
        connect_server::canonical_telephone::CanonicalTelephoneConfig,
    ),
) -> (Running, T) {
    let f = Fixture::with_native_end_scope(algorithm, 12).await;

    let (host, telephone) = configure(&f);
    let Fixture {
        temp,
        mut config,
        server,
        mut client,
        peer,
        admin,
        key_revision,
    } = f;
    server.shutdown().await.unwrap();
    config.telephone = Some(telephone);
    let path = temp.path().join("configured-canonical.toml");
    std::fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    config = CanonicalServerConfig::load(&path).unwrap();
    let server = RunningCanonicalServer::start(config.clone()).await.unwrap();
    client.endpoint = server.address;
    let f = Fixture {
        temp,
        config,
        server,
        client,
        peer,
        admin,
        key_revision,
    };
    setup_started_profile(
        f,
        host,
        "codec-target-offer",
        "codec-source-offer",
        capabilities(),
        source_scope,
    )
    .await
}
async fn setup_configured<T>(
    target_operation: &str,
    source_operation: &str,
    configure: impl FnOnce(&Fixture) -> T,
) -> (Running, T) {
    let f = Fixture::with_native_end_scope("EdDSA", 12).await;
    let host = configure(&f);
    setup_started(f, host, target_operation, source_operation).await
}
pub(super) async fn setup_started<T>(
    f: Fixture,
    host: T,
    target_operation: &str,
    source_operation: &str,
) -> (Running, T) {
    setup_started_capabilities(f, host, target_operation, source_operation, capabilities()).await
}
pub(super) async fn setup_started_capabilities<T>(
    f: Fixture,
    host: T,
    target_operation: &str,
    source_operation: &str,
    target_capabilities: Value,
) -> (Running, T) {
    setup_started_profile(
        f,
        host,
        target_operation,
        source_operation,
        target_capabilities,
        None,
    )
    .await
}
pub(super) async fn setup_started_dtmf(f: Fixture, receive: bool) -> Running {
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope.operations.extend([
        "stream.subscribe".into(),
        "stream.unsubscribe".into(),
        "dtmf.send".into(),
    ]);
    if receive {
        scope.operations.push("dtmf.receive".into());
    }
    setup_started_profile(
        f,
        (),
        "codec-target-offer",
        "codec-source-offer",
        capabilities(),
        Some(scope),
    )
    .await
    .0
}
async fn setup_started_profile<T>(
    f: Fixture,
    host: T,
    target_operation: &str,
    source_operation: &str,
    target_capabilities: Value,
    source_scope: Option<Scope>,
) -> (Running, T) {
    ready_endpoint(&f).await;
    let config = match source_scope {
        Some(scope) => party_with_scope(&f, &f.peer.identity_id, 99, vec!["audio".into()], scope),
        None => party_with_capabilities(&f, &f.peer.identity_id, 99, true, vec!["audio".into()]),
    };
    let mut target = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut source = CanonicalClient::open(config.clone()).await.unwrap();
    let cid = session_with_media(
        &f,
        &mut target,
        "voice",
        json!({"audio":["g.711-mu","g.711-a"]}),
    )
    .await;
    let t=negotiate_media(&mut target,&cid,target_operation,"codec-target",json!([{"id":"codec-monitor","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}]),target_capabilities).await;
    let s=negotiate_media(&mut source,&cid,source_operation,"codec-source",json!([{"id":"codec-voice","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}]),capabilities()).await;
    wait_native_operation(&f, &f.client.credential, &cid, target_operation).await;
    wait_native_operation(&f, &config.credential, &cid, source_operation).await;
    checked_control(
        &f,
        &f.client.credential,
        &mut target,
        &cid,
        "codec-target",
        "codec-select-voice",
        1,
        "stream.subscribe",
        json!({"subscriptions":[{"strm_id":"codec-voice"}]}),
    )
    .await;
    checked_control(
        &f,
        &config.credential,
        &mut source,
        &cid,
        "codec-source",
        "codec-select-return",
        1,
        "stream.subscribe",
        json!({"subscriptions":[{"strm_id":"codec-monitor"}]}),
    )
    .await;
    (
        Running {
            f,
            clients: vec![source, target],
            cid,
            sh: s.original()["payload"]["substrate_setup"]["streams"][0]["stream_local_id"]
                .as_u64()
                .unwrap() as u16,
            th: t.original()["payload"]["substrate_setup"]["streams"][0]["stream_local_id"]
                .as_u64()
                .unwrap() as u16,
        },
        host,
    )
}
async fn checked_control(
    f: &Fixture,
    token: &str,
    client: &mut CanonicalClient,
    cid: &str,
    connection: &str,
    operation: &str,
    revision: u64,
    kind: &str,
    input: Value,
) {
    use futures_util::FutureExt;
    let result = std::panic::AssertUnwindSafe(stream_control(
        client, cid, connection, operation, revision, kind, input,
    ))
    .catch_unwind()
    .await;
    if let Err(error) = result {
        let actor = f.server.store.authenticate(token).unwrap();
        let record = f.server.store.execute_operation_action(
            &actor.credential_id,
            connect_protocol::transitions::OperationAction::Get {
                operation_id: operation.into(),
            },
        );
        eprintln!("failed setup control durable record: {record:?}");
        std::panic::resume_unwind(error);
    }
}

fn scope(cid: &str, operation: &str, revision: u64) -> CommandScope {
    CommandScope {
        conversation_id: Some(cid.into()),
        session_id: Some("native-session".into()),
        connection_id: Some("codec-target".into()),
        operation_id: Some(operation.into()),
        expected_revision: Some(revision),
    }
}
pub(super) async fn answer(
    client: &mut CanonicalClient,
    callback: &Envelope,
    scope: &CommandScope,
    wrong: bool,
) {
    let p = &callback.original()["payload"];
    let streams=p["substrate_setup"]["streams"].as_array().unwrap().iter().map(|s|json!({"id":s["strm_id"],"kind":s["kind"],"direction":match s["direction"].as_str().unwrap(){"sendonly"=>"recvonly","recvonly"=>"sendonly",_=>"sendrecv"},"codec":s["codec"]})).collect::<Vec<_>>();
    let mut v = json!({"by_participant":p["by_participant"],"substrate":"quic","capabilities":capabilities(),"streams_answered":streams,"substrate_setup":p["substrate_setup"]});
    if wrong {
        v["substrate_setup"]["revision"] = json!(999);
    }
    client
        .answer_connection_offer(callback, serde_json::from_value(v).unwrap(), scope)
        .await
        .unwrap();
}
pub(super) async fn completed(client: &mut CanonicalClient, state: &str) -> Envelope {
    let e = client.receive_control().await.unwrap();
    assert_eq!(e.kind(), "operation.updated", "{:?}", e.original());
    assert_eq!(
        e.original()["payload"]["state"],
        state,
        "{:?}",
        e.original()
    );
    assert_eq!(client.receive_control().await.unwrap().kind(), "ack");
    e
}
pub(super) async fn decoded_pcma(clients: &mut [CanonicalClient], sh: u16, th: u16, sequence: u32) {
    let (source, target) = clients.split_at_mut(1);
    decoded_audio(&source[0], &mut target[0], sh, 0, th, 8, sequence).await;
}
pub(super) async fn decoded_return(
    clients: &mut [CanonicalClient],
    sh: u16,
    th: u16,
    sequence: u32,
) {
    let (target, source) = clients.split_at_mut(1);
    decoded_audio(&source[0], &mut target[0], th, 8, sh, 0, sequence).await;
}
pub(super) async fn decoded_audio(
    source: &CanonicalClient,
    target: &mut CanonicalClient,
    sh: u16,
    source_pt: u8,
    th: u16,
    target_pt: u8,
    sequence: u32,
) {
    use rvoip_uctp::substrate::{
        pack_rtp_datagram, unpack_rtp_datagram, RtpDatagram, RtpMediaPayload,
    };
    let send = async {
        for seq in sequence..sequence + 60 {
            source
                .send_native_rtp_datagram(
                    &pack_rtp_datagram(&RtpDatagram {
                        flags: 0,
                        stream_local_id: sh,
                        seq,
                        rtp: RtpMediaPayload {
                            payload: (0..160)
                                .map(|n| {
                                    if source_pt == 8 {
                                        if n % 16 < 8 {
                                            0xaa
                                        } else {
                                            0x2a
                                        }
                                    } else if n % 16 < 8 {
                                        0x80
                                    } else {
                                        0x00
                                    }
                                })
                                .collect::<Vec<_>>()
                                .into(),
                            payload_type: source_pt,
                            sequence_number: seq as u16,
                            timestamp: seq * 160,
                            ssrc: 0x6463_7001,
                        },
                    })
                    .unwrap(),
                )
                .unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    let receive = tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let bytes = target.receive_native_rtp_datagram().await.unwrap();
            let packet = unpack_rtp_datagram(&bytes).unwrap();
            if packet.rtp.payload_type != target_pt {
                continue;
            }
            assert_eq!(packet.stream_local_id, th);
            // Independent FFmpeg G.711 decoding proves real non-silent audio,
            // rather than trusting a negotiated codec name or payload type.
            use std::{
                io::Write,
                process::{Command, Stdio},
            };
            let mut decoder = Command::new("ffmpeg")
                .args([
                    "-v",
                    "error",
                    "-f",
                    if target_pt == 8 { "alaw" } else { "mulaw" },
                    "-ar",
                    "8000",
                    "-ac",
                    "1",
                    "-i",
                    "pipe:0",
                    "-f",
                    "s16le",
                    "pipe:1",
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            decoder
                .stdin
                .take()
                .unwrap()
                .write_all(&packet.rtp.payload)
                .unwrap();
            let output = decoder.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            if output
                .stdout
                .chunks_exact(2)
                .map(|s| i16::from_le_bytes([s[0], s[1]]))
                .any(|s| s.unsigned_abs() > 500)
            {
                break;
            }
        }
    });
    let (_, r) = tokio::join!(send, receive);
    r.expect("native codec replacement must deliver actual decoded PCMA");
}
#[tokio::test]
async fn production_native_codec_change_confirms_decoded_pcma_preserves_old_audio_and_replays_without_reapplying(
) {
    codec_change(false).await;
}
#[tokio::test]
async fn production_native_codec_captured_commit_recovers_after_recording_failure_role_writer_and_cancel(
) {
    codec_change(true).await;
}
async fn codec_change(recording_failure: bool) {
    let Running {
        f,
        mut clients,
        cid,
        sh,
        th,
    } = setup().await;
    assert_audio_reaches_other_participant(&mut clients, sh, &[th], 30000).await;
    let scope = scope(&cid, "codec-to-pcma", 2);
    let payload = json!({"action":"codec-renegotiate","streams":["codec-monitor"],"codec_preferences":["pcma"]});
    clients[1]
        .send_value("connection.update", payload.clone(), scope.clone())
        .await
        .unwrap();
    let callback = clients[1].receive_control().await.unwrap();
    assert_eq!(
        callback.kind(),
        "connection.offer",
        "{:?}",
        callback.original()
    );
    let selected = &callback.original()["payload"]["substrate_setup"]["streams"][0];
    assert_eq!(selected["codec"]["name"], "pcma");
    assert_eq!(selected["codec"]["params"]["payload_type"], 8);
    let changed = selected["stream_local_id"].as_u64().unwrap() as u16;
    assert_ne!(changed, th);
    answer(&mut clients[1], &callback, &scope, true).await;
    assert_eq!(clients[1].receive_control().await.unwrap().kind(), "error");
    assert_audio_reaches_other_participant(&mut clients, sh, &[th], 30200).await;
    let c = rusqlite::Connection::open(&f.config.database).unwrap();
    if recording_failure {
        c.execute_batch("CREATE TRIGGER controlled_codec_record_failure BEFORE INSERT ON journal WHEN json_extract(NEW.data_json,'$.operation_id')='codec-to-pcma' AND json_extract(NEW.data_json,'$.state')='completed' BEGIN SELECT RAISE(ABORT,'controlled codec recording failure'); END;").unwrap();
    }
    answer(&mut clients[1], &callback, &scope, false).await;
    let result = completed(
        &mut clients[1],
        if recording_failure {
            "preparing"
        } else {
            "completed"
        },
    )
    .await;
    decoded_pcma(&mut clients, sh, changed, 30400).await;
    decoded_return(&mut clients, sh, changed, 30500).await;
    if recording_failure {
        assert_eq!(
            c.query_row(
                "SELECT state FROM operations WHERE operation_id='codec-to-pcma'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "preparing"
        );
        let revision = c
            .query_row(
                "SELECT topology_revision FROM sessions WHERE id='native-session'",
                [],
                |r| r.get::<_, u64>(0),
            )
            .unwrap();
        tokio::time::timeout(
            Duration::from_secs(2),
            clients[1].session_command(Request {
                operation_id: "role-during-codec-recording".into(),
                conversation_id: cid.clone(),
                session_id: "native-session".into(),
                by_participant: None,
                expected_revision: Some(revision),
                action: Action::Role {
                    participant_id: callback.original()["payload"]["by_participant"]
                        .as_str()
                        .unwrap()
                        .into(),
                    role: "speaker".into(),
                },
            }),
        )
        .await
        .unwrap()
        .unwrap();
        let mut permission = Scope::collaboration(vec!["*".into()]);
        permission.operations.push("operation.cancel".into());
        let cancellation = f
            .server
            .store
            .issue(
                &f.client.identity_id,
                permission,
                chrono::Utc::now().timestamp() + 600,
            )
            .unwrap();
        let mut config = canonical_http_config(&f);
        config.credential = cancellation.token;
        let mut canceller = CanonicalHttpClient::open(config).await.unwrap();
        canceller
            .request(
                super::payload("operation.cancel", json!({"operation_id":"codec-to-pcma"})),
                CommandScope {
                    conversation_id: Some(cid.clone()),
                    operation_id: Some("cancel-captured-codec".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        c.execute_batch("DROP TRIGGER controlled_codec_record_failure")
            .unwrap();
        wait_native_operation(&f, &f.client.credential, &cid, "codec-to-pcma").await;
        assert!(c
            .query_row(
                "SELECT cancel_requested FROM operations WHERE operation_id='codec-to-pcma'",
                [],
                |r| r.get::<_, bool>(0)
            )
            .unwrap());
    }
    clients[1]
        .send_value("connection.update", payload, scope)
        .await
        .unwrap();
    let replay = completed(&mut clients[1], "completed").await;
    if !recording_failure {
        assert_eq!(replay.original()["payload"], result.original()["payload"]);
    }
    let observed = &replay.original()["payload"]["result"];
    assert_eq!(observed["stream_revision"], 3);
    assert_eq!(observed["codec_bindings"][0]["codec"]["name"], "pcma");
    assert_eq!(observed["codec_bindings"][0]["stream_local_id"], changed);
    decoded_pcma(&mut clients, sh, changed, 30600).await;
    assert_eq!(c.query_row("SELECT COUNT(*) FROM operation_effects e JOIN operations o ON o.record_id=e.operation_record_id WHERE o.operation_id='codec-to-pcma'",[],|r|r.get::<_,u64>(0)).unwrap(),1);
    assert_eq!(
        f.server
            .service
            .canonical_runtime_snapshot()
            .unwrap()
            .connections,
        2
    );
    for client in &mut clients {
        client.close();
    }
    f.server.shutdown().await.unwrap();
}
#[tokio::test]
async fn production_native_codec_refusals_and_foreign_physical_replay_preserve_audio_and_pending_original(
) {
    let Running {
        f,
        mut clients,
        cid,
        sh,
        th,
    } = setup().await;
    let c = rusqlite::Connection::open(&f.config.database).unwrap();
    for (operation, revision, stream, codec) in [
        ("codec-unsupported", 2, "codec-monitor", "vp9"),
        ("codec-stale", 1, "codec-monitor", "pcma"),
        ("codec-unknown-stream", 2, "unrelated-stream", "pcma"),
    ] {
        clients[1].send_value("connection.update",json!({"action":"codec-renegotiate","streams":[stream],"codec_preferences":[codec]}),scope(&cid,operation,revision)).await.unwrap();
        completed(&mut clients[1], "failed").await;
        assert_eq!(c.query_row("SELECT COUNT(*) FROM operation_effects e JOIN operations o ON o.record_id=e.operation_record_id WHERE o.operation_id=?1",[operation],|r|r.get::<_,u64>(0)).unwrap(),0);
    }
    assert_audio_reaches_other_participant(&mut clients, sh, &[th], 31000).await;
    let payload = json!({"action":"codec-renegotiate","streams":["codec-monitor"],"codec_preferences":["pcma"]});
    c.execute_batch("CREATE TRIGGER controlled_codec_begin_failure BEFORE INSERT ON journal WHEN json_extract(NEW.data_json,'$.operation_id')='pending-codec' AND json_extract(NEW.data_json,'$.state')='preparing' BEGIN SELECT RAISE(ABORT,'controlled codec attempt failure'); END;").unwrap();
    clients[1]
        .send_value(
            "connection.update",
            payload.clone(),
            scope(&cid, "pending-codec", 2),
        )
        .await
        .unwrap();
    assert_eq!(clients[1].receive_control().await.unwrap().kind(), "error");
    assert_eq!(
        c.query_row(
            "SELECT state FROM operations WHERE operation_id='pending-codec'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "accepted"
    );
    let mut foreign = CanonicalClient::open(f.client.clone()).await.unwrap();
    for operation in ["pending-codec", "foreign-new-codec"] {
        foreign
            .send_value(
                "connection.update",
                payload.clone(),
                scope(&cid, operation, 2),
            )
            .await
            .unwrap();
        assert_eq!(foreign.receive_control().await.unwrap().kind(), "error");
    }
    assert_eq!(
        c.query_row(
            "SELECT state FROM operations WHERE operation_id='pending-codec'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "accepted"
    );
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM operations WHERE operation_id='foreign-new-codec'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(c.query_row("SELECT COUNT(*) FROM operation_effects e JOIN operations o ON o.record_id=e.operation_record_id WHERE o.operation_id='pending-codec'",[],|r|r.get::<_,u64>(0)).unwrap(),0);
    c.execute_batch("DROP TRIGGER controlled_codec_begin_failure")
        .unwrap();
    clients[1]
        .send_value(
            "connection.update",
            payload,
            scope(&cid, "pending-codec", 2),
        )
        .await
        .unwrap();
    let callback = clients[1].receive_control().await.unwrap();
    assert_eq!(callback.kind(), "connection.offer");
    let changed = callback.original()["payload"]["substrate_setup"]["streams"][0]["stream_local_id"]
        .as_u64()
        .unwrap() as u16;
    answer(
        &mut clients[1],
        &callback,
        &scope(&cid, "pending-codec", 2),
        false,
    )
    .await;
    completed(&mut clients[1], "completed").await;
    decoded_pcma(&mut clients, sh, changed, 31200).await;
    foreign.close();
    for client in &mut clients {
        client.close();
    }
    f.server.shutdown().await.unwrap();
}
#[tokio::test]
async fn production_native_main_session_end_and_successor_cancel_follow_explicit_conversation_closure_policy(
) {
    use connect_client::conversations::CloseOptions;
    let Running {
        f,
        mut clients,
        cid,
        sh,
        th,
    } = setup().await;
    let room = clients[1]
        .list_conversations(Default::default())
        .await
        .unwrap()
        .conversations
        .into_iter()
        .find(|c| c.id == cid)
        .unwrap();
    let source = room
        .participants
        .iter()
        .find(|p| p.identity_id == f.peer.identity_id)
        .unwrap()
        .participant_id
        .clone();
    let refused = clients[1]
        .close_conversation(
            "close-active-native-room",
            &cid,
            CloseOptions {
                expected_revision: Some(room.revision),
                ..Default::default()
            },
        )
        .await;
    assert!(
        matches!(refused,Err(connect_client::ClientError::Application(ref e)) if e.code=="conflict"),
        "{:?}",
        refused
    );
    assert_audio_reaches_other_participant(&mut clients, sh, &[th], 32000).await;
    let end = Request {
        operation_id: "end-main-native-session".into(),
        conversation_id: cid.clone(),
        session_id: "native-session".into(),
        by_participant: None,
        expected_revision: Some(1),
        action: Action::End {
            reason_code: 1000,
            reason: "finished main media".into(),
        },
    };
    let first = clients[1].session_command(end.clone()).await.unwrap();
    assert_eq!(clients[1].session_command(end).await.unwrap(), first);
    wait_slots(&f, 0).await;
    let counts = f.server.service.canonical_runtime_snapshot().unwrap();
    assert_eq!(counts.connections, 0);
    assert_eq!(counts.live_sessions, 0);
    let room = clients[1]
        .list_conversations(Default::default())
        .await
        .unwrap()
        .conversations
        .into_iter()
        .find(|c| c.id == cid)
        .unwrap();
    assert!(!room.closed);
    let invited = clients[1]
        .session_command(Request {
            operation_id: "invite-successor-session".into(),
            conversation_id: cid.clone(),
            session_id: "successor-session".into(),
            by_participant: None,
            expected_revision: None,
            action: Action::Invite {
                participants: vec![source.clone()],
                medium: "voice".into(),
                intent: "conference".into(),
                capabilities: json!({"audio":["g.711-mu"]}),
            },
        })
        .await
        .unwrap();
    let Outcome::SessionControl(ref update) = invited else {
        panic!()
    };
    assert!(update.session.is_main);
    assert_eq!(update.session.state, "invited");
    let refused = clients[1]
        .close_conversation("close-invited-native-room", &cid, CloseOptions::default())
        .await;
    assert!(
        matches!(refused,Err(connect_client::ClientError::Application(ref e)) if e.code=="conflict")
    );
    clients[1]
        .session_command(Request {
            operation_id: "cancel-successor-session".into(),
            conversation_id: cid.clone(),
            session_id: "successor-session".into(),
            by_participant: None,
            expected_revision: Some(0),
            action: Action::Cancel {
                reason_code: 1000,
                reason: "no longer needed".into(),
            },
        })
        .await
        .unwrap();
    assert_eq!(
        f.server
            .service
            .canonical_runtime_snapshot()
            .unwrap()
            .live_sessions,
        0,
        "a cancelled main Session must retire its disposable coordinator"
    );
    clients[1]
        .session_command(Request {
            operation_id: "invite-rejected-main".into(),
            conversation_id: cid.clone(),
            session_id: "rejected-successor".into(),
            by_participant: None,
            expected_revision: None,
            action: Action::Invite {
                participants: vec![source],
                medium: "voice".into(),
                intent: "conference".into(),
                capabilities: json!({"audio":["g.711-mu"]}),
            },
        })
        .await
        .unwrap();
    // A synchronous identity writer on this single-thread runtime would stop
    // the pending invitation's actual IO from releasing its reader. Admin
    // Store work belongs on the blocking pool, including in this fixture.
    let store = f.server.store.clone();
    let identity = f.peer.identity_id.clone();
    let expires = chrono::Utc::now().timestamp() + 600;
    let recipient = tokio::time::timeout(
        Duration::from_secs(3),
        tokio::task::spawn_blocking(move || {
            store.issue(&identity, Scope::collaboration(vec!["*".into()]), expires)
        }),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    let state: String = rusqlite::Connection::open(&f.config.database)
        .unwrap()
        .query_row(
            "SELECT state FROM sessions WHERE id='rejected-successor'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        state, "invited",
        "credential setup must not settle the pending invitation"
    );
    let mut config = canonical_http_config(&f);
    config.credential = recipient.token;
    let mut http = CanonicalHttpClient::open(config).await.unwrap();
    http.session_command(Request {
        operation_id: "reject-successor-main".into(),
        conversation_id: cid.clone(),
        session_id: "rejected-successor".into(),
        by_participant: None,
        expected_revision: Some(0),
        action: Action::Reject {
            reason_code: 1000,
            reason: "declined".into(),
        },
    })
    .await
    .unwrap();
    assert_eq!(
        f.server
            .service
            .canonical_runtime_snapshot()
            .unwrap()
            .live_sessions,
        0,
        "an HTTP rejection must also retire the native coordinator"
    );
    let closed = clients[1]
        .close_conversation("close-settled-native-room", &cid, CloseOptions::default())
        .await
        .unwrap();
    assert!(closed.closed);
    assert_eq!(
        clients[1]
            .close_conversation("close-settled-native-room", &cid, CloseOptions::default())
            .await
            .unwrap(),
        closed
    );
    for client in &mut clients {
        client.close();
    }
    f.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn production_native_admissions_keep_equal_owner_scoped_operation_ids_distinct() {
    let Running {
        f,
        mut clients,
        cid,
        sh,
        th,
    } = setup_operations("same-native-operation", "same-native-operation").await;
    let c = rusqlite::Connection::open(&f.config.database).unwrap();
    let rows = c.prepare("SELECT record_id,state FROM operations WHERE operation_id='same-native-operation' ORDER BY record_id").unwrap()
        .query_map([], |r| Ok((r.get::<_,String>(0)?, r.get::<_,String>(1)?))).unwrap()
        .collect::<std::result::Result<Vec<_>, _>>().unwrap();
    assert_eq!(rows.len(), 2);
    assert_ne!(rows[0].0, rows[1].0);
    assert!(rows.iter().all(|r| r.1 == "completed"));
    assert_eq!(c.query_row("SELECT COUNT(*) FROM operation_effects e JOIN operations o ON e.operation_record_id=o.record_id WHERE o.operation_id='same-native-operation' AND e.state='confirmed'", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
    drop(c);
    assert_audio_reaches_other_participant(&mut clients, sh, &[th], 801).await;
    clients.swap(0, 1);
    assert_audio_reaches_other_participant(&mut clients, th, &[sh], 802).await;
    assert!(!cid.is_empty());
    for client in clients {
        client.close();
    }
    f.server.shutdown().await.unwrap();
}
