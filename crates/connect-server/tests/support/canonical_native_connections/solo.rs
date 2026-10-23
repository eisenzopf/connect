//! One implicit initiator, actual native input, and later growth on the same SID.
use super::*;

#[tokio::test]
async fn production_solo_session_admits_actual_native_flow_and_preserves_original_receipt_when_growing(
) {
    solo_session(false).await;
}
#[tokio::test]
async fn production_native_flow_fact_precedes_hints_and_recovers_storage_without_more_media() {
    solo_session(true).await;
}
async fn solo_session(fail_flow_storage: bool) {
    use rvoip_uctp::substrate::{pack_rtp_datagram, RtpDatagram, RtpMediaPayload};
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_native_end_scope(algorithm, 12).await;
        ready_endpoint(&f).await;
        let mut native = CanonicalClient::open(f.client.clone()).await.unwrap();
        let mut room_input = connect_client::conversations::CreateRequest::persistent(vec![f
            .peer
            .identity_id
            .clone()]);
        room_input.tenant_id = Some("tnt_default".into());
        let room = native
            .create_conversation("solo-room", room_input)
            .await
            .unwrap();
        let peer = room
            .participants
            .iter()
            .find(|p| p.identity_id == f.peer.identity_id)
            .unwrap()
            .participant_id
            .clone();
        let mut invite = Request {
            operation_id: "solo-create".into(),
            conversation_id: room.id.clone(),
            session_id: "native-session".into(),
            by_participant: None,
            expected_revision: None,
            action: Action::Invite {
                participants: vec![],
                medium: "voice".into(),
                intent: "conference".into(),
                capabilities: json!({"audio":["g.711-mu"]}),
            },
        };
        let created = native.session_command(invite.clone()).await.unwrap();
        assert!(
            matches!(&created, Outcome::SessionControl(v) if v.session.state == "accepted" && v.session.topology_revision == 0)
        );
        let mut http = CanonicalHttpClient::open(canonical_http_config(&f))
            .await
            .unwrap();
        assert_eq!(http.session_command(invite.clone()).await.unwrap(), created);
        let mut peer_config = canonical_http_config(&f);
        peer_config.credential = f
            .server
            .store
            .issue(
                &f.peer.identity_id,
                Scope::collaboration(vec!["*".into()]),
                chrono::Utc::now().timestamp() + 600,
            )
            .unwrap()
            .token;
        let mut other = CanonicalHttpClient::open(peer_config).await.unwrap();
        let uninvited_end = Request {
            operation_id: "other-cannot-end-solo".into(),
            action: Action::End {
                reason_code: 200,
                reason: "not-a-participant".into(),
            },
            expected_revision: Some(0),
            ..invite.clone()
        };
        assert!(
            matches!(other.session_command(uninvited_end).await, Err(ClientError::Application(e)) if e.code == "forbidden")
        );

        let answer = negotiate_av(&mut native, &room.id, "solo-audio-offer", "solo-audio", json!([
            {"id":"solo-voice","kind":"audio","direction":"sendonly","codec_preferences":["g.711-mu"]}
        ])).await;
        wait_native_operation(&f, &f.client.credential, &room.id, "solo-audio-offer").await;
        let handle = answer.original()["payload"]["substrate_setup"]["streams"][0]
            ["stream_local_id"]
            .as_u64()
            .unwrap() as u16;
        // A real reliable barrier confirms negotiation alone emits no flow fact.
        let barrier = native.keepalive().await.unwrap();
        let ack = native.receive().await.unwrap();
        assert_eq!(ack.kind(), "ack");
        assert_eq!(ack.original()["in_reply_to"], barrier);
        let database = rusqlite::Connection::open(&f.config.database).unwrap();
        assert_eq!(
            database
                .query_row(
                    "SELECT COUNT(*) FROM native_connection_flow_facts",
                    [],
                    |r| r.get::<_, u64>(0)
                )
                .unwrap(),
            0
        );
        if fail_flow_storage {
            database.execute_batch("CREATE TRIGGER fail_flow_fact BEFORE INSERT ON journal WHEN NEW.kind='session_started' BEGIN SELECT RAISE(ABORT,'isolated flow fact storage failure'); END;").unwrap();
        }
        for seq in 1..=30 {
            native
                .send_native_rtp_datagram(
                    &pack_rtp_datagram(&RtpDatagram {
                        flags: 0,
                        stream_local_id: handle,
                        seq,
                        rtp: RtpMediaPayload {
                            payload: vec![0x80; 160].into(),
                            payload_type: 0,
                            sequence_number: seq as u16,
                            timestamp: seq * 160,
                            ssrc: 0x4736_0022,
                        },
                    })
                    .unwrap(),
                )
                .unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        if fail_flow_storage {
            assert_eq!(
                database
                    .query_row(
                        "SELECT COUNT(*) FROM native_connection_flow_facts",
                        [],
                        |r| r.get::<_, u64>(0)
                    )
                    .unwrap(),
                0
            );
            let barrier = native.keepalive().await.unwrap();
            let ack = native.receive().await.unwrap();
            assert_eq!(
                ack.kind(),
                "ack",
                "durable fact failure must not publish ready/opened hints"
            );
            assert_eq!(ack.original()["in_reply_to"], barrier);
            database
                .execute_batch("DROP TRIGGER fail_flow_fact")
                .unwrap();
            // No more datagrams: metadata persistence recovers the existing
            // real flow rather than repeating or inventing physical IO.
        }
        let mut ready = false;
        let mut opened = false;
        let mut started = None;
        tokio::time::timeout(Duration::from_secs(5), async {
            while !ready || !opened || started.is_none() {
                let event = native.receive().await.unwrap();
                assert_eq!(event.original()["cid"], room.id);
                assert_eq!(event.original()["sid"], "native-session");
                if event.kind() == "session.started" {
                    assert!(started.is_none());
                    for absent in ["connid", "operation_id", "in_reply_to"] {
                        assert!(event.original().get(absent).is_none());
                    }
                    let body = &event.original()["payload"];
                    assert_eq!(body["active_connections"], json!([{"connid":"solo-audio","participant_id":room.participants.iter().find(|p|p.identity_id==f.client.identity_id).unwrap().participant_id,"transport":"quic"}]));
                    assert_eq!(body["participants_present"].as_array().unwrap().len(),1);
                    assert_eq!(body["negotiated_capabilities"]["audio_codecs"][0]["name"], "g.711-mu");
                    started = Some(body.clone());
                    continue;
                }
                if event.kind()=="session.participant.joined" {
                    assert_eq!(event.original()["payload"]["via_connection"],"solo-audio");
                    assert!(super::lifecycle::session_start(&event));
                    continue;
                }
                assert_eq!(event.original()["connid"], "solo-audio");
                match event.kind() {
                    "connection.ready" => {
                        assert!(!ready);
                        ready = true;
                    }
                    "stream.opened" => {
                        assert!(ready && !opened);
                        assert_eq!(
                            event.original()["payload"]["stream"]["strm_id"],
                            "solo-voice"
                        );
                        opened = true;
                    }
                    _ => panic!("unexpected solo lifecycle: {:?}", event.original()),
                }
            }
        })
        .await
        .unwrap();
        let (runtime,raw):(String,String)=database.query_row("SELECT m.runtime_connection_id,f.fact_json FROM native_connection_flow_facts f JOIN connection_offers m ON m.connection_id=f.connection_id WHERE f.connection_id='solo-audio'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        let fact: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(fact["runtime_connection_id"], runtime);
        assert_eq!(fact["streams"], json!(["solo-voice"]));
        assert_eq!(fact["generation"], 1);
        assert!(
            fact["observed_at_millis"].as_i64().unwrap() <= chrono::Utc::now().timestamp_millis()
        );

        let original_start: String = database
            .query_row("SELECT fact_json FROM session_started_facts", [], |r| {
                r.get(0)
            })
            .unwrap();
        let start: Value = serde_json::from_str(&original_start).unwrap();
        assert_eq!(start["session"]["state"], "active");
        assert_eq!(
            start["active_connections"],
            started.as_ref().unwrap()["active_connections"]
        );
        assert_eq!(
            database
                .query_row(
                    "SELECT state FROM sessions WHERE id='native-session'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "active"
        );
        assert_eq!(
            database
                .query_row(
                    "SELECT COUNT(*) FROM journal WHERE kind='session_started'",
                    [],
                    |r| r.get::<_, u64>(0)
                )
                .unwrap(),
            1
        );

        // The same logical Session can admit another Participant later. None
        // of these decisions rewrites the initial solo receipt or reoffers IO.
        invite.operation_id = "grow-solo".into();
        invite.expected_revision = Some(0);
        let Action::Invite { participants, .. } = &mut invite.action else {
            unreachable!()
        };
        *participants = vec![peer];
        let grown = native.session_command(invite.clone()).await.unwrap();
        assert!(
            matches!(grown, Outcome::SessionControl(v) if v.session.id == "native-session" && v.session.topology_revision == 1)
        );
        let accepted = other
            .session_command(Request {
                operation_id: "join-grown-solo".into(),
                expected_revision: Some(1),
                action: Action::Accept {
                    capabilities: json!({"audio":["g.711-mu"]}),
                },
                ..invite.clone()
            })
            .await
            .unwrap();
        assert!(
            matches!(accepted, Outcome::SessionControl(v) if v.session.id == "native-session" && v.session.topology_revision == 2)
        );
        invite.operation_id = "solo-create".into();
        invite.expected_revision = None;
        let Action::Invite { participants, .. } = &mut invite.action else {
            unreachable!()
        };
        participants.clear();
        assert_eq!(
            native.session_command(invite.clone()).await.unwrap(),
            created
        );
        assert_eq!(http.session_command(invite).await.unwrap(), created);
        wait_slots(&f, 1).await;
        assert_eq!(
            database
                .query_row(
                    "SELECT COUNT(*) FROM native_connection_flow_facts",
                    [],
                    |r| r.get::<_, u64>(0)
                )
                .unwrap(),
            1
        );
        assert_eq!(
            database
                .query_row(
                    "SELECT fact_json FROM native_connection_flow_facts",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            raw
        );
        assert_eq!(
            database
                .query_row("SELECT fact_json FROM session_started_facts", [], |r| {
                    r.get::<_, String>(0)
                })
                .unwrap(),
            original_start
        );
        native.close();
        f.server.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn production_session_started_multicasts_original_snapshot_without_recipient_media_and_denies_narrow_scope(
) {
    use rvoip_uctp::substrate::{pack_rtp_datagram, RtpDatagram, RtpMediaPayload};
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_native_end_scope(algorithm, 12).await;
        ready_endpoint(&f).await;
        let mut source = CanonicalClient::open(f.client.clone()).await.unwrap();
        let cid = session(&f, &mut source).await;
        let config = party_with_scope(
            &f,
            &f.peer.identity_id,
            81,
            vec!["audio".into()],
            Scope::collaboration(vec!["*".into()]),
        );
        let mut recipient = CanonicalClient::open(config).await.unwrap();
        let narrow = party_with_scope(
            &f,
            &f.peer.identity_id,
            82,
            vec!["audio".into()],
            Scope::collaboration(vec![f.peer.identity_id.clone()]),
        );
        let mut restricted = CanonicalClient::open(narrow).await.unwrap();
        let answer = negotiate_av(&mut source, &cid, "started-offer", "started-connection", json!([
            {"id":"started-voice","kind":"audio","direction":"sendonly","codec_preferences":["g.711-mu"]}
        ])).await;
        wait_native_operation(&f, &f.client.credential, &cid, "started-offer").await;
        let handle = answer.original()["payload"]["substrate_setup"]["streams"][0]
            ["stream_local_id"]
            .as_u64()
            .unwrap() as u16;
        let baseline = source
            .conversation_snapshot(&cid, serde_json::from_value(json!({})).unwrap())
            .await
            .unwrap()
            .cursor;

        let db = rusqlite::Connection::open(&f.config.database).unwrap();
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM session_started_facts", [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            0
        );
        for seq in 1..=30 {
            source
                .send_native_rtp_datagram(
                    &pack_rtp_datagram(&RtpDatagram {
                        flags: 0,
                        stream_local_id: handle,
                        seq,
                        rtp: RtpMediaPayload {
                            payload: vec![0x80; 160].into(),
                            payload_type: 0,
                            sequence_number: seq as u16,
                            timestamp: seq * 160,
                            ssrc: 0x47360024,
                        },
                    })
                    .unwrap(),
                )
                .unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let other = recipient.receive().await.unwrap();
        assert_eq!(other.kind(), "session.started", "{:?}", other.original());
        assert_eq!(other.original()["cid"], cid);
        assert_eq!(other.original()["sid"], "native-session");
        assert_eq!(
            other.original()["payload"]["participants_present"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        let original = other.original()["payload"].clone();
        for absent in ["connid", "operation_id", "in_reply_to"] {
            assert!(other.original().get(absent).is_none());
        }
        loop {
            let event = source.receive().await.unwrap();
            if event.kind() == "session.started" {
                assert_eq!(event.original()["payload"], original);
                break;
            }
            assert!(matches!(
                event.kind(),
                "connection.ready" | "stream.opened" | "connection.quality"
            ));
        }
        let barrier = restricted.keepalive().await.unwrap();
        let ack = restricted.receive().await.unwrap();
        assert_eq!(ack.kind(), "ack");
        assert_eq!(ack.original()["in_reply_to"], barrier);
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM connections WHERE identity_id=?1",
                [&f.peer.identity_id],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM journal WHERE kind='session_started'",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            2
        );
        assert_eq!(
            db.query_row(
                "SELECT state FROM sessions WHERE id='native-session'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "active"
        );
        let mut http = CanonicalHttpClient::open(canonical_http_config(&f))
            .await
            .unwrap();
        let ended = source
            .session_command(Request {
                operation_id: "started-end".into(),
                conversation_id: cid.clone(),
                session_id: "native-session".into(),
                by_participant: None,
                expected_revision: Some(1),
                action: Action::End {
                    reason_code: 200,
                    reason: "normal-clearing".into(),
                },
            })
            .await
            .unwrap();
        assert!(matches!(ended,Outcome::SessionControl(ref v) if v.session.state=="ended"));
        let catchup = http
            .subscribe_conversation(
                &cid,
                serde_json::from_value(json!({"after":baseline,"live":false})).unwrap(),
            )
            .await
            .unwrap();
        catchup.ack().unwrap();
        let hint_index = catchup
            .frames
            .iter()
            .position(|e| e.kind() == "session.started")
            .unwrap();
        assert_eq!(catchup.frames[hint_index].original()["payload"], original);
        let event_index = catchup
            .frames
            .iter()
            .position(|e| {
                e.kind() == "conversation.event"
                    && e.original()["payload"]["kind"] == "session_started"
            })
            .unwrap();
        assert!(hint_index < event_index);
        let journal: connect_protocol::events::JournalEvent = serde_json::from_value(
            catchup.frames[event_index].original()["payload"]["data"].clone(),
        )
        .unwrap();
        let connect_protocol::events::JournalData::SessionStarted(started) = journal.data else {
            panic!("original start journal");
        };
        assert_eq!(started.session.state, "active");
        assert_eq!(
            serde_json::to_value(&started.active_connections).unwrap(),
            original["active_connections"]
        );
        assert_eq!(
            chrono::DateTime::parse_from_rfc3339(original["started_at"].as_str().unwrap())
                .unwrap()
                .timestamp(),
            started.started_at
        );
        assert_eq!(
            db.query_row(
                "SELECT state FROM sessions WHERE id='native-session'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "ended"
        );
        if let Some(directory) = std::env::var_os("CONNECT_G6_SESSION_STARTED_SAMPLES") {
            let directory = std::path::PathBuf::from(directory);
            std::fs::create_dir_all(&directory).unwrap();
            let sample = json!({"algorithm":algorithm,"hint":other.original(),"catchup":catchup.frames.iter().map(|e|e.original().clone()).collect::<Vec<_>>(),"journal":catchup.frames[event_index].original()["payload"]["data"],"recipient_connections":0,"narrow_scope_barrier":ack.original(),"terminal_state":"ended"});
            std::fs::write(
                directory.join(format!("{algorithm}-session-started.json")),
                serde_json::to_vec_pretty(&sample).unwrap(),
            )
            .unwrap();
        }
        source.close();
        recipient.close();
        restricted.close();
        f.server.shutdown().await.unwrap();
    }
}
