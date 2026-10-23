//! Actual signed native runtime controls, audio directionality and receipt custody.
use super::*;
use codecs::Running;

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
async fn command(
    s: &mut Running,
    operation: &str,
    revision: u64,
    input: Value,
    state: &str,
) -> Value {
    let id = s.clients[1]
        .send_value(
            "connection.update",
            input.clone(),
            scope(&s.cid, operation, revision),
        )
        .await
        .unwrap();
    let mut result = None;
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(8), s.clients[1].receive_control())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(frame.original()["in_reply_to"], id);
        assert_eq!(frame.original()["cid"], s.cid);
        assert_eq!(frame.original()["sid"], "native-session");
        assert_eq!(frame.original()["connid"], "codec-target");
        if let Some(path) = std::env::var_os("CONNECT_G6_GATE_SAMPLES") {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .unwrap();
            writeln!(file, "{}", json!({"algorithm":s.f.client.signing_algorithm,"operation_id":operation,"expected_revision":revision,"input":input,"frame":frame.original()})).unwrap();
        }
        match frame.kind() {
            "operation.updated" => {
                assert_eq!(
                    frame.original()["payload"]["state"],
                    state,
                    "{:?}",
                    frame.original()
                );
                result = Some(frame.original()["payload"].clone());
            }
            "ack" => return result.unwrap(),
            _ => panic!(
                "runtime controls must not negotiate or accept unrelated replies: {:?}",
                frame.original()
            ),
        }
    }
}
async fn changed(s: &mut Running, operation: &str, revision: u64, input: Value) -> Value {
    let response = command(s, operation, revision, input.clone(), "completed").await;
    let result = &response["result"];
    assert_eq!(result["resources_changed"], false);
    assert_eq!(result["stream_revision"], revision + 1);
    assert_eq!(result["connection_generation"], 1);
    let streams: Vec<connect_protocol::native_gates::GateStream> =
        serde_json::from_value(result["gate_streams"].clone()).unwrap();
    connect_protocol::native_gates::validate_observation(&input, &streams).unwrap();
    assert_eq!(streams.len(), 1);
    assert_eq!(streams[0].binding.stream_id, "codec-monitor");
    assert_eq!(streams[0].binding.stream_local_id, s.th);
    assert_eq!(streams[0].binding.stream_generation, 1);
    assert_eq!(streams[0].binding.codec["name"], "pcmu");
    assert_eq!(
        s.f.server
            .service
            .canonical_runtime_snapshot()
            .unwrap()
            .connections,
        2
    );
    assert_eq!(readiness(&s.f)["last_gate"], *result);
    response
}
async fn audible(s: &mut Running, source_index: usize, sequence: u32) {
    let (left, right) = s.clients.split_at_mut(1);
    if source_index == 0 {
        codecs::decoded_audio(&left[0], &mut right[0], s.sh, 0, s.th, 0, sequence).await;
    } else {
        codecs::decoded_audio(&right[0], &mut left[0], s.th, 0, s.sh, 0, sequence).await;
    }
}
async fn silent(s: &mut Running, source_index: usize, sequence: u32) {
    silent_format(s, source_index, sequence, 0).await;
}
async fn silent_format(s: &mut Running, source_index: usize, sequence: u32, source_pt: u8) {
    use rvoip_uctp::substrate::{
        pack_rtp_datagram, unpack_rtp_datagram, RtpDatagram, RtpMediaPayload,
    };
    let (left, right) = s.clients.split_at_mut(1);
    let (sender, receiver, sh, th) = if source_index == 0 {
        (&left[0], &mut right[0], s.sh, s.th)
    } else {
        (&right[0], &mut left[0], s.th, s.sh)
    };
    let sending = async {
        for seq in sequence..sequence + 60 {
            sender
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
                            timestamp: seq.wrapping_mul(160),
                            ssrc: 0x6463_7001,
                        },
                    })
                    .unwrap(),
                )
                .unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    let receiving = async {
        let start = tokio::time::Instant::now();
        while start.elapsed() < Duration::from_millis(1300) {
            match tokio::time::timeout(
                Duration::from_millis(40),
                receiver.receive_native_rtp_datagram(),
            )
            .await
            {
                Ok(Ok(bytes)) => {
                    let packet = unpack_rtp_datagram(&bytes).unwrap();
                    assert_eq!(packet.stream_local_id, th);
                    if start.elapsed() >= Duration::from_millis(300) {
                        assert!(
                            packet.rtp.payload.iter().all(|v| *v == 0xff || *v == 0x7f),
                            "gated source crossed the actual route"
                        );
                    }
                }
                Ok(Err(error)) => panic!("gate retired a healthy receiver: {error:?}"),
                Err(_) => (),
            }
        }
    };
    tokio::join!(sending, receiving);
}
async fn setup(algorithm: &str) -> Running {
    let f = Fixture::with_native_end_scope(algorithm, 12).await;
    codecs::setup_started(f, (), "gate-target-offer", "gate-source-offer")
        .await
        .0
}
#[tokio::test]
async fn production_native_mute_survives_codec_replacement_and_old_receipt_cannot_reset_new_binding(
) {
    let mut s = setup("EdDSA").await;
    audible(&mut s, 0, 30000).await;
    audible(&mut s, 1, 30200).await;
    let input = json!({"action":"mute","streams":["codec-monitor"]});
    let original = command(&mut s, "gate-codec-mute", 2, input.clone(), "completed").await;
    let codec_scope = scope(&s.cid, "gate-codec-pcma", 3);
    s.clients[1].send_value("connection.update", json!({"action":"codec-renegotiate","streams":["codec-monitor"],"codec_preferences":["pcma"]}), codec_scope.clone()).await.unwrap();
    let offer = s.clients[1].receive_control().await.unwrap();
    assert_eq!(offer.kind(), "connection.offer");
    let handle = offer.original()["payload"]["substrate_setup"]["streams"][0]["stream_local_id"]
        .as_u64()
        .unwrap() as u16;
    assert_ne!(handle, s.th);
    codecs::answer(&mut s.clients[1], &offer, &codec_scope, false).await;
    let codec = codecs::completed(&mut s.clients[1], "completed").await;
    assert_eq!(codec.original()["payload"]["result"]["stream_revision"], 4);
    s.th = handle;
    silent_format(&mut s, 1, 30400, 8).await;
    {
        let (source, target) = s.clients.split_at_mut(1);
        codecs::decoded_audio(&source[0], &mut target[0], s.sh, 0, s.th, 8, 30600).await;
    }
    let unmuted = command(
        &mut s,
        "gate-codec-unmute",
        4,
        json!({"action":"unmute","streams":["codec-monitor"]}),
        "completed",
    )
    .await;
    assert_eq!(
        unmuted["result"]["gate_streams"][0]["binding"]["codec"]["name"],
        "pcma"
    );
    assert_eq!(
        unmuted["result"]["gate_streams"][0]["binding"]["stream_local_id"],
        handle
    );
    assert_eq!(
        unmuted["result"]["gate_streams"][0]["binding"]["stream_generation"],
        2
    );
    assert_eq!(
        command(&mut s, "gate-codec-mute", 2, input, "completed").await,
        original
    );
    assert_eq!(readiness(&s.f)["last_gate"], unmuted["result"]);
    {
        let (source, target) = s.clients.split_at_mut(1);
        codecs::decoded_audio(&target[0], &mut source[0], s.th, 8, s.sh, 0, 30800).await;
    }
    if let Some(path) = std::env::var_os("CONNECT_G6_GATE_CODEC_SAMPLE") {
        std::fs::write(path, serde_json::to_vec_pretty(&json!({"offer":offer.original(),"codec_completion":codec.original(),"mute":original,"unmute":unmuted,"decoded_after_unmute":true})).unwrap()).unwrap();
    }
    for client in &mut s.clients {
        client.close();
    }
    s.f.server.shutdown().await.unwrap();
}
#[tokio::test]
async fn production_native_eddsa_hold_resume_mute_are_exact_current_controls_with_real_audio() {
    normal("EdDSA").await;
}
#[tokio::test]
async fn production_native_es256_hold_resume_mute_are_exact_current_controls_with_real_audio() {
    normal("ES256").await;
}
async fn normal(algorithm: &str) {
    let mut s = setup(algorithm).await;
    audible(&mut s, 0, 10000).await;
    audible(&mut s, 1, 10200).await;
    let mute = json!({"action":"mute","streams":["codec-monitor"],"future":{"exact":true}});
    let original = changed(&mut s, "gate-mute", 2, mute.clone()).await;
    silent(&mut s, 1, 10400).await;
    audible(&mut s, 0, 10600).await;
    let mut foreign = CanonicalClient::open(s.f.client.clone()).await.unwrap();
    foreign
        .send_value(
            "connection.update",
            mute.clone(),
            scope(&s.cid, "gate-foreign", 3),
        )
        .await
        .unwrap();
    assert_eq!(foreign.receive_control().await.unwrap().kind(), "error");
    foreign.close();
    let c = rusqlite::Connection::open(&s.f.config.database).unwrap();
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM operations WHERE operation_id='gate-foreign'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    changed(
        &mut s,
        "gate-unmute",
        3,
        json!({"action":"unmute","streams":["codec-monitor"]}),
    )
    .await;
    assert_eq!(
        command(&mut s, "gate-mute", 2, mute.clone(), "completed").await,
        original
    );
    assert_eq!(readiness(&s.f)["stream_revision"], 4);
    audible(&mut s, 1, 10800).await;
    changed(&mut s, "gate-hold-receive", 4, json!({"action":"hold"})).await;
    silent(&mut s, 1, 11000).await;
    audible(&mut s, 0, 11200).await;
    changed(&mut s, "gate-resume", 5, json!({"action":"resume"})).await;
    audible(&mut s, 1, 11400).await;
    changed(
        &mut s,
        "gate-hold-send",
        6,
        json!({"action":"hold","details":{"direction":"sendonly"}}),
    )
    .await;
    silent(&mut s, 0, 11600).await;
    audible(&mut s, 1, 11800).await;
    changed(
        &mut s,
        "gate-inactive",
        7,
        json!({"action":"hold","details":{"direction":"inactive"}}),
    )
    .await;
    silent(&mut s, 0, 12000).await;
    silent(&mut s, 1, 12200).await;
    changed(
        &mut s,
        "gate-muted-paused",
        8,
        json!({"action":"mute","streams":["codec-monitor"]}),
    )
    .await;
    let resumed = changed(
        &mut s,
        "gate-resume-still-muted",
        9,
        json!({"action":"resume"}),
    )
    .await;
    assert_eq!(resumed["result"]["gate_streams"][0]["source_muted"], true);
    silent(&mut s, 1, 12400).await;
    audible(&mut s, 0, 12600).await;
    changed(
        &mut s,
        "gate-unmute-restored",
        10,
        json!({"action":"unmute","streams":["codec-monitor"]}),
    )
    .await;
    audible(&mut s, 1, 12800).await;
    for (operation, revision, input) in [
        ("gate-stale", 10, json!({"action":"hold"})),
        (
            "gate-unknown",
            11,
            json!({"action":"mute","streams":["foreign-stream"]}),
        ),
    ] {
        command(&mut s, operation, revision, input, "failed").await;
        assert_eq!(c.query_row("SELECT COUNT(*) FROM operation_effects e JOIN operations o ON o.record_id=e.operation_record_id WHERE o.operation_id=?1", [operation], |r| r.get::<_,u64>(0)).unwrap(), 0);
    }
    let mut http = CanonicalHttpClient::open(canonical_http_config(&s.f))
        .await
        .unwrap();
    let batch = http
        .request_value("connection.update", mute, scope(&s.cid, "gate-http", 11))
        .await;
    let error = match batch {
        Ok(batch) => batch.ack().unwrap_err(),
        Err(error) => error,
    };
    assert!(
        matches!(error, ClientError::Application(e) if matches!(e.code.as_str(), "unsupported-media-profile"|"unsupported-command"))
    );
    audible(&mut s, 0, 13000).await;
    audible(&mut s, 1, 13200).await;
    for client in &mut s.clients {
        client.close();
    }
    s.f.server.shutdown().await.unwrap();
}
#[tokio::test]
async fn production_native_gate_captured_receipt_recovers_after_storage_failure_writer_and_cancel()
{
    let f = Fixture::with_extra_scopes("EdDSA", 12, true, true, true, &["operation.cancel"]).await;
    let (mut s, ()) = codecs::setup_started(f, (), "gate-target-offer", "gate-source-offer").await;
    audible(&mut s, 0, 20000).await;
    audible(&mut s, 1, 20200).await;
    let c = rusqlite::Connection::open(&s.f.config.database).unwrap();
    c.execute_batch("CREATE TRIGGER controlled_gate_recovery BEFORE INSERT ON journal WHEN json_extract(NEW.data_json,'$.operation_id')='gate-recover' AND json_extract(NEW.data_json,'$.state')='completed' BEGIN SELECT RAISE(ABORT,'controlled gate receipt failure'); END;").unwrap();
    let input = json!({"action":"mute","streams":["codec-monitor"]});
    command(&mut s, "gate-recover", 2, input.clone(), "preparing").await;
    assert_eq!(readiness(&s.f)["stream_revision"], 2);
    silent(&mut s, 1, 20400).await;
    audible(&mut s, 0, 20600).await;
    let pid: String = c
        .query_row(
            "SELECT participant_id FROM connections WHERE id='codec-target'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    tokio::time::timeout(
        Duration::from_secs(2),
        s.clients[1].session_command(Request {
            operation_id: "role-during-gate-recovery".into(),
            conversation_id: s.cid.clone(),
            session_id: "native-session".into(),
            by_participant: None,
            expected_revision: Some(1),
            action: Action::Role {
                participant_id: pid,
                role: "speaker".into(),
            },
        }),
    )
    .await
    .unwrap()
    .unwrap();
    let mut http = CanonicalHttpClient::open(canonical_http_config(&s.f))
        .await
        .unwrap();
    http.request_value(
        "operation.cancel",
        json!({"operation_id":"gate-recover"}),
        CommandScope {
            conversation_id: Some(s.cid.clone()),
            operation_id: Some("gate-recovery-cancel".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .ack()
    .unwrap();
    c.execute_batch("DROP TRIGGER controlled_gate_recovery")
        .unwrap();
    wait_native_operation(&s.f, &s.f.client.credential, &s.cid, "gate-recover").await;
    let original = command(&mut s, "gate-recover", 2, input.clone(), "completed").await;
    assert!(c
        .query_row(
            "SELECT cancel_requested FROM operations WHERE operation_id='gate-recover'",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap());
    assert_eq!(original["result"]["stream_revision"], 3);
    assert_eq!(readiness(&s.f)["last_gate"], original["result"]);
    assert_eq!(c.query_row("SELECT COUNT(*) FROM operation_effects e JOIN operations o ON o.record_id=e.operation_record_id WHERE o.operation_id='gate-recover'", [], |r| r.get::<_,u64>(0)).unwrap(), 1);
    changed(
        &mut s,
        "gate-recovery-unmute",
        3,
        json!({"action":"unmute","streams":["codec-monitor"]}),
    )
    .await;
    assert_eq!(
        command(&mut s, "gate-recover", 2, input, "completed").await,
        original
    );
    audible(&mut s, 1, 20800).await;
    for client in &mut s.clients {
        client.close();
    }
    s.f.server.shutdown().await.unwrap();
}
