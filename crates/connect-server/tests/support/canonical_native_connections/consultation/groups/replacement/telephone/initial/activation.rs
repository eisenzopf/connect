//! Real canonical native peers, actual UDP SIP, and controlled metadata failure.
use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_mixed_session_activation_waits_for_actual_native_and_sip_readiness_with_exact_capabilities(
) {
    for algorithm in ["EdDSA", "ES256"] {
        activation(algorithm, 0, false).await;
        activation(algorithm, 8, false).await;
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_sip_last_ready_recovers_recording_without_redial_or_another_native_packet() {
    for algorithm in ["EdDSA", "ES256"] {
        activation(algorithm, 0, true).await;
    }
}
async fn activation(algorithm: &str, payload: u8, stall_connected: bool) {
    let ConfiguredInitial { f, peer, .. } =
        configured_initial_for(algorithm, Some(payload), 30).await;
    ready_endpoint(&f).await;
    let db = rusqlite::Connection::open(&f.config.database).unwrap();
    if stall_connected {
        db.execute_batch("CREATE TRIGGER reject_initial_connected BEFORE UPDATE OF connected_at ON initial_telephone_answers WHEN NEW.connected_at IS NOT NULL BEGIN SELECT RAISE(ABORT,'isolated connected record failure'); END;").unwrap();
    }
    let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut input =
        connect_client::conversations::CreateRequest::persistent(vec![f.peer.identity_id.clone()]);
    input.tenant_id = Some("tnt_default".into());
    let room = client
        .create_conversation("mixed-activation-room", input)
        .await
        .unwrap();
    let target = room
        .participants
        .iter()
        .find(|p| p.identity_id == f.peer.identity_id)
        .unwrap()
        .participant_id
        .clone();
    let request = connect_protocol::session_control::Request {
        operation_id: "mixed-activation-invite".into(),
        conversation_id: room.id.clone(),
        session_id: "native-session".into(),
        by_participant: None,
        expected_revision: None,
        action: connect_protocol::session_control::Action::Invite {
            participants: vec![target],
            medium: "voice".into(),
            intent: "call".into(),
            capabilities: json!({"audio":["g.711-mu","g.711-a"]}),
        },
    };
    let receipt = client.session_command(request.clone()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(12),async {
        loop {
            let ready:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM initial_telephone_answers WHERE accepted=1 AND (?1=1 OR connected_at IS NOT NULL))",[stall_connected],|r|r.get(0)).unwrap();
            if ready {break;}
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.unwrap();
    assert_eq!(phase(&db), "accepted");
    assert_eq!(
        starts(&db),
        0,
        "SIP alone cannot cover the accepted native Participant"
    );
    assert_eq!(peer.unique(), 1);
    let answer=crate::canonical_native_connections::negotiate_av(&mut client,&room.id,"mixed-native-offer","mixed-native",json!([
        {"id":"mixed-native-voice","kind":"audio","direction":"sendonly","codec_preferences":["g.711-mu"]}
    ])).await;
    wait_native_operation(&f, &f.client.credential, &room.id, "mixed-native-offer").await;
    assert_eq!(
        phase(&db),
        "accepted",
        "native negotiation alone is not ready flow"
    );
    let handle = answer.original()["payload"]["substrate_setup"]["streams"][0]["stream_local_id"]
        .as_u64()
        .unwrap() as u16;
    crate::canonical_native_connections::participants::flow(&client, handle, 1).await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while db
            .query_row(
                "SELECT COUNT(*) FROM native_connection_flow_facts",
                [],
                |r| r.get::<_, u64>(0),
            )
            .unwrap()
            != 1
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    if stall_connected {
        assert_eq!(phase(&db),"accepted","actual SIP admission with an unrecorded ready observation cannot be fabricated from provisional metadata");
        assert_eq!(starts(&db), 0);
        assert_eq!(db.query_row("SELECT COUNT(*) FROM initial_telephone_answers WHERE accepted=1 AND connected_at IS NULL",[],|r|r.get::<_,u64>(0)).unwrap(),1);
        db.execute_batch("DROP TRIGGER reject_initial_connected;")
            .unwrap();
        // The existing SIP controller and ready observation must be recorded.
        // Send no further native datagrams and never request another SIP call.
    }
    tokio::time::timeout(Duration::from_secs(8), async {
        while phase(&db) != "active" {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let started = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let frame = client.receive().await.unwrap();
            if frame.kind() == "session.started" {
                break frame;
            }
            assert!(
                matches!(
                    frame.kind(),
                    "session.participant.joined"
                        | "connection.ready"
                        | "stream.opened"
                        | "connection.quality"
                        | "recording.vcon-ready"
                ),
                "{:?}",
                frame.original()
            );
        }
    })
    .await
    .unwrap();
    assert_eq!(started.original()["signature"]["alg"], algorithm);
    let fact = f
        .server
        .store
        .session_started_template(&room.id, "native-session")
        .unwrap()
        .0;
    assert_eq!(fact.participants_present.len(), 2);
    assert_eq!(fact.active_connections.len(), 2);
    assert_eq!(
        fact.active_connections
            .iter()
            .map(|c| c.transport.as_str())
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from(["quic", "sip-interop"])
    );
    let caps = &fact.negotiated_capabilities;
    if payload == 0 {
        assert_eq!(caps["audio_codecs"].as_array().unwrap().len(), 1);
        assert_eq!(caps["audio_codecs"][0]["name"], "g.711-mu");
    } else {
        assert_eq!(
            caps["audio_codecs"],
            json!([]),
            "different selected formats cannot be reported as a common format"
        );
        let exact = caps["connections"].as_array().unwrap();
        assert_eq!(exact.len(), 2);
        assert_eq!(
            exact
                .iter()
                .map(|c| c["capabilities"]["audio_codecs"][0]["name"]
                    .as_str()
                    .unwrap())
                .collect::<std::collections::BTreeSet<_>>(),
            std::collections::BTreeSet::from(["g.711-mu", "g.711-a"])
        );
    }
    assert_eq!(client.session_command(request).await.unwrap(), receipt);
    assert_eq!(
        f.server
            .store
            .session_started_template(&room.id, "native-session")
            .unwrap()
            .0,
        fact
    );
    assert_eq!(starts(&db), 1);
    assert_eq!(peer.unique(), 1);
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM initial_telephone_operations WHERE signaling_started=1",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(db.query_row("SELECT COUNT(*) FROM operation_effects WHERE provider='sip-initial' AND attempt=1 AND state='confirmed'",[],|r|r.get::<_,u64>(0)).unwrap(),1);
    if let Some(path) = std::env::var_os("CONNECT_G6_MIXED_ACTIVATION_SAMPLES") {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        writeln!(file,"{}",json!({"algorithm":algorithm,"sip_payload":payload,"sip_last_storage_recovery":stall_connected,"native_packets_after_sip_recording_released":0,"unique_sip_calls":peer.unique(),"started":started.original()})).unwrap();
    }
    client.close();
    f.server.shutdown().await.unwrap();
    assert!(peer.methods.lock().unwrap().contains(&Method::Bye));
}
fn phase(db: &rusqlite::Connection) -> String {
    db.query_row(
        "SELECT state FROM sessions WHERE id='native-session'",
        [],
        |r| r.get(0),
    )
    .unwrap()
}
fn starts(db: &rusqlite::Connection) -> u64 {
    db.query_row("SELECT COUNT(*) FROM session_started_facts", [], |r| {
        r.get(0)
    })
    .unwrap()
}
