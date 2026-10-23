//! One owned native invite/decision/media/end/record journey through public SDKs.
use super::*;
async fn receive(client: &mut CanonicalClient, cid: &str, frames: &mut Vec<Value>) -> Envelope {
    let e = tokio::time::timeout(Duration::from_secs(6), client.receive())
        .await
        .unwrap()
        .unwrap();
    if let Some(actual) = e.scope(rvoip_uctp::v2::Scope::Conversation) {
        assert_eq!(actual, cid);
    }
    if e.scope(rvoip_uctp::v2::Scope::Session).is_some() {
        assert_eq!(e.original()["sid"], "native-session");
    }
    frames.push(e.original().clone());
    e
}
async fn fact(
    client: &mut CanonicalClient,
    cid: &str,
    frames: &mut Vec<Value>,
    kind: &str,
) -> Value {
    for _ in 0..128 {
        if let Some(e) = frames.iter().find(|e| e["type"] == kind) {
            return e.clone();
        }
        let e = receive(client, cid, frames).await;
        assert!(
            matches!(
                e.kind(),
                "connection.ready"
                    | "stream.opened"
                    | "stream.closed"
                    | "connection.quality"
                    | "session.started"
                    | "session.participant.joined"
                    | "session.participant.left"
                    | "session.ended"
                    | "recording.vcon-ready"
            ),
            "{:?}",
            e.original()
        );
    }
    panic!("missing {kind}")
}
async fn media(
    client: &mut CanonicalClient,
    cid: &str,
    name: &str,
    frames: &mut Vec<Value>,
) -> u16 {
    let scope = CommandScope {
        conversation_id: Some(cid.into()),
        session_id: Some("native-session".into()),
        connection_id: Some(name.into()),
        operation_id: Some(format!("{name}-offer")),
        expected_revision: Some(0),
    };
    client.send_value("connection.offer",json!({"substrate":"quic","capabilities":av_capabilities(),"streams_offered":[{"id":format!("{name}-audio"),"kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}]}),scope.clone()).await.unwrap();
    let mut answer = None;
    for _ in 0..128 {
        let e = receive(client, cid, frames).await;
        match e.kind() {
            "connection.answer" => {
                assert_eq!(e.original()["connid"], name);
                answer = Some(e);
            }
            "connection.offer" => {
                assert_eq!(e.original()["connid"], name);
                let p = &e.original()["payload"];
                let streams=p["substrate_setup"]["streams"].as_array().unwrap().iter().map(|s|json!({"id":s["strm_id"],"kind":s["kind"],"direction":match s["direction"].as_str().unwrap(){"sendonly"=>"recvonly","recvonly"=>"sendonly",_=>"sendrecv"},"codec":s["codec"]})).collect::<Vec<_>>();
                client.answer_connection_offer(&e,serde_json::from_value(json!({"by_participant":p["by_participant"],"substrate":"quic","capabilities":av_capabilities(),"streams_answered":streams,"substrate_setup":p["substrate_setup"]})).unwrap(),&scope).await.unwrap();
                break;
            }
            _ => assert!(super::lifecycle::session_start(&e)),
        }
    }
    answer.unwrap().original()["payload"]["substrate_setup"]["streams"][0]["stream_local_id"]
        .as_u64()
        .unwrap() as u16
}
async fn barrier_without_record(client: &mut CanonicalClient, cid: &str, frames: &mut Vec<Value>) {
    let id = client.keepalive().await.unwrap();
    for _ in 0..128 {
        let e = receive(client, cid, frames).await;
        assert_ne!(
            e.kind(),
            "recording.vcon-ready",
            "a denied or uncommitted Record cannot be advertised"
        );
        if e.kind() == "ack" {
            assert_eq!(e.original()["in_reply_to"], id);
            return;
        }
        assert!(super::lifecycle::session_start(&e));
    }
    panic!("missing reliable barrier")
}
#[tokio::test]
async fn production_owned_native_session_runs_invite_relay_media_end_and_independent_private_record_multicast(
) {
    run(false).await;
}
#[tokio::test]
async fn production_record_ready_storage_failure_recovers_same_outcome_without_more_media_or_duplicate_notice(
) {
    run(true).await;
}
async fn run(fail_ready: bool) {
    use connect_client::canonical::{CanonicalRecordSelector, SessionInvitationAnswer};
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_native_end_scope(algorithm, 12).await;
        ready_endpoint(&f).await;
        let mut owner = CanonicalClient::open(f.client.clone()).await.unwrap();
        let mut input = connect_client::conversations::CreateRequest::persistent(vec![f
            .peer
            .identity_id
            .clone()]);
        input.tenant_id = Some("tnt_default".into());
        let room = owner
            .create_conversation("journey-room", input)
            .await
            .unwrap();
        let cid = &room.id;
        let owner_pid = room
            .participants
            .iter()
            .find(|p| p.identity_id == f.client.identity_id)
            .unwrap()
            .participant_id
            .clone();
        let peer_pid = room
            .participants
            .iter()
            .find(|p| p.identity_id == f.peer.identity_id)
            .unwrap()
            .participant_id
            .clone();
        let mut scope = Scope::collaboration(vec!["*".into()]);
        scope
            .operations
            .extend(["stream.subscribe".into(), "stream.unsubscribe".into()]);
        let peer_config = party_with_scope(
            &f,
            &f.peer.identity_id,
            310,
            vec!["audio".into()],
            scope.clone(),
        );
        let mut no_record_scope = scope.clone();
        no_record_scope.operations.retain(|s| s != "record_get");
        let mut peer = CanonicalClient::open(peer_config.clone()).await.unwrap();
        let mut owner_frames = vec![];
        let mut peer_frames = vec![];
        let mut denied_frames = vec![];
        let invited=owner.request_value("session.invite",json!({"from":owner_pid,"to":[peer_pid],"medium":"voice","intent":"synchronous-engagement","capabilities_offer":{"audio":["g.711-mu"]}}),CommandScope {conversation_id:Some(cid.clone()),session_id:Some("native-session".into()),operation_id:Some("journey-invite".into()),..Default::default()}).await.unwrap();
        invited.ack().unwrap();
        let invitation = peer.receive_session_invitation().await.unwrap();
        peer_frames.push(invitation.original().clone());
        assert_eq!(invitation.original()["payload"]["from"], owner_pid);
        assert_eq!(invitation.original()["payload"]["to"], json!([peer_pid]));
        peer.answer_session_invitation(
            &invitation,
            SessionInvitationAnswer::Accept {
                capabilities: json!({"audio":["g.711-mu"]}),
            },
        )
        .await
        .unwrap();
        let relay = receive(&mut owner, cid, &mut owner_frames).await;
        assert_eq!(relay.kind(), "session.accept");
        assert_eq!(relay.original()["in_reply_to"], invited.request_id);
        assert_eq!(relay.original()["payload"]["by"], peer_pid);
        // This additional physical binding has the same Participant epoch but
        // lacks Record read permission. It was not an invitation waiter.
        let denied_config = party_with_scope(
            &f,
            &f.peer.identity_id,
            311,
            vec!["audio".into()],
            no_record_scope,
        );
        let mut denied = CanonicalClient::open(denied_config).await.unwrap();
        let baseline = owner
            .conversation_snapshot(cid, serde_json::from_value(json!({})).unwrap())
            .await
            .unwrap()
            .cursor;
        let owner_handle = media(&mut owner, cid, "journey-owner", &mut owner_frames).await;
        let peer_handle = media(&mut peer, cid, "journey-peer", &mut peer_frames).await;
        wait_native_operation(&f, &f.client.credential, cid, "journey-owner-offer").await;
        wait_native_operation(&f, &peer_config.credential, cid, "journey-peer-offer").await;
        let mut clients = vec![owner, peer];
        assert!(stream_control(
            &mut clients[0],
            cid,
            "journey-owner",
            "journey-owner-select",
            1,
            "stream.subscribe",
            json!({"subscriptions":[{"from_participant":peer_pid,"kinds":["audio"]}]})
        )
        .await
        .is_none());
        assert!(stream_control(
            &mut clients[1],
            cid,
            "journey-peer",
            "journey-peer-select",
            1,
            "stream.subscribe",
            json!({"subscriptions":[{"from_participant":owner_pid,"kinds":["audio"]}]})
        )
        .await
        .is_none());
        // Route selection is explicit initial media setup. With no selected
        // receive routes, duplex readiness must remain unobserved.
        super::participants::flow(&clients[0], owner_handle, 1).await;
        super::participants::flow(&clients[1], peer_handle, 101).await;
        let owner_start = fact(&mut clients[0], cid, &mut owner_frames, "session.started").await;
        let peer_start = fact(&mut clients[1], cid, &mut peer_frames, "session.started").await;
        assert_eq!(owner_start["payload"], peer_start["payload"]);
        assert_eq!(
            owner_start["payload"]["participants_present"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_audio_reaches_other_participant(&mut clients, owner_handle, &[peer_handle], 1001)
            .await;
        clients.swap(0, 1);
        assert_audio_reaches_other_participant(&mut clients, peer_handle, &[owner_handle], 2001)
            .await;
        clients.swap(0, 1);
        let db = rusqlite::Connection::open(&f.config.database).unwrap();
        if fail_ready {
            db.execute_batch("CREATE TRIGGER fail_journey_ready BEFORE INSERT ON record_ready_facts BEGIN SELECT RAISE(ABORT,'controlled Record readiness failure'); END").unwrap();
        }
        let end = Request {
            operation_id: "journey-peer-end".into(),
            conversation_id: cid.clone(),
            session_id: "native-session".into(),
            by_participant: Some(peer_pid.clone()),
            expected_revision: Some(1),
            action: Action::End {
                reason_code: 200,
                reason: "journey-complete".into(),
            },
        };
        let ended_receipt = clients[1].session_command(end.clone()).await.unwrap();
        let owner_end = fact(&mut clients[0], cid, &mut owner_frames, "session.ended").await;
        let peer_end = fact(&mut clients[1], cid, &mut peer_frames, "session.ended").await;
        assert_eq!(owner_end["payload"], peer_end["payload"]);
        assert_eq!(owner_end["payload"]["by"], peer_pid);
        if fail_ready {
            assert_eq!(
                f.server
                    .service
                    .finalize_record_outcomes(1)
                    .await
                    .unwrap()
                    .failed,
                1
            );
            assert_eq!(
                db.query_row("SELECT COUNT(*) FROM record_ready_facts", [], |r| r
                    .get::<_, u64>(0))
                    .unwrap(),
                0
            );
            assert_eq!(
                db.query_row("SELECT COUNT(*) FROM session_records", [], |r| r
                    .get::<_, u64>(0))
                    .unwrap(),
                0
            );
            barrier_without_record(&mut clients[0], cid, &mut owner_frames).await;
            db.execute_batch("DROP TRIGGER fail_journey_ready").unwrap();
        }
        let owner_ready = fact(
            &mut clients[0],
            cid,
            &mut owner_frames,
            "recording.vcon-ready",
        )
        .await;
        let peer_ready = fact(
            &mut clients[1],
            cid,
            &mut peer_frames,
            "recording.vcon-ready",
        )
        .await;
        assert_eq!(owner_ready["payload"], peer_ready["payload"]);
        for hint in [&owner_ready, &peer_ready] {
            for absent in ["connid", "operation_id", "in_reply_to"] {
                assert!(hint.get(absent).is_none());
            }
            assert!(hint["payload"].get("vcon").is_none());
            assert!(hint["payload"]["vcon_handle"].get("content_hash").is_none());
        }
        barrier_without_record(&mut denied, cid, &mut denied_frames).await;
        let id = owner_ready["payload"]["vcon_handle"]["uuid"]
            .as_str()
            .unwrap()
            .to_owned();
        let own_record = clients[0]
            .fetch_record(
                cid,
                "native-session",
                CanonicalRecordSelector {
                    uuid: Some(id.clone()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let peer_record = clients[1]
            .fetch_record(
                cid,
                "native-session",
                CanonicalRecordSelector {
                    url: Some(format!("connect:vcon/{id}")),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_ne!(own_record.content_hash, peer_record.content_hash);
        let owner_body: Value = serde_json::from_str(&own_record.body).unwrap();
        let peer_body: Value = serde_json::from_str(&peer_record.body).unwrap();
        assert_eq!(owner_body["uuid"], id);
        assert_eq!(peer_body["uuid"], id);
        assert!(own_record.body.contains("journey-owner"));
        assert!(!own_record.body.contains("journey-peer"));
        assert!(peer_record.body.contains("journey-peer"));
        assert!(!peer_record.body.contains("journey-owner"));
        for body in [&owner_body, &peer_body] {
            assert!(body.get("signatures").is_none());
            assert_eq!(body["analysis"][0]["body"]["capture_complete"], false);
            assert_eq!(body["analysis"][0]["body"]["recorded_audio"], false);
        }
        assert!(denied
            .fetch_record(
                cid,
                "native-session",
                CanonicalRecordSelector {
                    uuid: Some(id.clone()),
                    ..Default::default()
                }
            )
            .await
            .is_err());
        assert_eq!(
            clients[0]
                .fetch_record(
                    cid,
                    "native-session",
                    CanonicalRecordSelector {
                        uuid: Some(id.clone()),
                        ..Default::default()
                    }
                )
                .await
                .unwrap()
                .content_hash,
            own_record.content_hash
        );
        assert_eq!(
            clients[1].session_command(end).await.unwrap(),
            ended_receipt
        );
        barrier_without_record(&mut clients[1], cid, &mut peer_frames).await;
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM record_ready_facts", [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            1
        );
        let mut http = CanonicalHttpClient::open(canonical_http_config(&f))
            .await
            .unwrap();
        let catchup = http
            .subscribe_conversation(
                cid,
                serde_json::from_value(json!({"after":baseline,"live":false})).unwrap(),
            )
            .await
            .unwrap();
        catchup.ack().unwrap();
        assert_eq!(
            catchup
                .frames
                .iter()
                .filter(|e| e.kind() == "recording.vcon-ready")
                .count(),
            1
        );
        if let Some(directory) = std::env::var_os("CONNECT_G6_JOURNEY_SAMPLES") {
            let directory = std::path::PathBuf::from(directory);
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join(format!("{algorithm}-journey-{}.json",if fail_ready {"recovery"}else{"normal"})),serde_json::to_vec_pretty(&json!({"algorithm":algorithm,"recovery":fail_ready,"invite_request":invited.request_id,"owner_frames":owner_frames,"peer_frames":peer_frames,"denied_frames":denied_frames,"owner_record":owner_body,"peer_record":peer_body,"owner_record_bytes":own_record.body,"peer_record_bytes":peer_record.body,"owner_hash":own_record.content_hash,"peer_hash":peer_record.content_hash,"catchup":catchup.frames.iter().map(|e|e.original()).collect::<Vec<_>>(),"bidirectional_audio_verified":true})).unwrap()).unwrap();
        }
        for client in clients {
            client.close();
        }
        denied.close();
        f.server.shutdown().await.unwrap();
    }
}
