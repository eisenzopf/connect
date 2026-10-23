use super::*;
use connect_protocol::{events::EventQuery, receipts::*};
fn read(cid: &str, message: &str, operation: &str, participant: Option<&str>) -> Value {
    let mut value = env(
        "message.read",
        json!({"msg_id":message,"read_at":"1970-01-01T00:00:01Z"}),
        Some(operation),
    );
    value["cid"] = json!(cid);
    if let Some(p) = participant {
        value["payload"]["by_participant"] = json!(p);
    }
    value
}
fn subscribe(cid: &str, after: Option<&str>, live: bool) -> Value {
    let mut value = env(
        "conversation.subscribe",
        json!({"live":live,"limit":127,"lease_seconds":30}),
        None,
    );
    value["cid"] = json!(cid);
    if let Some(cursor) = after {
        value["payload"]["after"] = json!(cursor);
    }
    value
}
fn counts(f: &agent::Fixture, cid: &str) -> (i64, i64) {
    let c = rusqlite::Connection::open(f.server.store.path()).unwrap();
    (c.query_row("SELECT COUNT(*) FROM journal WHERE conversation_id=?1 AND kind='message_receipt'",[cid],|r|r.get(0)).unwrap(),c.query_row("SELECT COUNT(*) FROM operations WHERE conversation_id=?1 AND json_extract(intent_json,'$.op')='message_receipt'",[cid],|r|r.get(0)).unwrap())
}
async fn receipt_hint(wire: &mut Wire, kind: &str, request: &Value) -> Value {
    let hint = wire.read().await;
    assert_eq!(hint.kind(), kind);
    assert_eq!(hint.original()["in_reply_to"], request["id"]);
    let event = wire.read().await;
    assert_eq!(event.kind(), "conversation.event");
    assert_eq!(event.original()["in_reply_to"], request["id"]);
    assert_eq!(
        hint.original()["payload"]["evidence"]["event_id"],
        event.original()["payload"]["event_id"]
    );
    assert_eq!(
        hint.original()["payload"]["evidence"]["provenance"],
        "client_reported"
    );
    assert!(
        hint.original().get("operation_id").is_none(),
        "another actor's operation correlation is private"
    );
    assert_eq!(wire.read().await.kind(), "ack");
    hint.original().clone()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_canonical_read_reports_share_native_http_replay_and_project_only_opted_in_receipts()
{
    let f = agent::Fixture::start().await;
    let sender_config = f.issue_as_operator(&f.agent, StoreScope::collaboration(vec!["*".into()]));
    let receiver_config = f.issue_as_operator(&f.peer, StoreScope::collaboration(vec!["*".into()]));
    let mut sender = ConnectClient::open(sender_config.clone()).await.unwrap();
    let mut receiver = ConnectClient::open(receiver_config.clone()).await.unwrap();
    let room = sender
        .create(
            "canonical-read-room",
            vec![receiver_config.identity_id.clone()],
        )
        .await
        .unwrap();
    let message = sender
        .send("canonical-read-source", &room.id, "receipt subject")
        .await
        .unwrap()
        .message_id;
    let membership = f
        .server
        .store
        .membership_snapshot(&receiver_config.credential_id, &room.id)
        .unwrap();
    let recipient = membership
        .participants
        .iter()
        .find(|p| p.identity_id == receiver_config.identity_id)
        .unwrap()
        .participant_id
        .clone();
    let sender_pid = membership
        .participants
        .iter()
        .find(|p| p.identity_id == sender_config.identity_id)
        .unwrap()
        .participant_id
        .clone();
    let mut observing = Wire::open(&f, &sender_config).await;
    let subscription = subscribe(&room.id, None, true);
    assert_eq!(
        observing.exchange(subscription.clone()).await.kind(),
        "conversation.snapshot"
    );
    let boundary = observing.read().await;
    let cursor = boundary.original()["payload"]["details"]["cursor"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut reporting = Wire::open(&f, &receiver_config).await;
    let first = reporting
        .exchange(read(
            &room.id,
            &message,
            "canonical-read-delivery",
            Some(&recipient),
        ))
        .await;
    assert_eq!(first.kind(), "operation.updated");
    assert_eq!(first.original()["payload"]["state"], "completed");
    let delivery_record = first.original()["payload"].clone();
    assert_eq!(
        reporting.read().await.original()["payload"]["details"]["recorded"],
        "delivered"
    );
    let delivered = receipt_hint(&mut observing, "message.delivered", &subscription).await;
    assert_ne!(delivered["payload"]["delivered_at"], "1970-01-01T00:00:01Z");
    receiver
        .set_receipt_preferences("canonical-read-opt-in", &room.id, true, true)
        .await
        .unwrap();
    let before_opt_in_replay = counts(&f, &room.id);
    assert_eq!(
        reporting
            .exchange(read(&room.id, &message, "canonical-read-delivery", None))
            .await
            .original()["payload"],
        delivery_record
    );
    assert_eq!(
        reporting.read().await.original()["payload"]["details"]["recorded"],
        "delivered"
    );
    assert_eq!(counts(&f, &room.id), before_opt_in_replay);
    let first = reporting
        .exchange(read(
            &room.id,
            &message,
            "canonical-read-explicit",
            Some(&recipient),
        ))
        .await;
    let record = first.original()["payload"].clone();
    assert_eq!(
        reporting.read().await.original()["payload"]["details"]["recorded"],
        "read"
    );
    let hint = receipt_hint(&mut observing, "message.read", &subscription).await;
    assert_eq!(hint["payload"]["by_participant"], recipient);
    assert_ne!(hint["payload"]["read_at"], "1970-01-01T00:00:01Z");
    let before = counts(&f, &room.id);
    let native = receiver
        .message_receipt(
            "canonical-read-explicit",
            &room.id,
            &message,
            ClientReceiptState::Read,
        )
        .await
        .unwrap();
    assert_eq!(native.recorded, Some(DeliveryState::Read));
    let request = Operation::MessageReceipt {
        request_id: "canonical-read-explicit".into(),
        conversation_id: room.id.clone(),
        message_id: message.clone(),
        state: ClientReceiptState::Read,
    };
    let response = reqwest::Client::new()
        .post(format!(
            "http://{}/v1/operations",
            f.server.http_address.unwrap()
        ))
        .bearer_auth(&receiver_config.credential)
        .json(&request)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    if let Some(directory) = std::env::var_os("CONNECT_GATE6_HTTP_SAMPLES") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("canonical-read-http-replay.json"),serde_json::to_vec_pretty(&json!({"api":"attachments","route":"/v1/operations","status":200,"request":request,"response":body})).unwrap()).unwrap();
    }
    assert_eq!(
        serde_json::from_value::<Response>(body)
            .unwrap()
            .result
            .unwrap(),
        Outcome::ReceiptAcknowledged(native)
    );
    assert_eq!(counts(&f, &room.id), before);
    reporting.close().await;
    let rotated = f.issue_as_operator(&f.peer, StoreScope::collaboration(vec!["*".into()]));
    let mut reporting = Wire::open(&f, &rotated).await;
    let mut retry = read(&room.id, &message, "canonical-read-explicit", None);
    retry["payload"]["read_at"] = json!(chrono::Utc::now());
    assert_eq!(
        reporting.exchange(retry).await.original()["payload"],
        record
    );
    assert_eq!(reporting.read().await.kind(), "ack");
    assert_eq!(counts(&f, &room.id), before);
    let spoof = reporting
        .exchange(read(
            &room.id,
            &message,
            "canonical-read-spoof",
            Some(&sender_pid),
        ))
        .await;
    assert_eq!(spoof.original()["payload"]["code"], 403);
    let mut forge = env(
        "message.delivered",
        json!({"msg_id":message,"to_participant":recipient,"delivered_at":chrono::Utc::now()}),
        None,
    );
    forge["cid"] = json!(room.id);
    assert_eq!(
        reporting.exchange(forge).await.original()["payload"]["code"],
        501
    );
    assert_eq!(counts(&f, &room.id), before);
    receiver
        .set_receipt_preferences("canonical-read-private", &room.id, false, false)
        .await
        .unwrap();
    assert_eq!(
        reporting
            .exchange(read(&room.id, &message, "canonical-read-suppressed", None))
            .await
            .kind(),
        "operation.updated"
    );
    assert!(reporting.read().await.original()["payload"]["details"]["recorded"].is_null());
    let after = counts(&f, &room.id);
    assert_eq!(after.0, before.0);
    // Current opt-out filters retained facts before both generic and typed output.
    let catchup = subscribe(&room.id, Some(&cursor), false);
    let result = observing.exchange(catchup).await;
    assert_eq!(result.kind(), "ack");
    assert!(sender
        .message_receipts(&room.id, &message, None, 100)
        .await
        .unwrap()
        .receipts
        .is_empty());
    reporting.close().await;
    observing.close().await;
    f.close().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_canonical_receipt_pages_preserve_all_cursors_across_typed_hint_batch_boundaries() {
    let f = agent::Fixture::start().await;
    let sender_config = f.issue_as_operator(&f.agent, StoreScope::collaboration(vec!["*".into()]));
    let receiver_config = f.issue_as_operator(&f.peer, StoreScope::collaboration(vec!["*".into()]));
    let mut sender = ConnectClient::open(sender_config.clone()).await.unwrap();
    let mut receiver = ConnectClient::open(receiver_config.clone()).await.unwrap();
    let room = sender
        .create(
            "receipt-page-budget",
            vec![receiver_config.identity_id.clone()],
        )
        .await
        .unwrap();
    receiver
        .set_receipt_preferences("receipt-page-opt-in", &room.id, true, true)
        .await
        .unwrap();
    let mut messages = vec![];
    for i in 0..64 {
        messages.push(
            sender
                .send(
                    &format!("receipt-page-source-{i}"),
                    &room.id,
                    "bounded receipt",
                )
                .await
                .unwrap()
                .message_id,
        );
    }
    let mut wire = Wire::open(&f, &sender_config).await;
    assert_eq!(
        wire.exchange(subscribe(&room.id, None, false)).await.kind(),
        "conversation.snapshot"
    );
    let boundary = wire.read().await;
    let mut cursor = boundary.original()["payload"]["details"]["cursor"]
        .as_str()
        .unwrap()
        .to_owned();
    for (i, message) in messages.iter().enumerate() {
        receiver
            .message_receipt(
                &format!("receipt-page-read-{i}"),
                &room.id,
                message,
                ClientReceiptState::Read,
            )
            .await
            .unwrap();
    }
    let mut seen = std::collections::BTreeSet::new();
    for expected in [63usize, 1] {
        let request = subscribe(&room.id, Some(&cursor), false);
        wire.send(&request, true).await;
        for _ in 0..expected {
            let hint = wire.read().await;
            assert_eq!(hint.kind(), "message.read");
            let event = wire.read().await;
            assert_eq!(event.kind(), "conversation.event");
            assert_eq!(hint.original()["in_reply_to"], request["id"]);
            assert_eq!(
                hint.original()["payload"]["evidence"]["event_id"],
                event.original()["payload"]["event_id"]
            );
            assert!(seen.insert(
                event.original()["payload"]["event_id"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            ));
        }
        let ack = wire.read().await;
        assert_eq!(ack.kind(), "ack");
        assert_eq!(
            ack.original()["payload"]["details"]["has_more"],
            expected == 63
        );
        cursor = ack.original()["payload"]["details"]["cursor"]
            .as_str()
            .unwrap()
            .to_owned();
    }
    assert_eq!(seen.len(), 64);
    let page = f
        .server
        .store
        .conversation_events(
            &sender_config.credential_id,
            &room.id,
            EventQuery {
                after: Some(cursor),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(page.events.is_empty());
    wire.close().await;
    f.close().await;
}
