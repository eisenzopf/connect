//! Production signed data, real physical writers and durable message custody.
use super::*;
fn capabilities_for(protocol: &str) -> Value {
    json!({"data_protocols":[protocol],"max_streams_per_connection":8,"identity_assurance_offered":"identified"})
}
async fn select_protocol(
    target: &mut CanonicalClient,
    cid: &str,
    protocol: &str,
    selection: Value,
) {
    let scope = CommandScope {
        conversation_id: Some(cid.into()),
        session_id: Some("native-session".into()),
        connection_id: Some("data-target".into()),
        operation_id: Some("data-subscribe".into()),
        expected_revision: Some(1),
    };
    target
        .send_value(
            "stream.subscribe",
            json!({"subscriptions":[selection]}),
            scope.clone(),
        )
        .await
        .unwrap();
    loop {
        let e = target.receive_control().await.unwrap();
        match e.kind() {
            "connection.offer" => {
                let p = &e.original()["payload"];
                let streams=p["substrate_setup"]["streams"].as_array().unwrap().iter().map(|s|json!({"id":s["strm_id"],"kind":s["kind"],"direction":match s["direction"].as_str().unwrap(){"sendonly"=>"recvonly","recvonly"=>"sendonly",_=>"sendrecv"},"codec":s["codec"]})).collect::<Vec<_>>();
                target.answer_connection_offer(&e,serde_json::from_value(json!({"by_participant":p["by_participant"],"substrate":"quic","capabilities":capabilities_for(protocol),"streams_answered":streams,"substrate_setup":p["substrate_setup"]})).unwrap(),&scope).await.unwrap();
            }
            "operation.updated" => assert_eq!(
                e.original()["payload"]["state"],
                "completed",
                "{:?}",
                e.original()
            ),
            "ack" => return,
            _ => panic!("unexpected data subscription reply: {:?}", e.original()),
        }
    }
}
async fn publish(
    source: &mut CanonicalClient,
    cid: &str,
    operation: &str,
    message: &str,
    to: &str,
    body: &str,
) -> Envelope {
    publish_state(source, cid, operation, message, to, body, "completed").await
}
async fn publish_state(
    source: &mut CanonicalClient,
    cid: &str,
    operation: &str,
    message: &str,
    to: &str,
    body: &str,
    state: &str,
) -> Envelope {
    source.send_value("message.send",json!({"msg_id":message,"label":"application","to":[to],"content_type":"application/json","body":body,"reliability":{"mode":"reliable_ordered"}}),CommandScope{conversation_id:Some(cid.into()),session_id:Some("native-session".into()),connection_id:Some("data-source".into()),operation_id:Some(operation.into()),expected_revision:None}).await.unwrap();
    let reply = source.receive_control().await.unwrap();
    assert_eq!(reply.kind(), "operation.updated", "{:?}", reply.original());
    assert_eq!(
        reply.original()["payload"]["state"],
        state,
        "{:?}",
        reply.original()
    );
    if state == "completed" {
        assert_eq!(
            reply.original()["payload"]["result"]["core_queue_admission"],
            true
        );
    }
    assert_eq!(source.receive_control().await.unwrap().kind(), "ack");
    reply
}
struct Running {
    f: Fixture,
    source: CanonicalClient,
    target: CanonicalClient,
    cid: String,
    pid: String,
    config: CanonicalClientConfig,
    audio: Option<(u16, u16)>,
}
async fn setup(audio: bool) -> Running {
    setup_protocol(audio, "json").await
}
async fn setup_protocol(audio: bool, protocol: &str) -> Running {
    setup_protocol_selection(audio, protocol, false).await
}
async fn setup_protocol_selection(
    audio: bool,
    protocol: &str,
    participant_selector: bool,
) -> Running {
    let f = Fixture::with_native_end_scope("EdDSA", 12).await;
    let endpoint_caps = if audio {
        vec!["audio".into(), "data".into()]
    } else {
        vec!["data".into()]
    };
    ready_endpoint_with_capabilities(&f, endpoint_caps.clone()).await;
    let config = party_with_capabilities(&f, &f.peer.identity_id, 91, false, endpoint_caps);
    let mut target = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut source = CanonicalClient::open(config.clone()).await.unwrap();
    let cid = session_with_media(
        &f,
        &mut target,
        if audio { "voice" } else { "text-chat" },
        if audio {
            json!({"audio":["g.711-mu"],"data":["reliable-ordered"]})
        } else {
            json!({"data":["reliable-ordered"]})
        },
    )
    .await;
    let audio = if audio {
        let t=negotiate_av(&mut target,&cid,"audio-target-offer","audio-target",json!([{"id":"monitor-audio","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}])).await;
        let s=negotiate_av(&mut source,&cid,"audio-source-offer","audio-source",json!([{"id":"publisher-audio","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}])).await;
        wait_native_operation(&f, &f.client.credential, &cid, "audio-target-offer").await;
        wait_native_operation(&f, &config.credential, &cid, "audio-source-offer").await;
        stream_control(
            &mut target,
            &cid,
            "audio-target",
            "audio-subscribe",
            1,
            "stream.subscribe",
            json!({"subscriptions":[{"strm_id":"publisher-audio"}]}),
        )
        .await;
        target
            .session_command(Request {
                operation_id: "data-upgrade".into(),
                conversation_id: cid.clone(),
                session_id: "native-session".into(),
                by_participant: None,
                expected_revision: Some(1),
                action: Action::Upgrade {
                    media: vec!["data".into()],
                },
            })
            .await
            .unwrap();
        Some((
            s.original()["payload"]["substrate_setup"]["streams"][0]["stream_local_id"]
                .as_u64()
                .unwrap() as u16,
            t.original()["payload"]["substrate_setup"]["streams"][0]["stream_local_id"]
                .as_u64()
                .unwrap() as u16,
        ))
    } else {
        None
    };
    let target_answer=negotiate_media(&mut target,&cid,"data-target-offer","data-target",json!([{"id":"receiver-application","kind":"data","direction":"recvonly","codec_preferences":[protocol]}]),capabilities_for(protocol)).await;
    // Operation IDs are scoped to each actor. Core admission must not collide
    // when independent participants choose the same logical operation ID.
    let source_answer=negotiate_media(&mut source,&cid,"data-target-offer","data-source",json!([{"id":"application","kind":"data","direction":"sendonly","codec_preferences":[protocol]}]),capabilities_for(protocol)).await;
    wait_native_operation(&f, &f.client.credential, &cid, "data-target-offer").await;
    wait_native_operation(&f, &config.credential, &cid, "data-target-offer").await;
    let pid = target_answer.original()["payload"]["by_participant"]
        .as_str()
        .unwrap()
        .to_owned();
    select_protocol(&mut target, &cid, protocol, if participant_selector {
        json!({"from_participant":source_answer.original()["payload"]["by_participant"],"kinds":["data"]})
    } else { json!({"strm_id":"application"}) }).await;
    Running {
        f,
        source,
        target,
        cid,
        pid,
        config,
        audio,
    }
}
#[tokio::test]
async fn production_native_participant_data_selection_routes_real_json() {
    let Running {
        f,
        mut source,
        mut target,
        cid,
        pid,
        ..
    } = setup_protocol_selection(false, "json", true).await;
    publish(
        &mut source,
        &cid,
        "participant-data",
        "participant-message",
        &pid,
        "{\"selected\":true}",
    )
    .await;
    let delivered = target.receive_control().await.unwrap();
    assert_eq!(
        delivered.original()["payload"]["msg_id"],
        "participant-message"
    );
    assert_eq!(
        delivered.original()["payload"]["body"],
        "{\"selected\":true}"
    );
    source.close();
    target.close();
    f.server.shutdown().await.unwrap();
}
#[tokio::test]
async fn production_native_role_projection_preserves_mixed_connections_and_custom_role_permissions()
{
    let Running {
        f,
        source,
        target,
        cid,
        pid,
        audio,
        ..
    } = setup(true).await;
    let (sh, th) = audio.unwrap();
    let mut clients = vec![source, target];
    let c = rusqlite::Connection::open(&f.config.database).unwrap();
    assert_audio_reaches_other_participant(&mut clients, sh, &[th], 27000).await;
    for (index, role) in ["observer", "speaker", "observer"].iter().enumerate() {
        let before = f
            .server
            .service
            .canonical_runtime_snapshot()
            .unwrap()
            .role_projections;
        let revision = c
            .query_row(
                "SELECT topology_revision FROM sessions WHERE id='native-session'",
                [],
                |r| r.get::<_, u64>(0),
            )
            .unwrap();
        clients[1]
            .session_command(Request {
                operation_id: format!("mixed-role-{index}"),
                conversation_id: cid.clone(),
                session_id: "native-session".into(),
                by_participant: None,
                expected_revision: Some(revision),
                action: Action::Role {
                    participant_id: pid.clone(),
                    role: (*role).into(),
                },
            })
            .await
            .unwrap();
        let end = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            if f.server
                .service
                .canonical_runtime_snapshot()
                .unwrap()
                .role_projections
                > before
            {
                break;
            }
            assert!(
                tokio::time::Instant::now() < end,
                "durable role did not reach native coordinator: {role}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let stored: String = c.query_row("SELECT role FROM session_participants WHERE session_id='native-session' AND participant_id=?1", [&pid], |r|r.get(0)).unwrap();
        assert_eq!(stored, *role);
        // Canonical role labels are custom values, independent of permissions.
        assert_audio_reaches_other_participant(&mut clients, sh, &[th], 27200 + index as u32 * 200)
            .await;
        publish(
            &mut clients[0],
            &cid,
            &format!("mixed-role-data-{index}"),
            &format!("mixed-role-message-{index}"),
            &pid,
            "{\"role\":true}",
        )
        .await;
        assert_eq!(
            clients[1].receive_control().await.unwrap().original()["payload"]["body"],
            "{\"role\":true}"
        );
    }
    assert_eq!(
        f.server
            .service
            .canonical_runtime_snapshot()
            .unwrap()
            .connections,
        4
    );
    for client in &mut clients {
        client.close();
    }
    f.server.shutdown().await.unwrap();
}
#[tokio::test]
async fn production_native_data_routes_original_messages_and_records_local_writes_without_republishing_replay(
) {
    let Running {
        f,
        mut source,
        mut target,
        cid,
        pid,
        ..
    } = setup(false).await;
    let pid = pid.as_str();
    let first = publish(
        &mut source,
        &cid,
        "data-operation-1",
        "data-message-1",
        pid,
        "{\"sequence\":1}",
    )
    .await;
    let delivered = target.receive_control().await.unwrap();
    assert_eq!(
        delivered.kind(),
        "message.send",
        "{:?}",
        delivered.original()
    );
    assert_eq!(
        delivered.scope(rvoip_uctp::v2::Scope::Operation),
        Some("data-operation-1")
    );
    assert_eq!(delivered.original()["payload"]["msg_id"], "data-message-1");
    assert_eq!(delivered.original()["payload"]["body"], "{\"sequence\":1}");
    assert_eq!(
        delivered.scope(rvoip_uctp::v2::Scope::Connection),
        Some("data-target")
    );
    let replay = publish(
        &mut source,
        &cid,
        "data-operation-1",
        "data-message-1",
        pid,
        "{\"sequence\":1}",
    )
    .await;
    assert_eq!(first.original()["payload"], replay.original()["payload"]);
    publish(
        &mut source,
        &cid,
        "data-operation-2",
        "data-message-2",
        pid,
        "{\"sequence\":2}",
    )
    .await;
    let next = target.receive_control().await.unwrap();
    assert_eq!(next.original()["payload"]["msg_id"], "data-message-2");
    let c = rusqlite::Connection::open(&f.config.database).unwrap();
    let end = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let count: u64 = c
            .query_row(
                "SELECT COUNT(*) FROM native_data_writes WHERE disposition='local_written'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        if count == 2 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < end,
            "local writer outcomes not recorded: {count}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM native_data_messages", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM native_data_writes", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM native_data_writes WHERE provenance!='local_writer'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM message_receipts WHERE kind IN ('delivered','read')",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    let sender:String=c.query_row("SELECT n.participant_id FROM native_offer_operations n JOIN operations o ON o.record_id=n.operation_record_id WHERE o.connection_id='data-source'",[],|r|r.get(0)).unwrap();
    publish(
        &mut source,
        &cid,
        "data-self-only",
        "data-self-message",
        &sender,
        "{}",
    )
    .await;
    // A statically excluded target must not block the next addressed message.
    publish(
        &mut source,
        &cid,
        "data-operation-4",
        "data-message-4",
        pid,
        "{}",
    )
    .await;
    let next = target.receive_control().await.unwrap();
    assert_eq!(next.original()["payload"]["msg_id"], "data-message-4");
    let end = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        if c.query_row(
            "SELECT COUNT(*) FROM native_data_writes WHERE disposition='local_written'",
            [],
            |r| r.get::<_, u64>(0),
        )
        .unwrap()
            == 3
        {
            break;
        }
        assert!(tokio::time::Instant::now() < end);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM native_data_writes", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        3
    );
    drop(c);
    drop(source);
    drop(target);
    f.server.shutdown().await.unwrap();
}

async fn rejected(
    source: &mut CanonicalClient,
    cid: &str,
    pid: &str,
    operation: &str,
    message: &str,
    body: &str,
) {
    source.send_value("message.send",json!({"msg_id":message,"label":"application","to":[pid],"content_type":"application/json","body":body}),CommandScope{conversation_id:Some(cid.into()),session_id:Some("native-session".into()),connection_id:Some("data-source".into()),operation_id:Some(operation.into()),expected_revision:None}).await.unwrap();
    let e = source.receive_control().await.unwrap();
    assert_eq!(e.kind(), "error", "{:?}", e.original());
}
#[tokio::test]
async fn production_native_data_preflight_and_pending_replay_cannot_borrow_another_physical_peer() {
    let Running {
        f,
        mut source,
        mut target,
        cid,
        pid,
        config,
        ..
    } = setup(false).await;
    let c = rusqlite::Connection::open(&f.config.database).unwrap();
    c.execute_batch("CREATE TRIGGER data_begin_fault BEFORE INSERT ON journal WHEN json_extract(NEW.data_json,'$.operation_id')='pending-data' AND json_extract(NEW.data_json,'$.state')='preparing' BEGIN SELECT RAISE(ABORT,'controlled begin failure'); END;").unwrap();
    rejected(
        &mut source,
        &cid,
        &pid,
        "pending-data",
        "pending-message",
        "{}",
    )
    .await;
    assert_eq!(
        c.query_row(
            "SELECT state FROM operations WHERE operation_id='pending-data'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "accepted"
    );
    let mut foreign = CanonicalClient::open(config.clone()).await.unwrap();
    rejected(
        &mut foreign,
        &cid,
        &pid,
        "pending-data",
        "pending-message",
        "{}",
    )
    .await;
    assert_eq!(
        c.query_row(
            "SELECT state FROM operations WHERE operation_id='pending-data'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "accepted"
    );
    assert_eq!(c.query_row("SELECT COUNT(*) FROM operation_effects WHERE operation_record_id=(SELECT record_id FROM operations WHERE operation_id='pending-data')",[],|r|r.get::<_,u64>(0)).unwrap(),0);
    rejected(
        &mut foreign,
        &cid,
        &pid,
        "foreign-new-data",
        "foreign-new-message",
        "{}",
    )
    .await;
    rejected(
        &mut source,
        &cid,
        &pid,
        "bad-json-data",
        "bad-json-message",
        "invalid JSON",
    )
    .await;
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM native_data_messages", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        1
    );
    c.execute_batch("DROP TRIGGER data_begin_fault").unwrap();
    // Replay the exact intent, including omission of the optional reliability.
    source.send_value("message.send",json!({"msg_id":"pending-message","label":"application","to":[pid],"content_type":"application/json","body":"{}"}),CommandScope{conversation_id:Some(cid.clone()),session_id:Some("native-session".into()),connection_id:Some("data-source".into()),operation_id:Some("pending-data".into()),expected_revision:None}).await.unwrap();
    let e = source.receive_control().await.unwrap();
    assert_eq!(
        e.original()["payload"]["state"],
        "completed",
        "{:?}",
        e.original()
    );
    assert_eq!(source.receive_control().await.unwrap().kind(), "ack");
    assert_eq!(
        target.receive_control().await.unwrap().original()["payload"]["msg_id"],
        "pending-message"
    );
    drop(c);
    drop(foreign);
    drop(source);
    drop(target);
    f.server.shutdown().await.unwrap();
}
#[tokio::test]
async fn production_native_data_writer_storage_recovery_preserves_audio_and_other_connections() {
    let Running {
        f,
        source,
        target,
        cid,
        pid,
        audio,
        ..
    } = setup(true).await;
    let (sh, th) = audio.unwrap();
    let mut clients = vec![source, target];
    assert_audio_reaches_other_participant(&mut clients, sh, &[th], 24000).await;
    let c = rusqlite::Connection::open(&f.config.database).unwrap();
    c.execute_batch("CREATE TRIGGER data_writer_fault BEFORE INSERT ON journal WHEN NEW.kind='native_data_write' BEGIN SELECT RAISE(ABORT,'controlled writer recording failure'); END;").unwrap();
    publish(
        &mut clients[0],
        &cid,
        "recover-data",
        "recover-message",
        &pid,
        "{\"recover\":true}",
    )
    .await;
    assert_eq!(
        clients[1].receive_control().await.unwrap().original()["payload"]["msg_id"],
        "recover-message"
    );
    let end = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        if c.query_row(
            "SELECT COUNT(*) FROM native_data_writes WHERE disposition IS NULL",
            [],
            |r| r.get::<_, u64>(0),
        )
        .unwrap()
            == 1
        {
            break;
        }
        assert!(tokio::time::Instant::now() < end);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_audio_reaches_other_participant(&mut clients, sh, &[th], 24200).await;
    c.execute_batch("DROP TRIGGER data_writer_fault").unwrap();
    let end = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        if c.query_row(
            "SELECT COUNT(*) FROM native_data_writes WHERE disposition='local_written'",
            [],
            |r| r.get::<_, u64>(0),
        )
        .unwrap()
            == 1
        {
            break;
        }
        assert!(tokio::time::Instant::now() < end);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM native_data_writes", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        1
    );
    drop(c);
    end_native_connection(
        &mut clients[1],
        &cid,
        "data-target",
        "close-data-target",
        None,
        "completed",
    )
    .await;
    wait_slots(&f, 3).await;
    assert_audio_reaches_other_participant(&mut clients, sh, &[th], 24400).await;
    drop(clients);
    f.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn production_native_text_and_binary_data_preserve_exact_encodings_and_local_write_identity()
{
    for (protocol, body, encoding, mime) in [
        ("text", "hello 🦀", "utf8", "text/plain"),
        ("binary", "AP8=", "base64", "application/octet-stream"),
    ] {
        let Running {
            f,
            mut source,
            mut target,
            cid,
            pid,
            ..
        } = setup_protocol(false, protocol).await;
        source.send_value("message.send",json!({"msg_id":"format-message","label":"application","to":[pid],"content_type":mime,"body":body,"body_encoding":encoding}),CommandScope{conversation_id:Some(cid),session_id:Some("native-session".into()),connection_id:Some("data-source".into()),operation_id:Some("format-operation".into()),expected_revision:None}).await.unwrap();
        let result = source.receive_control().await.unwrap();
        assert_eq!(
            result.original()["payload"]["state"],
            "completed",
            "{:?}",
            result.original()
        );
        assert_eq!(source.receive_control().await.unwrap().kind(), "ack");
        let e = target.receive_control().await.unwrap();
        assert_eq!(e.kind(), "message.send");
        assert_eq!(e.original()["payload"]["msg_id"], "format-message");
        assert_eq!(e.original()["payload"]["body"], body);
        assert_eq!(e.original()["payload"]["body_encoding"], encoding);
        assert_eq!(e.original()["payload"]["content_type"], mime);
        let c = rusqlite::Connection::open(&f.config.database).unwrap();
        let end = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            if c.query_row(
                "SELECT COUNT(*) FROM native_data_writes WHERE disposition='local_written'",
                [],
                |r| r.get::<_, u64>(0),
            )
            .unwrap()
                == 1
            {
                break;
            }
            assert!(tokio::time::Instant::now() < end);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        drop(c);
        drop(source);
        drop(target);
        f.server.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn production_native_data_cancelled_queued_message_records_refusal_and_unblocks_following_data(
) {
    let Running {
        f,
        mut source,
        mut target,
        cid,
        pid,
        config,
        ..
    } = setup(false).await;
    let mut permission = Scope::collaboration(vec!["*".into()]);
    permission.operations.push("operation.cancel".into());
    let cancellation = f
        .server
        .store
        .issue(
            &f.peer.identity_id,
            permission,
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let mut http_config = canonical_http_config(&f);
    http_config.credential = cancellation.token;
    let mut canceller = CanonicalHttpClient::open(http_config).await.unwrap();
    let c = rusqlite::Connection::open(&f.config.database).unwrap();
    c.execute_batch("CREATE TRIGGER hold_first_writer BEFORE INSERT ON journal WHEN NEW.kind='native_data_write' BEGIN SELECT RAISE(ABORT,'controlled pending outcome'); END; CREATE TRIGGER hold_cancelled_queue_receipt BEFORE INSERT ON journal WHEN json_extract(NEW.data_json,'$.operation_id')='cancel-queued-data' AND json_extract(NEW.data_json,'$.state') IN ('committing','completed') BEGIN SELECT RAISE(ABORT,'controlled queue recording fault'); END;").unwrap();
    publish(
        &mut source,
        &cid,
        "first-queued-data",
        "first-queued-message",
        &pid,
        "{}",
    )
    .await;
    assert_eq!(
        target.receive_control().await.unwrap().original()["payload"]["msg_id"],
        "first-queued-message"
    );
    publish_state(
        &mut source,
        &cid,
        "cancel-queued-data",
        "cancel-queued-message",
        &pid,
        "{}",
        "preparing",
    )
    .await;
    let end = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        if c.query_row("SELECT COUNT(*) FROM native_data_writes WHERE operation_record_id=(SELECT record_id FROM operations WHERE operation_id='cancel-queued-data') AND disposition IS NULL",[],|r|r.get::<_,u64>(0)).unwrap()==1 {break;}
        assert!(
            tokio::time::Instant::now() < end,
            "second message did not reach its native writer queue"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    canceller
        .request(
            payload(
                "operation.cancel",
                json!({"operation_id":"cancel-queued-data"}),
            ),
            CommandScope {
                conversation_id: Some(cid.clone()),
                operation_id: Some("cancel-data-decision".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        c.query_row(
            "SELECT cancel_requested FROM operations WHERE operation_id='cancel-queued-data'",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap(),
        true
    );
    c.execute_batch("DROP TRIGGER hold_cancelled_queue_receipt; DROP TRIGGER hold_first_writer;")
        .unwrap();
    wait_native_operation(&f, &config.credential, &cid, "cancel-queued-data").await;
    let end = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        if c.query_row("SELECT COUNT(*) FROM native_data_writes WHERE operation_record_id=(SELECT record_id FROM operations WHERE operation_id='cancel-queued-data') AND disposition='not_started'",[],|r|r.get::<_,u64>(0)).unwrap()==1 {break;}
        assert!(
            tokio::time::Instant::now() < end,
            "cancelled queued message still blocks its port"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    publish(
        &mut source,
        &cid,
        "after-cancel-data",
        "after-cancel-message",
        &pid,
        "{}",
    )
    .await;
    assert_eq!(
        target.receive_control().await.unwrap().original()["payload"]["msg_id"],
        "after-cancel-message"
    );
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM native_data_messages", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        3
    );
    drop(c);
    drop(source);
    drop(target);
    drop(canceller);
    f.server.shutdown().await.unwrap();
}
