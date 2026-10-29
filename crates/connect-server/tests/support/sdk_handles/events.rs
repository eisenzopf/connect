//! Real signed live events, current authority, restart and decoded media.
use super::selections::{revision, stream};
use super::*;
use sdk::{Cursor, Direction, Medium, StreamSelector};
async fn client(config: CanonicalClientConfig) -> Client {
    Client::connect(config, "tnt_default".parse().unwrap())
        .await
        .unwrap()
}
fn logs() {
    static LOGS: std::sync::Once = std::sync::Once::new();
    LOGS.call_once(|| {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "connect_client::sdk::events=debug".into()),
            )
            .with_test_writer()
            .try_init();
    });
}
fn event_wait() -> RequestOptions {
    RequestOptions::new(Duration::from_secs(15)).unwrap()
}
async fn until_message(
    events: &mut sdk::EventStream,
    id: &str,
    forbidden: Option<&str>,
) -> sdk::JournalEvent {
    let until = tokio::time::Instant::now() + Duration::from_secs(15);
    for _ in 0..128 {
        let event = events
            .next()
            .await
            .unwrap()
            .expect("live journal must not end");
        assert!(
            tokio::time::Instant::now() < until,
            "message missing from live event stream"
        );
        if event.data["kind"] == "message" {
            assert_ne!(
                event.data["data"]["id"].as_str(),
                forbidden,
                "resumed cursor must not repeat an already processed Message"
            );
            if event.data["data"]["id"] == id {
                return event;
            }
        }
    }
    panic!("event budget exceeded before message {id}")
}
async fn send(
    room: &sdk::Conversation,
    recipients: NonEmpty<sdk::ParticipantId>,
    index: usize,
) -> sdk::Operation<sdk::MessageAcceptance> {
    room.send(
        format!("live-event-message-{index}").parse().unwrap(),
        recipients,
        format!("journal message {index}"),
        mutation(&format!("live-event-send-{index}")),
    )
    .await
    .unwrap()
}
async fn create_room(client: &Client, peer: &str, id: &str) -> sdk::Conversation {
    client
        .create_conversation(
            vec![peer.parse().unwrap()],
            ConversationPolicy::Persistent,
            mutation(id),
        )
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap()
        .value
}
async fn owner(room: &sdk::Conversation, identity: &str) -> sdk::ParticipantId {
    room.refresh(options())
        .await
        .unwrap()
        .participants
        .into_iter()
        .find(|p| p.identity_id.as_str() == identity)
        .unwrap()
        .id
}
#[tokio::test]
async fn public_sdk_live_events_preserve_original_facts_resume_cursor_and_isolate_current_authority(
) {
    logs();
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_native_end_scope(algorithm, 16).await;
        let peer_config = super::super::canonical_native_connections::party_with_scope(
            &f,
            &f.peer.identity_id,
            231,
            vec!["audio".into()],
            Scope::collaboration(vec!["*".into()]),
        );
        let publisher = client(f.client.clone()).await;
        let peer = client(peer_config.clone()).await;
        let room = create_room(&publisher, &f.peer.identity_id, "live-events-create").await;
        let mine = owner(&room, &f.client.identity_id).await;
        let theirs = owner(&room, &f.peer.identity_id).await;
        let recipients = NonEmpty {
            first: mine,
            rest: vec![theirs.clone()],
        };
        let mut events = room.events(None, event_wait());
        let initial = events.snapshot().await.unwrap().unwrap();
        assert_eq!(initial.conversation.id, *room.id());
        assert!(initial
            .facts
            .iter()
            .any(|f| matches!(f, sdk::SnapshotFact::Conversation(_))));
        let baseline = events.cursor().unwrap();
        let peer_room = peer.conversation_by_id(room.id().clone());
        let mut peer_events = peer_room.events(None, event_wait());
        peer_events.snapshot().await.unwrap();
        let peer_baseline = peer_events.cursor().unwrap();
        let first = send(&room, recipients.clone(), 0).await;
        let saved_send = first.reference_json();
        first.wait(options()).await.unwrap();
        let event = until_message(&mut events, "live-event-message-0", None).await;
        assert_eq!(
            event.operation_id.as_ref().unwrap().as_str(),
            "live-event-send-0"
        );
        assert_eq!(event.data["data"]["body"], "journal message 0");
        let other = until_message(&mut peer_events, "live-event-message-0", None).await;
        let peer_credential = f
            .server
            .store
            .authenticate(&peer_config.credential)
            .unwrap()
            .credential_id;
        let owner_credential = f
            .server
            .store
            .authenticate(&f.client.credential)
            .unwrap()
            .credential_id;
        let peer_actual = f
            .server
            .store
            .conversation_events(
                &peer_credential,
                room.id().as_str(),
                connect_protocol::events::EventQuery {
                    after: Some(peer_baseline.as_str().into()),
                    limit: Some(100),
                    ..Default::default()
                },
            )
            .unwrap();
        let peer_actual = peer_actual
            .events
            .iter()
            .find(|e| e.event_id == other.event_id)
            .expect("exact participant-specific journal row");
        assert_eq!(other.revision, peer_actual.revision);
        assert_eq!(
            other.source.correlation_id,
            peer_actual.source.correlation_id
        );
        assert_eq!(other.audience[0].participant_id.as_str(), theirs.as_str());
        assert_eq!(other.data, serde_json::to_value(&peer_actual.data).unwrap());
        assert!(
            other.operation_id.is_none(),
            "another actor cannot inherit private operation correlation"
        );
        assert!(peer
            .restore(saved_send, peer.operation_kinds().message_send)
            .unwrap()
            .status(options())
            .await
            .is_err());
        let actual = f
            .server
            .store
            .conversation_events(
                &owner_credential,
                room.id().as_str(),
                connect_protocol::events::EventQuery {
                    after: Some(baseline.as_str().into()),
                    limit: Some(100),
                    ..Default::default()
                },
            )
            .unwrap();
        let actual = actual
            .events
            .iter()
            .find(|e| e.event_id == event.event_id)
            .unwrap();
        assert_eq!(event.revision, actual.revision);
        assert_eq!(event.observed_at, actual.observed_at);
        assert_eq!(event.source.kind, actual.source.kind);
        assert_eq!(event.source.correlation_id, actual.source.correlation_id);
        assert_eq!(event.audience.len(), actual.audience.len());
        let saved = event.cursor.clone();
        events.close().await.unwrap();
        peer_events.close().await.unwrap();
        send(&room, recipients.clone(), 1)
            .await
            .wait(options())
            .await
            .unwrap();
        let mut resumed = room.events(Some(saved), event_wait());
        assert!(resumed.snapshot().await.unwrap().is_none());
        assert_eq!(
            until_message(
                &mut resumed,
                "live-event-message-1",
                Some("live-event-message-0")
            )
            .await
            .data["data"]["body"],
            "journal message 1"
        );
        let second = create_room(&publisher, &f.peer.identity_id, "live-events-other-create").await;
        let other_wait = event_wait();
        let cancelled = other_wait.signal.clone();
        let mut isolated = second.events(None, other_wait);
        isolated.snapshot().await.unwrap();
        cancelled.cancel();
        assert!(matches!(isolated.next().await,Err(e) if e.kind==sdk::ErrorKind::Cancelled));
        send(&room, recipients, 2)
            .await
            .wait(options())
            .await
            .unwrap();
        until_message(&mut resumed, "live-event-message-2", None).await;
        let mut revoked = peer_room.events(None, event_wait());
        revoked.snapshot().await.unwrap();
        let current = room.refresh(options()).await.unwrap();
        room.participant_by_id(theirs)
            .remove(current.revision, mutation("live-events-remove"))
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap();
        let failure = tokio::time::timeout(Duration::from_secs(15), revoked.next())
            .await
            .unwrap()
            .unwrap_err();
        assert!(
            matches!(
                failure.kind,
                sdk::ErrorKind::Authentication | sdk::ErrorKind::ResyncRequired
            ),
            "{failure:?}"
        );
        peer.conversation_by_id(second.id().clone())
            .refresh(options())
            .await
            .unwrap();
        revoked.close().await.unwrap();
        resumed.close().await.unwrap();
        isolated.close().await.unwrap();
        peer.close(options()).await.unwrap();
        publisher.close(options()).await.unwrap();
        f.server.shutdown().await.unwrap();
        eprintln!("SDK live journal {algorithm}: complete bootstrap, original fact/source/audience, owner-only correlation, cursor resume, independent cancellation and current membership refusal");
    }
}
#[tokio::test]
async fn public_sdk_live_event_renewal_keeps_mutations_and_decoded_native_audio_running() {
    logs();
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_native_end_scope(algorithm, 16).await;
        super::super::canonical_native_connections::ready_endpoint_with_capabilities(
            &f,
            vec!["audio".into()],
        )
        .await;
        let mut scope = Scope::collaboration(vec!["*".into()]);
        scope.operations.push("stream.subscribe".into());
        let peer_config = super::super::canonical_native_connections::party_with_scope(
            &f,
            &f.peer.identity_id,
            232,
            vec!["audio".into()],
            scope,
        );
        let publisher = client(f.client.clone()).await;
        let peer = client(peer_config).await;
        let room = create_room(&publisher, &f.peer.identity_id, "renew-events-create").await;
        let mine = owner(&room, &f.client.identity_id).await;
        let theirs = owner(&room, &f.peer.identity_id).await;
        let caps = json!({"audio":["g.711-mu"]});
        let session = room
            .invite(
                "event-audio-session".parse().unwrap(),
                vec![theirs.clone()],
                sdk::SessionMedium::Voice,
                "conference".into(),
                caps.clone(),
                mutation("event-audio-invite"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        let receiving = peer
            .conversation_by_id(room.id().clone())
            .session_by_id(session.id().clone());
        receiving
            .accept(
                caps,
                sdk::TopologyRevision::try_from(0).unwrap(),
                mutation("event-audio-accept"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap();
        let source = session
            .offer_connection(
                "event-audio-source".parse().unwrap(),
                NonEmpty {
                    first: stream(
                        "event-audio",
                        Medium::Audio,
                        Direction::Sendonly,
                        "g.711-mu",
                    ),
                    rest: vec![],
                },
                mutation("event-audio-source-offer"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        let target = receiving
            .offer_connection(
                "event-audio-target".parse().unwrap(),
                NonEmpty {
                    first: stream(
                        "event-audio",
                        Medium::Audio,
                        Direction::Recvonly,
                        "g.711-mu",
                    ),
                    rest: vec![],
                },
                mutation("event-audio-target-offer"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        target
            .subscribe(
                NonEmpty {
                    first: StreamSelector::Stream("event-audio".parse().unwrap()),
                    rest: vec![],
                },
                revision(1),
                mutation("event-audio-select"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap();
        let source = source.stream_by_id("event-audio".parse().unwrap());
        let target = target.stream_by_id("event-audio".parse().unwrap());
        let mut events = room.events(None, event_wait());
        events.snapshot().await.unwrap();
        let clock = tokio::time::Instant::now();
        let done = sdk::CancellationToken::new();
        let receiving_done = done.clone();
        let publishing = async {
            for i in 0..600u16 {
                let payload = (0..160)
                    .map(|n| if n % 16 < 8 { 0x80 } else { 0 })
                    .collect::<Vec<_>>();
                source
                    .send_rtp(
                        sdk::RtpPacket::new(
                            u32::from(i),
                            media::rtp(
                                0,
                                i,
                                (clock.elapsed().as_micros() * 8 / 1000) as u32,
                                0x65766e74,
                                false,
                                &payload,
                            ),
                        )
                        .unwrap(),
                        options(),
                    )
                    .await
                    .unwrap();
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            done.cancel();
        };
        let listening = async {
            let mut before = vec![];
            let mut after = vec![];
            while !receiving_done.is_cancelled() {
                match target
                    .receive_packet(RequestOptions::new(Duration::from_millis(250)).unwrap())
                    .await
                {
                    Ok(sdk::MediaPacket::Rtp(p)) => {
                        if clock.elapsed() < Duration::from_secs(2) && before.len() < 16000 {
                            before.extend_from_slice(p.payload());
                        }
                        if clock.elapsed() > Duration::from_secs(11) && after.len() < 16000 {
                            after.extend_from_slice(p.payload());
                        }
                    }
                    Ok(_) => (),
                    Err(e) if e.kind == sdk::ErrorKind::Deadline => (),
                    Err(e) => panic!("audio retired during live renewal: {e:?}"),
                }
            }
            assert!(
                before.len() >= 1600 && after.len() >= 1600,
                "decoded media must bracket native event lease renewal"
            );
            media::decoded_audio(f.temp.path(), &before);
            media::decoded_audio(f.temp.path(), &after);
        };
        let mutations = async {
            for index in 0..12 {
                send(
                    &room,
                    NonEmpty {
                        first: mine.clone(),
                        rest: vec![theirs.clone()],
                    },
                    index,
                )
                .await
                .wait(options())
                .await
                .unwrap();
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        };
        let journal = async {
            let mut cursor: Option<Cursor> = None;
            for index in 0..12 {
                let event =
                    until_message(&mut events, &format!("live-event-message-{index}"), None).await;
                assert_eq!(
                    event.operation_id.as_ref().unwrap().as_str(),
                    format!("live-event-send-{index}")
                );
                cursor = Some(event.cursor);
            }
            cursor.unwrap()
        };
        let (_, _, _, cursor) = tokio::join!(publishing, listening, mutations, journal);
        assert_eq!(events.cursor().unwrap(), cursor);
        events.close().await.unwrap();
        peer.close(options()).await.unwrap();
        publisher.close(options()).await.unwrap();
        f.server.shutdown().await.unwrap();
        eprintln!("SDK live renewal {algorithm}:12 original mutations/facts and decoded PCMU before/after10-second lease, same owned control/media peer");
    }
}
#[tokio::test]
async fn public_sdk_durable_event_cursor_survives_restart_without_reusing_the_old_live_lease() {
    logs();
    for algorithm in ["EdDSA", "ES256"] {
        let mut f = Fixture::with_native_end_scope(algorithm, 12).await;
        let first = client(f.client.clone()).await;
        let room = create_room(&first, &f.peer.identity_id, "restart-events-create").await;
        let mine = owner(&room, &f.client.identity_id).await;
        let mut events = room.events(None, event_wait());
        events.snapshot().await.unwrap();
        let original = send(
            &room,
            NonEmpty {
                first: mine.clone(),
                rest: vec![],
            },
            0,
        )
        .await;
        let saved = original.reference_json();
        original.wait(options()).await.unwrap();
        let event = until_message(&mut events, "live-event-message-0", None).await;
        let cursor = event.cursor.as_str().to_owned();
        let cid = room.id().clone();
        events.close().await.unwrap();
        first.close(options()).await.unwrap();
        f.server.shutdown().await.unwrap();
        f.server = RunningCanonicalServer::start(f.config.clone())
            .await
            .unwrap();
        f.client.endpoint = f.server.address;
        let next = client(f.client.clone()).await;
        let room = next.conversation_by_id(cid);
        next.restore(saved, next.operation_kinds().message_send)
            .unwrap()
            .wait(options())
            .await
            .unwrap();
        let mut resumed = room.events(Some(cursor.parse().unwrap()), event_wait());
        assert!(resumed.snapshot().await.unwrap().is_none());
        send(
            &room,
            NonEmpty {
                first: mine,
                rest: vec![],
            },
            1,
        )
        .await
        .wait(options())
        .await
        .unwrap();
        let fresh = until_message(
            &mut resumed,
            "live-event-message-1",
            Some("live-event-message-0"),
        )
        .await;
        assert_ne!(fresh.event_id, event.event_id);
        let db = rusqlite::Connection::open(&f.config.database).unwrap();
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM messages WHERE id='live-event-message-0'",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            1
        );
        resumed.close().await.unwrap();
        next.close(options()).await.unwrap();
        f.server.shutdown().await.unwrap();
        eprintln!("SDK durable journal restart {algorithm}: same original cursor/key, fresh source lease, original operation recovery, no message republish");
    }
}

#[tokio::test]
async fn managed_retained_journal_fact_rechecks_current_credential_before_reexposure() {
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_native_end_scope(algorithm, 16).await;
        let client = client(f.client.clone()).await;
        let room = create_room(&client, &f.peer.identity_id, "retained-authority-room").await;
        let recipient = owner(&room, &f.peer.identity_id).await;
        let managed = sdk::helper::ManagedClient::new(client.clone());
        let poll = |ready| {
            serde_json::from_value(json!({"timeout_ms":4000,"action":{
            "type":"events_poll","reader_id":"retained-authority-reader","conversation_id":room.id(),
            "after":null,"acknowledged":null,"ready":ready}})).unwrap()
        };
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if managed.execute(poll(true)).await.unwrap()["ready"] == true {
                    break;
                }
            }
        })
        .await
        .unwrap();
        send(
            &room,
            NonEmpty {
                first: recipient,
                rest: vec![],
            },
            9,
        )
        .await
        .wait(options())
        .await
        .unwrap();
        let retained = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let value = managed.execute(poll(false)).await.unwrap();
                if !value["event"].is_null() {
                    break value;
                }
            }
        })
        .await
        .unwrap();
        assert!(!retained["event"].is_null());
        let principal = f.server.store.authenticate(&f.client.credential).unwrap();
        let admin = f.server.store.authenticate(&f.admin).unwrap();
        let store = f.server.store.clone();
        tokio::task::spawn_blocking(move || store.revoke(&admin, &principal.credential_id))
            .await
            .unwrap()
            .unwrap();
        // No local acknowledgement, source renewal, sleep or fresh owner: the
        // exact retained fact must not leave this original helper again.
        let refused = managed.execute(poll(false)).await.unwrap_err();
        assert!(
            matches!(
                refused.kind,
                sdk::ErrorKind::Authentication
                    | sdk::ErrorKind::Cancelled
                    | sdk::ErrorKind::TransportUncertain
            ),
            "{refused:?}"
        );
        managed.close(options()).await.unwrap();
        client.close(options()).await.unwrap();
        f.server.shutdown().await.unwrap();
        eprintln!(
            "managed-retained-authority-witness {}",
            json!({"algorithm":algorithm,"retained_fact_not_reexposed":true,"original_owner_joined":true})
        );
    }
}
