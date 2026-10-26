//! Advanced public finite exchanges against the actual signed listener and Store.
use super::*;
fn envelope(kind: &str, cid: &str, payload: Value, operation: Option<&str>) -> Value {
    let mut v = json!({"v":2,"id":"saved-physical-attempt","ts":"2026-01-01T00:00:00Z","type":kind,"cid":cid,"payload":payload});
    if let Some(id) = operation {
        v["operation_id"] = json!(id);
    }
    v
}
async fn exchange(client: &Client, value: Value) -> sdk::Result<Vec<Value>> {
    Ok(client
        .raw()
        .request(client.raw().validate(value)?, options())
        .await?
        .into_iter()
        .map(|e| e.to_json())
        .collect())
}
async fn room(client: &Client, peer: &str, id: &str) -> sdk::Conversation {
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
fn send(cid: &str, recipient: &str) -> Value {
    let mut draft = connect_client::MessageDraft::text_all("raw SDK original body");
    draft.msg_id = Some("raw-sdk-message".into());
    draft.to = connect_client::MessageAudience::Participants(vec![recipient.into()]);
    let mut payload = serde_json::to_value(draft).unwrap();
    payload["vendor"] = json!({"intent":"preserved optional metadata"});
    envelope("message.send", cid, payload, Some("raw-sdk-send"))
}
#[tokio::test]
async fn public_sdk_raw_original_intent_status_and_finite_journal_survive_restart() {
    for algorithm in ["EdDSA", "ES256"] {
        let mut f = Fixture::with_native_end_scope(algorithm, 16).await;
        let client = Client::connect(f.client.clone(), "tnt_default".parse().unwrap())
            .await
            .unwrap();
        let room = room(&client, &f.peer.identity_id, "raw-sdk-create").await;
        let cid = room.id().as_str().to_owned();
        let mine = room
            .refresh(options())
            .await
            .unwrap()
            .participants
            .into_iter()
            .find(|p| p.identity_id.as_str() == f.client.identity_id)
            .unwrap()
            .id;
        let snapshot = exchange(
            &client,
            envelope("conversation.get", &cid, json!({"limit":100}), None),
        )
        .await
        .unwrap();
        let baseline = snapshot
            .iter()
            .find(|v| v["type"] == "conversation.snapshot")
            .unwrap()["payload"]["cursor"]
            .as_str()
            .unwrap()
            .to_owned();
        let original = send(&cid, mine.as_str());
        let first = exchange(&client, original.clone()).await.unwrap();
        let again = exchange(&client, original.clone()).await.unwrap();
        assert_eq!(
            first.last().unwrap()["payload"]["details"],
            again.last().unwrap()["payload"]["details"]
        );
        assert_ne!(
            first.last().unwrap()["in_reply_to"],
            again.last().unwrap()["in_reply_to"]
        );
        assert!(first.iter().all(|v| v.get("signature").is_some()));
        let mut changed = original.clone();
        changed["payload"]["body"] = json!("changed intent");
        let e = exchange(&client, changed).await.unwrap_err();
        assert_eq!(
            (e.kind, e.certainty),
            (sdk::ErrorKind::Conflict, sdk::Certainty::Refused)
        );
        assert_eq!(e.operation.unwrap().operation_id.as_str(), "raw-sdk-send");
        let query = envelope(
            "operation.get",
            &cid,
            json!({"operation_id":"raw-sdk-send"}),
            None,
        );
        let current = exchange(&client, query.clone()).await.unwrap();
        assert!(current
            .iter()
            .any(|v| v["type"] == "operation.updated" && v["payload"]["state"] == "completed"));
        let replay = envelope(
            "conversation.subscribe",
            &cid,
            json!({"after":baseline,"live":false,"limit":100}),
            None,
        );
        let events = exchange(&client, replay.clone()).await.unwrap();
        let fact = events
            .iter()
            .find(|v| {
                v["type"] == "conversation.event"
                    && v["payload"]["data"]["data"]["kind"] == "message"
            })
            .unwrap();
        assert_eq!(
            fact["payload"]["data"]["data"]["data"]["id"],
            "raw-sdk-message"
        );
        let owner = f
            .server
            .store
            .authenticate(&f.client.credential)
            .unwrap()
            .credential_id;
        let actual = f
            .server
            .store
            .conversation_events(
                &owner,
                &cid,
                connect_protocol::events::EventQuery {
                    after: Some(baseline),
                    limit: Some(100),
                    ..Default::default()
                },
            )
            .unwrap();
        let actual = actual
            .events
            .iter()
            .find(|e| e.event_id == fact["payload"]["event_id"])
            .unwrap();
        assert_eq!(
            fact["payload"]["data"],
            serde_json::to_value(actual).unwrap()
        );
        client.close(options()).await.unwrap();
        f.server.shutdown().await.unwrap();
        f.server = RunningCanonicalServer::start(f.config.clone())
            .await
            .unwrap();
        f.client.endpoint = f.server.address;
        let fresh = Client::connect(f.client.clone(), "tnt_default".parse().unwrap())
            .await
            .unwrap();
        let restored = exchange(&fresh, query).await.unwrap();
        assert_eq!(current[0]["payload"], restored[0]["payload"]);
        let repeated = exchange(&fresh, original).await.unwrap();
        assert_eq!(
            first.last().unwrap()["payload"]["details"],
            repeated.last().unwrap()["payload"]["details"]
        );
        let resumed = exchange(&fresh, replay).await.unwrap();
        assert!(resumed
            .iter()
            .any(|v| v["payload"]["event_id"] == fact["payload"]["event_id"]));
        let db = rusqlite::Connection::open(&f.config.database).unwrap();
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM messages WHERE id='raw-sdk-message'",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            1
        );
        fresh.close(options()).await.unwrap();
        f.server.shutdown().await.unwrap();
        eprintln!("SDK raw {algorithm}: exact intent replay, changed-intent refusal, signed status, original journal and restart with one durable message");
    }
}
#[tokio::test]
async fn public_sdk_raw_current_membership_is_required_for_mutation_status_and_events() {
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_native_end_scope(algorithm, 16).await;
        let config = super::super::canonical_native_connections::party_with_scope(
            &f,
            &f.peer.identity_id,
            241,
            vec!["audio".into()],
            Scope::collaboration(vec!["*".into()]),
        );
        let owner = Client::connect(f.client.clone(), "tnt_default".parse().unwrap())
            .await
            .unwrap();
        let peer = Client::connect(config, "tnt_default".parse().unwrap())
            .await
            .unwrap();
        let room = room(&owner, &f.peer.identity_id, "raw-sdk-authority-create").await;
        let cid = room.id().as_str();
        let participant = room
            .refresh(options())
            .await
            .unwrap()
            .participants
            .into_iter()
            .find(|p| p.identity_id.as_str() == f.peer.identity_id)
            .unwrap()
            .id;
        let snapshot = exchange(
            &peer,
            envelope("conversation.get", cid, json!({"limit":100}), None),
        )
        .await
        .unwrap();
        let cursor = snapshot[0]["payload"]["cursor"].clone();
        let original = send(cid, participant.as_str());
        exchange(&peer, original.clone()).await.unwrap();
        let current = room.refresh(options()).await.unwrap();
        room.participant_by_id(participant)
            .remove(current.revision, mutation("raw-sdk-remove"))
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap();
        let mut fresh = original.clone();
        fresh["operation_id"] = json!("raw-sdk-after-removal");
        fresh["payload"]["msg_id"] = json!("raw-sdk-forbidden-message");
        for request in [
            original,
            fresh,
            envelope(
                "operation.get",
                cid,
                json!({"operation_id":"raw-sdk-send"}),
                None,
            ),
            envelope(
                "conversation.subscribe",
                cid,
                json!({"after":cursor,"live":false,"limit":100}),
                None,
            ),
        ] {
            let e = exchange(&peer, request).await.unwrap_err();
            assert_eq!(e.kind, sdk::ErrorKind::Authentication);
            assert_eq!(e.certainty, sdk::Certainty::Refused);
        }
        let db = rusqlite::Connection::open(&f.config.database).unwrap();
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM messages WHERE id='raw-sdk-forbidden-message'",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            0
        );
        peer.close(options()).await.unwrap();
        owner.close(options()).await.unwrap();
        f.server.shutdown().await.unwrap();
    }
}
