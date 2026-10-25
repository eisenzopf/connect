//! Actual public ready/loss aggregation; no injected lifecycle or flow rows.
use super::*;
use rvoip_uctp::substrate::{pack_rtp_datagram, RtpDatagram, RtpMediaPayload};
async fn media(client: &mut CanonicalClient, cid: &str, name: &str) -> u16 {
    let answer=negotiate_av(client,cid,&format!("{name}-offer"),name,json!([{"id":format!("{name}-voice"),"kind":"audio","direction":"sendonly","codec_preferences":["g.711-mu"]}])).await;
    answer.original()["payload"]["substrate_setup"]["streams"][0]["stream_local_id"]
        .as_u64()
        .unwrap() as u16
}
pub(super) async fn flow(client: &CanonicalClient, handle: u16, start: u32) {
    for seq in start..start + 30 {
        client
            .send_native_media_datagram(
                &pack_rtp_datagram(&RtpDatagram {
                    flags: 0,
                    stream_local_id: handle,
                    seq,
                    rtp: RtpMediaPayload {
                        payload: vec![0x80; 160].into(),
                        payload_type: 0,
                        sequence_number: seq as u16,
                        timestamp: seq * 160,
                        ssrc: 0x47360026,
                    },
                })
                .unwrap(),
            )
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
// Every accepted Participant supplies real readiness before initial activation.
// The independent Device can then end its Connection; logical presence remains.
pub(super) async fn cover_accepted_peer(
    f: &Fixture,
    cid: &str,
    observer: Option<&mut CanonicalClient>,
) -> CanonicalClient {
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope.operations.extend([
        "stream.subscribe".into(),
        "stream.unsubscribe".into(),
        "connection.end".into(),
    ]);
    let config = party_with_scope(f, &f.peer.identity_id, 237, vec!["audio".into()], scope);
    let mut client = CanonicalClient::open(config.clone()).await.unwrap();
    let handle = media(&mut client, cid, "accepted-peer-bootstrap").await;
    wait_native_operation(f, &config.credential, cid, "accepted-peer-bootstrap-offer").await;
    flow(&client, handle, 1).await;
    if let Some(observer) = observer {
        let joined = peer_event(observer, "session.participant.joined", cid).await;
        assert_eq!(
            joined.original()["payload"]["via_connection"],
            "accepted-peer-bootstrap"
        );
        assert_eq!(
            joined.original()["payload"]["participant"]["identity_id"],
            f.peer.identity_id
        );
    }
    client
}
pub(super) async fn end_coverage(
    client: &mut CanonicalClient,
    cid: &str,
    observer: Option<&mut CanonicalClient>,
) {
    end_native_connection(
        client,
        cid,
        "accepted-peer-bootstrap",
        "accepted-peer-bootstrap-end",
        None,
        "completed",
    )
    .await;
    if let Some(observer) = observer {
        peer_event(observer, "session.participant.left", cid).await;
    }
    client.close();
}
async fn peer_event(peer: &mut CanonicalClient, kind: &str, cid: &str) -> Envelope {
    let event = peer.receive().await.unwrap();
    assert_eq!(event.kind(), kind, "{:?}", event.original());
    assert_eq!(event.original()["cid"], cid);
    assert_eq!(event.original()["sid"], "native-session");
    for absent in ["connid", "operation_id", "in_reply_to"] {
        assert!(event.original().get(absent).is_none());
    }
    event
}
async fn quiet(peer: &mut CanonicalClient) {
    let request = peer.keepalive().await.unwrap();
    let ack = loop {
        let event = peer.receive().await.unwrap();
        if event.kind() == "recording.vcon-ready" {
            assert!(super::lifecycle::session_start(&event));
            continue;
        }
        break event;
    };
    assert_eq!(
        ack.kind(),
        "ack",
        "unexpected duplicate presence event: {:?}",
        ack.original()
    );
    assert_eq!(ack.original()["in_reply_to"], request);
}
#[tokio::test]
async fn production_participant_presence_joins_once_across_connections_and_leaves_only_last_then_rejoins_same_session(
) {
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_native_end_scope(algorithm, 12).await;
        ready_endpoint(&f).await;
        let mut source = CanonicalClient::open(f.client.clone()).await.unwrap();
        let cid = session(&f, &mut source).await;
        let config = party_with_scope(
            &f,
            &f.peer.identity_id,
            126,
            vec!["audio".into()],
            Scope::collaboration(vec!["*".into()]),
        );
        let mut peer = CanonicalClient::open(config).await.unwrap();
        let baseline = source
            .conversation_snapshot(&cid, serde_json::from_value(json!({})).unwrap())
            .await
            .unwrap()
            .cursor;
        let db = rusqlite::Connection::open(&f.config.database).unwrap();
        let mut coverage = cover_accepted_peer(&f, &cid, Some(&mut peer)).await;
        let first = media(&mut source, &cid, "presence-first").await;
        quiet(&mut peer).await;
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM session_participant_facts WHERE participant_id=(SELECT participant_id FROM participants WHERE identity_id=?1 AND active=1)", [&f.client.identity_id], |r| {
                r.get::<_, u64>(0)
            })
            .unwrap(),
            0
        );
        flow(&source, first, 1).await;
        let started = peer_event(&mut peer, "session.started", &cid).await;
        let joined = peer_event(&mut peer, "session.participant.joined", &cid).await;
        end_coverage(&mut coverage, &cid, Some(&mut peer)).await;
        let participant = joined.original()["payload"]["participant"].clone();
        let pid = participant["participant_id"].as_str().unwrap().to_owned();
        assert_eq!(participant["identity_id"], f.client.identity_id);
        assert_eq!(participant["kind"], "human");
        assert!(!participant["display_name"].as_str().unwrap().is_empty());
        assert_eq!(
            joined.original()["payload"]["via_connection"],
            "presence-first"
        );
        let original:String=db.query_row("SELECT fact_json FROM session_participant_facts WHERE kind='session_participant_joined' AND participant_id=(SELECT participant_id FROM participants WHERE identity_id=?1 AND active=1)",[&f.client.identity_id],|r|r.get(0)).unwrap();
        let original_id = serde_json::from_str::<Value>(&original).unwrap()["data"]["fact_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let second = media(&mut source, &cid, "presence-second").await;
        flow(&source, second, 101).await;
        quiet(&mut peer).await;
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM session_participant_facts WHERE participant_id=(SELECT participant_id FROM participants WHERE identity_id=?1 AND active=1)", [&f.client.identity_id], |r| {
                r.get::<_, u64>(0)
            })
            .unwrap(),
            1
        );
        end_native_connection(
            &mut source,
            &cid,
            "presence-first",
            "presence-end-first",
            None,
            "completed",
        )
        .await;
        wait_slots(&f, 1).await;
        quiet(&mut peer).await;
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM session_participant_facts WHERE participant_id=(SELECT participant_id FROM participants WHERE identity_id=?1 AND active=1)", [&f.client.identity_id], |r| {
                r.get::<_, u64>(0)
            })
            .unwrap(),
            1
        );
        end_native_connection(
            &mut source,
            &cid,
            "presence-second",
            "presence-end-second",
            None,
            "completed",
        )
        .await;
        wait_slots(&f, 0).await;
        let left = peer_event(&mut peer, "session.participant.left", &cid).await;
        assert_eq!(left.original()["payload"]["participant_id"], pid);
        assert_eq!(
            left.original()["payload"]["reason"],
            "all-connections-ended"
        );
        let third = media(&mut source, &cid, "presence-third").await;
        flow(&source, third, 201).await;
        let rejoined = peer_event(&mut peer, "session.participant.joined", &cid).await;
        assert_eq!(
            rejoined.original()["payload"]["participant"]["participant_id"],
            pid
        );
        assert_eq!(
            rejoined.original()["payload"]["via_connection"],
            "presence-third"
        );
        assert_ne!(joined.id(), rejoined.id());
        let leave = Request {
            operation_id: "presence-explicit-leave".into(),
            conversation_id: cid.clone(),
            session_id: "native-session".into(),
            by_participant: None,
            expected_revision: Some(1),
            action: Action::Leave {
                participant_id: pid.clone(),
            },
        };
        let receipt = source.session_command(leave.clone()).await.unwrap();
        let explicit = peer_event(&mut peer, "session.participant.left", &cid).await;
        assert_eq!(explicit.original()["payload"]["reason"], "explicit-leave");
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM session_started_facts", [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            db.query_row(
                "SELECT fact_json FROM session_participant_facts WHERE fact_id=?1",
                [&original_id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            original
        );
        let mut http = CanonicalHttpClient::open(canonical_http_config(&f))
            .await
            .unwrap();
        let catchup = http
            .subscribe_conversation(
                &cid,
                serde_json::from_value(json!({"after":baseline,"live":false})).unwrap(),
            )
            .await
            .unwrap();
        catchup.ack().unwrap();
        for frame in &catchup.frames {
            if matches!(
                frame.kind(),
                "session.participant.joined" | "session.participant.left"
            ) {
                assert!(
                    [joined.id(), left.id(), rejoined.id(), explicit.id()]
                        .iter()
                        .all(|id| *id != frame.id()),
                    "each catch-up envelope needs a fresh ID while its fact remains stable"
                );
            }
        }
        assert_eq!(source.session_command(leave).await.unwrap(), receipt);
        quiet(&mut peer).await;
        let kinds = catchup
            .frames
            .iter()
            .filter(|e| {
                e.original()["payload"]["participant"]["participant_id"] == pid
                    || e.original()["payload"]["participant_id"] == pid
            })
            .filter(|e| {
                matches!(
                    e.kind(),
                    "session.participant.joined" | "session.participant.left"
                )
            })
            .map(|e| e.kind())
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec![
                "session.participant.joined",
                "session.participant.left",
                "session.participant.joined",
                "session.participant.left"
            ]
        );
        let data = catchup
            .frames
            .iter()
            .filter(|e| e.kind() == "conversation.event")
            .filter_map(|e| {
                let event: connect_protocol::events::JournalEvent =
                    serde_json::from_value(e.original()["payload"]["data"].clone()).unwrap();
                if match &event.data {
                    connect_protocol::events::JournalData::SessionParticipantJoined(fact) => {
                        fact.participant.participant_id == pid
                    }
                    connect_protocol::events::JournalData::SessionParticipantLeft(fact) => {
                        fact.participant_id == pid
                    }
                    _ => false,
                } {
                    Some(event)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(data.len(), 4);
        if let Some(directory) = std::env::var_os("CONNECT_G6_PARTICIPANT_SAMPLES") {
            let directory = std::path::PathBuf::from(directory);
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join(format!("{algorithm}-participants.json")),serde_json::to_vec_pretty(&json!({"algorithm":algorithm,"started":started.original(),"joined":joined.original(),"left":left.original(),"rejoined":rejoined.original(),"explicit_left":explicit.original(),"journal":data})).unwrap()).unwrap();
        }
        source.close();
        peer.close();
        f.server.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn production_reconnect_grace_preserves_same_session_ignores_silence_and_explicit_end_is_immediate(
) {
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_native_end_scope(algorithm, 12).await;
        f.server.store.configure_session_reconnect_grace(2).unwrap();
        ready_endpoint(&f).await;
        let mut source = CanonicalClient::open(f.client.clone()).await.unwrap();
        let cid = session(&f, &mut source).await;
        let mut peer = CanonicalClient::open(party_with_scope(
            &f,
            &f.peer.identity_id,
            126,
            vec!["audio".into()],
            Scope::collaboration(vec!["*".into()]),
        ))
        .await
        .unwrap();
        let baseline = source
            .conversation_snapshot(&cid, serde_json::from_value(json!({})).unwrap())
            .await
            .unwrap()
            .cursor;
        let mut coverage = cover_accepted_peer(&f, &cid, Some(&mut peer)).await;
        let first = media(&mut source, &cid, "grace-first").await;
        flow(&source, first, 1).await;
        peer_event(&mut peer, "session.started", &cid).await;
        peer_event(&mut peer, "session.participant.joined", &cid).await;
        end_coverage(&mut coverage, &cid, Some(&mut peer)).await;
        tokio::time::sleep(Duration::from_millis(2200)).await;
        quiet(&mut peer).await;
        let db = rusqlite::Connection::open(&f.config.database).unwrap();
        assert_eq!(
            db.query_row(
                "SELECT state FROM sessions WHERE id='native-session'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "active"
        );
        assert!(db.query_row("SELECT deadline_millis FROM session_reconnect_boundaries WHERE session_id='native-session'",[],|r|r.get::<_,Option<i64>>(0)).unwrap().is_none());
        end_native_connection(
            &mut source,
            &cid,
            "grace-first",
            "grace-end-first",
            None,
            "completed",
        )
        .await;
        wait_slots(&f, 0).await;
        peer_event(&mut peer, "session.participant.left", &cid).await;
        let deadline:i64=db.query_row("SELECT deadline_millis FROM session_reconnect_boundaries WHERE session_id='native-session'",[],|r|r.get(0)).unwrap();
        assert!(deadline > chrono::Utc::now().timestamp_millis());
        let second = media(&mut source, &cid, "grace-second").await;
        flow(&source, second, 101).await;
        peer_event(&mut peer, "session.participant.joined", &cid).await;
        assert!(db.query_row("SELECT deadline_millis FROM session_reconnect_boundaries WHERE session_id='native-session'",[],|r|r.get::<_,Option<i64>>(0)).unwrap().is_none());
        tokio::time::sleep(Duration::from_millis(2200)).await;
        quiet(&mut peer).await;
        let end = Request {
            operation_id: "grace-explicit-end".into(),
            conversation_id: cid.clone(),
            session_id: "native-session".into(),
            by_participant: None,
            expected_revision: Some(1),
            action: Action::End {
                reason_code: 486,
                reason: "explicit-completion".into(),
            },
        };
        let start = tokio::time::Instant::now();
        let receipt = source.session_command(end.clone()).await.unwrap();
        peer_event(&mut peer, "session.participant.left", &cid).await;
        let ended = peer_event(&mut peer, "session.ended", &cid).await;
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "explicit end must not wait for reconnect grace"
        );
        assert_eq!(ended.original()["payload"]["reason"], "explicit-completion");
        assert_eq!(ended.original()["payload"]["reason_code"], 486);
        let terminal: String = db
            .query_row(
                "SELECT fact_json FROM session_terminal_facts WHERE session_id='native-session'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let fact: connect_protocol::events::SessionEnded = serde_json::from_str(&terminal).unwrap();
        assert_eq!(
            ended.original()["payload"]["by"],
            json!(fact.by_participant)
        );
        assert!(db.query_row("SELECT deadline_millis FROM session_reconnect_boundaries WHERE session_id='native-session'",[],|r|r.get::<_,Option<i64>>(0)).unwrap().is_none());
        assert_eq!(source.session_command(end).await.unwrap(), receipt);
        quiet(&mut peer).await;
        let mut http = CanonicalHttpClient::open(canonical_http_config(&f))
            .await
            .unwrap();
        let catchup = http
            .subscribe_conversation(
                &cid,
                serde_json::from_value(json!({"after":baseline,"live":false})).unwrap(),
            )
            .await
            .unwrap();
        catchup.ack().unwrap();
        let typed = catchup
            .frames
            .iter()
            .filter(|e| e.kind() == "session.ended")
            .collect::<Vec<_>>();
        assert_eq!(typed.len(), 1);
        assert_eq!(typed[0].original()["payload"], ended.original()["payload"]);
        assert_ne!(typed[0].id(), ended.id());
        let journal = catchup
            .frames
            .iter()
            .filter(|e| e.kind() == "conversation.event")
            .filter_map(|e| {
                let event: connect_protocol::events::JournalEvent =
                    serde_json::from_value(e.original()["payload"]["data"].clone()).unwrap();
                if matches!(
                    event.data,
                    connect_protocol::events::JournalData::SessionEnded(_)
                ) {
                    Some(event)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(journal.len(), 1);
        assert_eq!(
            journal[0].data,
            connect_protocol::events::JournalData::SessionEnded(fact)
        );
        if let Some(directory) = std::env::var_os("CONNECT_G6_RECONNECT_SAMPLES") {
            let directory = std::path::PathBuf::from(directory);
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join(format!("{algorithm}-session-reconnect.json")),serde_json::to_vec_pretty(&json!({"algorithm":algorithm,"configured_grace_seconds":2,"same_session":"native-session","ended":ended.original(),"journal":journal,"catchup":catchup.frames.iter().map(|e|e.original()).collect::<Vec<_>>()})).unwrap()).unwrap();
        }
        source.close();
        peer.close();
        f.server.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn production_default_thirty_second_loss_grace_ends_once_and_requires_new_invitation() {
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_native_end_scope(algorithm, 12).await;
        ready_endpoint(&f).await;
        let mut source = CanonicalClient::open(f.client.clone()).await.unwrap();
        let cid = session(&f, &mut source).await;
        let mut peer = CanonicalClient::open(party_with_scope(
            &f,
            &f.peer.identity_id,
            126,
            vec!["audio".into()],
            Scope::collaboration(vec!["*".into()]),
        ))
        .await
        .unwrap();
        let mut coverage = cover_accepted_peer(&f, &cid, Some(&mut peer)).await;
        let first = media(&mut source, &cid, "default-grace-first").await;
        flow(&source, first, 1).await;
        peer_event(&mut peer, "session.started", &cid).await;
        peer_event(&mut peer, "session.participant.joined", &cid).await;
        end_coverage(&mut coverage, &cid, Some(&mut peer)).await;
        end_native_connection(
            &mut source,
            &cid,
            "default-grace-first",
            "default-grace-end-first",
            None,
            "completed",
        )
        .await;
        wait_slots(&f, 0).await;
        peer_event(&mut peer, "session.participant.left", &cid).await;
        let db = rusqlite::Connection::open(&f.config.database).unwrap();
        let (loss,deadline):(i64,i64)=db.query_row("SELECT lost_at_millis,deadline_millis FROM session_reconnect_boundaries WHERE session_id='native-session'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(deadline - loss, 30_000);
        while deadline - chrono::Utc::now().timestamp_millis() > 500 {
            let remaining = (deadline - chrono::Utc::now().timestamp_millis() - 500).max(0) as u64;
            tokio::time::sleep(Duration::from_millis(remaining.min(5000))).await;
            quiet(&mut peer).await;
            let request = source.keepalive().await.unwrap();
            let ack = source.receive_control().await.unwrap();
            assert_eq!(ack.kind(), "ack");
            assert_eq!(ack.original()["in_reply_to"], request);
        }
        assert_eq!(
            db.query_row(
                "SELECT state FROM sessions WHERE id='native-session'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "active"
        );
        quiet(&mut peer).await;
        let until = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            if db
                .query_row(
                    "SELECT state FROM sessions WHERE id='native-session'",
                    [],
                    |r| r.get::<_, String>(0),
                )
                .unwrap()
                == "ended"
            {
                break;
            }
            assert!(
                tokio::time::Instant::now() < until,
                "grace worker did not close the Session"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let ended = peer_event(&mut peer, "session.ended", &cid).await;
        assert_eq!(
            ended.original()["payload"]["reason"],
            "reconnect-grace-expired"
        );
        assert_eq!(ended.original()["payload"]["reason_code"], 408);
        assert!(ended.original()["payload"]["by"].is_null());
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM session_terminal_facts", [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM session_lifecycle_hints WHERE kind='session_ended'",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            1
        );
        let refused=source.send_value("connection.offer",json!({"substrate":"quic","capabilities":av_capabilities(),"streams_offered":[{"id":"late-audio","kind":"audio","direction":"sendonly","codec_preferences":["g.711-mu"]}]}),CommandScope {conversation_id:Some(cid.clone()),session_id:Some("native-session".into()),connection_id:Some("late-after-grace".into()),operation_id:Some("late-after-grace".into()),expected_revision:Some(0)}).await.unwrap();
        let failure = source.receive_control().await.unwrap();
        assert_eq!(failure.original()["in_reply_to"], refused);
        assert_eq!(failure.kind(), "operation.updated");
        assert_eq!(failure.original()["payload"]["state"], "failed");
        let ack = source.receive_control().await.unwrap();
        assert_eq!(ack.original()["in_reply_to"], refused);
        assert_eq!(ack.kind(), "ack");
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM connections WHERE id='late-after-grace'",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(db.query_row("SELECT COUNT(*) FROM operation_effects e JOIN operations o ON o.record_id=e.operation_record_id WHERE o.operation_id='late-after-grace'",[],|r|r.get::<_,u64>(0)).unwrap(),0);
        quiet(&mut peer).await;
        let invited = source
            .session_command(Request {
                operation_id: "fresh-after-grace".into(),
                conversation_id: cid.clone(),
                session_id: "fresh-after-grace".into(),
                by_participant: None,
                expected_revision: None,
                action: Action::Invite {
                    participants: vec![],
                    medium: "voice".into(),
                    intent: "conference".into(),
                    capabilities: json!({"audio":["g.711-mu"]}),
                },
            })
            .await
            .unwrap();
        assert!(
            matches!(invited,Outcome::SessionControl(ref value) if value.session.id=="fresh-after-grace" && value.session.state=="accepted")
        );
        if let Some(directory) = std::env::var_os("CONNECT_G6_RECONNECT_SAMPLES") {
            let directory = std::path::PathBuf::from(directory);
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join(format!("{algorithm}-session-default-grace.json")),serde_json::to_vec_pretty(&json!({"algorithm":algorithm,"lost_at_millis":loss,"deadline_millis":deadline,"ended":ended.original(),"late_offer_refused":true})).unwrap()).unwrap();
        }
        source.close();
        peer.close();
        f.server.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn production_current_native_offer_in_flight_survives_grace_and_reconnects_same_session() {
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_native_end_scope(algorithm, 12).await;
        f.server.store.configure_session_reconnect_grace(2).unwrap();
        ready_endpoint(&f).await;
        let mut source = CanonicalClient::open(f.client.clone()).await.unwrap();
        let cid = session(&f, &mut source).await;
        let mut peer = CanonicalClient::open(party_with_scope(
            &f,
            &f.peer.identity_id,
            126,
            vec!["audio".into()],
            Scope::collaboration(vec!["*".into()]),
        ))
        .await
        .unwrap();
        let mut coverage = cover_accepted_peer(&f, &cid, Some(&mut peer)).await;
        let first = media(&mut source, &cid, "pending-first").await;
        flow(&source, first, 1).await;
        peer_event(&mut peer, "session.started", &cid).await;
        peer_event(&mut peer, "session.participant.joined", &cid).await;
        end_coverage(&mut coverage, &cid, Some(&mut peer)).await;
        end_native_connection(
            &mut source,
            &cid,
            "pending-first",
            "pending-end-first",
            None,
            "completed",
        )
        .await;
        wait_slots(&f, 0).await;
        peer_event(&mut peer, "session.participant.left", &cid).await;
        let scope = CommandScope {
            conversation_id: Some(cid.clone()),
            session_id: Some("native-session".into()),
            connection_id: Some("pending-reconnect".into()),
            operation_id: Some("pending-reconnect".into()),
            expected_revision: Some(0),
        };
        let capabilities = av_capabilities();
        source.send_value("connection.offer",json!({"substrate":"quic","capabilities":capabilities,"streams_offered":[{"id":"pending-voice","kind":"audio","direction":"sendonly","codec_preferences":["g.711-mu"]}]}),scope.clone()).await.unwrap();
        let answer = source.receive_control().await.unwrap();
        assert_eq!(answer.kind(), "connection.answer");
        let callback = source.receive_control().await.unwrap();
        assert_eq!(callback.kind(), "connection.offer");
        tokio::time::sleep(Duration::from_millis(2300)).await;
        let db = rusqlite::Connection::open(&f.config.database).unwrap();
        assert_eq!(
            db.query_row(
                "SELECT state FROM sessions WHERE id='native-session'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "active"
        );
        assert_eq!(
            db.query_row(
                "SELECT state FROM operations WHERE operation_id='pending-reconnect'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "preparing"
        );
        quiet(&mut peer).await;
        let handle = answer.original()["payload"]["substrate_setup"]["streams"][0]
            ["stream_local_id"]
            .as_u64()
            .unwrap() as u16;
        let p = &callback.original()["payload"];
        let streams=p["substrate_setup"]["streams"].as_array().unwrap().iter().map(|s|json!({"id":s["strm_id"],"kind":s["kind"],"direction":match s["direction"].as_str().unwrap(){"sendonly"=>"recvonly","recvonly"=>"sendonly",_=>"sendrecv"},"codec":s["codec"]})).collect::<Vec<_>>();
        // Real input can arrive while the original callback is awaiting its
        // answer. The readiness source retries metadata after completion.
        flow(&source, handle, 101).await;
        source.answer_connection_offer(&callback,serde_json::from_value(json!({"by_participant":p["by_participant"],"substrate":"quic","capabilities":capabilities,"streams_answered":streams,"substrate_setup":p["substrate_setup"]})).unwrap(),&scope).await.unwrap();
        flow(&source, handle, 201).await;
        let joined = peer_event(&mut peer, "session.participant.joined", &cid).await;
        assert_eq!(
            joined.original()["payload"]["via_connection"],
            "pending-reconnect"
        );
        assert!(db.query_row("SELECT deadline_millis FROM session_reconnect_boundaries WHERE session_id='native-session'",[],|r|r.get::<_,Option<i64>>(0)).unwrap().is_none());
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM session_terminal_facts", [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            0
        );
        if let Some(directory) = std::env::var_os("CONNECT_G6_RECONNECT_SAMPLES") {
            let directory = std::path::PathBuf::from(directory);
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join(format!("{algorithm}-session-pending-grace.json")),serde_json::to_vec_pretty(&json!({"algorithm":algorithm,"configured_grace_seconds":2,"delay_millis":2300,"joined":joined.original(),"session_state":"active"})).unwrap()).unwrap();
        }
        source.close();
        peer.close();
        f.server.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn production_confirmed_loss_deadline_survives_owned_listener_restart_and_policy_change() {
    for algorithm in ["EdDSA", "ES256"] {
        let mut f = Fixture::with_native_end_scope(algorithm, 12).await;
        f.server.store.configure_session_reconnect_grace(5).unwrap();
        ready_endpoint(&f).await;
        let mut source = CanonicalClient::open(f.client.clone()).await.unwrap();
        let cid = session(&f, &mut source).await;
        let baseline = source
            .conversation_snapshot(&cid, serde_json::from_value(json!({})).unwrap())
            .await
            .unwrap()
            .cursor;
        let mut coverage = cover_accepted_peer(&f, &cid, None).await;
        let first = media(&mut source, &cid, "restart-grace-first").await;
        flow(&source, first, 1).await;
        let db = rusqlite::Connection::open(&f.config.database).unwrap();
        let until = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            if db
                .query_row("SELECT COUNT(*) FROM session_started_facts", [], |r| {
                    r.get::<_, u64>(0)
                })
                .unwrap()
                == 1
            {
                break;
            }
            assert!(tokio::time::Instant::now() < until);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        end_coverage(&mut coverage, &cid, None).await;
        end_native_connection(
            &mut source,
            &cid,
            "restart-grace-first",
            "restart-grace-end",
            None,
            "completed",
        )
        .await;
        wait_slots(&f, 0).await;
        let durable = tokio::time::Instant::now() + Duration::from_secs(3);
        let (lost, deadline) = loop {
            let (lost,deadline):(Option<i64>,Option<i64>)=db.query_row("SELECT lost_at_millis,deadline_millis FROM session_reconnect_boundaries WHERE session_id='native-session'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
            if let (Some(lost), Some(deadline)) = (lost, deadline) {
                break (lost, deadline);
            }
            assert!(
                tokio::time::Instant::now() < durable,
                "actual retirement must settle its durable loss metadata"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        assert_eq!(deadline - lost, 5000);
        let original_runtime = f.server.store.runtime_epoch().to_owned();
        source.close();
        f.server.shutdown().await.unwrap();
        assert_eq!(
            db.query_row(
                "SELECT state FROM sessions WHERE id='native-session'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "active",
            "restart must exercise a live loss deadline"
        );
        assert!(chrono::Utc::now().timestamp_millis() < deadline);
        // The protected server configuration defaults to 30 seconds. This may
        // affect future invitations, never renew an existing loss deadline.
        f.server = RunningCanonicalServer::start(f.config.clone())
            .await
            .unwrap();
        f.client.endpoint = f.server.address;
        assert_ne!(original_runtime, f.server.store.runtime_epoch());
        assert_eq!(db.query_row("SELECT deadline_millis FROM session_reconnect_boundaries WHERE session_id='native-session'",[],|r|r.get::<_,i64>(0)).unwrap(),deadline);
        assert_eq!(
            db.query_row(
                "SELECT state FROM sessions WHERE id='native-session'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "active"
        );
        assert_eq!(db.query_row("SELECT grace_seconds FROM session_reconnect_boundaries WHERE session_id='native-session'",[],|r|r.get::<_,u32>(0)).unwrap(),5);
        let until = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let current: String = db
                .query_row(
                    "SELECT state FROM sessions WHERE id='native-session'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            if current == "ended" {
                break;
            }
            assert_eq!(db.query_row("SELECT deadline_millis FROM session_reconnect_boundaries WHERE session_id='native-session'",[],|r|r.get::<_,i64>(0)).unwrap(),deadline);
            assert!(tokio::time::Instant::now() < until);
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(chrono::Utc::now().timestamp_millis() >= deadline);
        let mut http = CanonicalHttpClient::open(canonical_http_config(&f))
            .await
            .unwrap();
        let catchup = http
            .subscribe_conversation(
                &cid,
                serde_json::from_value(json!({"after":baseline,"live":false})).unwrap(),
            )
            .await
            .unwrap();
        catchup.ack().unwrap();
        let journals = catchup
            .frames
            .iter()
            .filter(|e| e.kind() == "conversation.event")
            .filter_map(|e| {
                let event: connect_protocol::events::JournalEvent =
                    serde_json::from_value(e.original()["payload"]["data"].clone()).unwrap();
                if matches!(
                    event.data,
                    connect_protocol::events::JournalData::SessionEnded(_)
                ) {
                    Some(event)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(journals.len(), 1);
        let connect_protocol::events::JournalData::SessionEnded(fact) = &journals[0].data else {
            panic!()
        };
        assert_eq!(fact.reason, "reconnect-grace-expired");
        assert_eq!(fact.reason_code, 408);
        assert!(fact.by_participant.is_none());
        if let Some(directory) = std::env::var_os("CONNECT_G6_RECONNECT_SAMPLES") {
            let directory = std::path::PathBuf::from(directory);
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join(format!("{algorithm}-session-restart-grace.json")),serde_json::to_vec_pretty(&json!({"algorithm":algorithm,"lost_at_millis":lost,"deadline_millis":deadline,"frozen_grace_seconds":5,"restart_default_seconds":30,"runtime_changed":true,"journal":journals})).unwrap()).unwrap();
        }
        f.server.shutdown().await.unwrap();
    }
}
