//! Signed live batches, ACK boundaries, bootstrap and independent reader custody.
use super::*;
use crate::sdk::{self, Client, Cursor, ErrorKind, RequestOptions};
#[derive(Clone, Copy)]
pub(in crate::canonical) enum Case {
    Bootstrap,
    DropStart,
    Incomplete,
    Renew,
    Duplicate,
    ChangedDuplicate,
    WrongScope,
    WrongWrapper,
    WrongAudience,
    Resync,
    Refused,
    Overflow,
    Bytes,
    Capacity,
    Close,
}
fn options() -> RequestOptions {
    RequestOptions::new(Duration::from_secs(3)).unwrap()
}
async fn read(reader: &mut quinn::RecvStream) -> Option<Envelope> {
    let mut prefix = [0; 4];
    reader.read_exact(&mut prefix).await.ok()?;
    let n = u32::from_be_bytes(prefix) as usize;
    assert!(n > 0 && n <= MAX_ENVELOPE_BYTES);
    let mut bytes = vec![0; n];
    reader.read_exact(&mut bytes).await.ok()?;
    Some(Envelope::parse(&bytes).unwrap())
}
async fn write(
    writer: &mut quinn::SendStream,
    signer: &Sig9421Signer,
    request: &Envelope,
    kind: &str,
    payload: Value,
) {
    let mut frame = envelope(kind, payload);
    frame["cid"] = request.original()["cid"].clone();
    frame["in_reply_to"] = json!(request.id());
    let signed = signer.sign(&frame).unwrap();
    Envelope::from_value(signed.clone()).unwrap();
    let bytes = serde_json::to_vec(&signed).unwrap();
    writer
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .await
        .unwrap();
    writer.write_all(&bytes).await.unwrap();
}
async fn ack(
    writer: &mut quinn::SendStream,
    signer: &Sig9421Signer,
    request: &Envelope,
    cursor: &str,
    case: Case,
) {
    write(writer,signer,request,"ack",json!({"details":{"cursor":cursor,"has_more":false,"resync_required":matches!(case,Case::Resync),"lease":"opaque-lease","expires_at":chrono::Utc::now().timestamp()+if matches!(case,Case::Renew){2}else{10},"live":true}})).await;
}
async fn event(
    writer: &mut quinn::SendStream,
    signer: &Sig9421Signer,
    request: &Envelope,
    index: usize,
    cursor: &str,
    case: Case,
) {
    let cid = request.scope(Scope::Conversation).unwrap();
    let mut fact = json!({"event_id":format!("event-{index}"),"cursor":cursor,"conversation_id":cid,"session_id":null,"connection_id":null,"participant_id":"part_one","operation_id":null,"kind":"changed","revision":1,"source":{"kind":"connect-journal","correlation_id":cid},"observed_at":1,"audience":[{"participant_id":"part_one","membership_epoch":1}],"data":{"kind":"changed","data":{"resource_kind":"conversation","resource_id":cid,"private_body":"never in Debug"}}});
    if matches!(case, Case::Bytes) {
        fact["data"]["data"]["private_body"] = json!("x".repeat(600_000));
    }
    if matches!(case, Case::WrongScope) {
        fact["conversation_id"] = json!("foreign");
    }
    if matches!(case, Case::WrongAudience) {
        fact["audience"][0]["membership_epoch"] = json!(0);
    }
    if matches!(case, Case::ChangedDuplicate) {
        fact["data"]["data"]["private_body"] = json!("changed original immutable fact");
    }
    let p = json!({"event_id":fact["event_id"],"cursor":cursor,"kind":"changed","revision":if matches!(case,Case::WrongWrapper){2}else{1},"data":fact});
    write(writer, signer, request, "conversation.event", p).await;
}
async fn control(writer: &mut quinn::SendStream, signer: &Sig9421Signer, request: &Envelope) {
    assert_eq!(request.kind(), "operation.get");
    assert_eq!(request.original()["payload"]["operation_id"], "control");
    write(writer,signer,request,"operation.updated",json!({"operation_id":"control","command_type":"message.send","record_id":"opr_control","state":"unknown","revision":1})).await;
    write(writer, signer, request, "ack", json!({})).await;
}
pub(super) async fn emit(
    case: Case,
    mut request: Envelope,
    writer: &mut quinn::SendStream,
    reader: &mut quinn::RecvStream,
    signer: &Sig9421Signer,
    signal: &Notify,
) {
    if matches!(case, Case::Bootstrap) {
        assert_eq!(request.kind(), "conversation.get");
        write(writer,signer,&request,"conversation.snapshot",json!({"revision":1,"cursor":"base","items":[{"kind":"conversation","data":{"conversation_id":"conv_one","revision":1,"closed":false,"policy":"persistent"}}],"has_more":true,"next_page":"second-page"})).await;
        write(writer, signer, &request, "ack", json!({})).await;
        request = read(reader).await.unwrap();
        assert_eq!(request.kind(), "conversation.get");
        assert_eq!(request.original()["payload"]["page"], "second-page");
        signal.notify_one();
        signal.notified().await;
        write(writer,signer,&request,"conversation.snapshot",json!({"revision":1,"cursor":"base","items":[{"kind":"participant","data":{"participant_id":"part_one","conversation_id":"conv_one","identity_id":"identity","kind":"human","role":"participant","membership_epoch":1,"active":true,"removal_reason":null}}],"has_more":false})).await;
        write(writer, signer, &request, "ack", json!({})).await;
        request = read(reader).await.unwrap();
    }
    assert_eq!(request.kind(), "conversation.subscribe");
    assert_eq!(request.original()["payload"]["after"], "base");
    assert_eq!(request.original()["payload"]["live"], true);
    assert_eq!(request.original()["payload"]["lease_seconds"], 10);
    if matches!(case, Case::Refused) {
        write(
            writer,
            signer,
            &request,
            "error",
            json!({"code":403,"category":"authorization","reason":"current-subscription-refused"}),
        )
        .await;
    } else {
        if matches!(case, Case::DropStart) {
            event(
                writer,
                signer,
                &request,
                1,
                "cursor-1",
                if matches!(case, Case::ChangedDuplicate) {
                    Case::Duplicate
                } else {
                    case
                },
            )
            .await;
            signal.notify_one();
            signal.notified().await;
        } else if !matches!(
            case,
            Case::Incomplete
                | Case::Overflow
                | Case::Bytes
                | Case::Capacity
                | Case::Close
                | Case::Resync
        ) {
            event(
                writer,
                signer,
                &request,
                1,
                "cursor-1",
                if matches!(case, Case::ChangedDuplicate) {
                    Case::Duplicate
                } else {
                    case
                },
            )
            .await;
        }
        ack(
            writer,
            signer,
            &request,
            if matches!(case, Case::Renew) {
                "ack-through-hidden"
            } else {
                "base"
            },
            case,
        )
        .await;
        if matches!(case, Case::Duplicate | Case::ChangedDuplicate) {
            event(writer, signer, &request, 1, "cursor-duplicate", case).await;
            event(writer, signer, &request, 2, "cursor-2", case).await;
            ack(writer, signer, &request, "ack-2", case).await;
        }
    }
    let mut source = request;
    let mut controls = 0;
    let mut subscriptions = 1;
    while let Some(request) = read(reader).await {
        if request.kind() == "conversation.subscribe" {
            subscriptions += 1;
            if matches!(case, Case::Renew) {
                assert_eq!(subscriptions, 2);
                assert_ne!(request.id(), source.id());
                assert_eq!(request.original()["payload"]["after"], "ack-through-hidden");
                event(writer, signer, &source, 99, "late-retired", Case::Close).await;
                event(writer, signer, &request, 2, "cursor-2", case).await;
                ack(writer, signer, &request, "ack-2", case).await;
            } else if matches!(case, Case::Close | Case::Capacity) {
                event(writer, signer, &source, 99, "late-retired", Case::Close).await;
                event(writer, signer, &request, 2, "cursor-2", case).await;
                ack(writer, signer, &request, "ack-2", case).await;
            } else {
                panic!("Unexpected duplicate subscription for case {subscriptions}");
            }
            source = request;
            continue;
        }
        controls += 1;
        if matches!(case, Case::Incomplete) {
            if controls == 1 {
                event(writer, signer, &source, 1, "cursor-1", case).await;
            }
            if controls == 2 {
                ack(writer, signer, &source, "ack-1", case).await;
            }
        }
        if matches!(case, Case::Overflow) {
            for index in 0..32 {
                let id = (controls - 1) * 32 + index + 1;
                event(writer, signer, &source, id, &format!("cursor-{id}"), case).await;
            }
            ack(
                writer,
                signer,
                &source,
                &format!("through-{controls}"),
                case,
            )
            .await;
        }
        if matches!(case, Case::Bytes) {
            event(
                writer,
                signer,
                &source,
                controls,
                &format!("cursor-{controls}"),
                case,
            )
            .await;
            ack(
                writer,
                signer,
                &source,
                &format!("through-{controls}"),
                case,
            )
            .await;
        }
        control(writer, signer, &request).await;
    }
}
async fn client(case: Case) -> (Client, tokio::task::JoinHandle<()>, Arc<Notify>) {
    let (peer, host, signal) = controlled(Fault::SdkEvents(case)).await;
    (
        Client::from_authenticated_peer(peer, "tenant".parse().unwrap()).unwrap(),
        host,
        signal,
    )
}
fn stream(client: &Client, after: Option<Cursor>, wait: RequestOptions) -> sdk::EventStream {
    client
        .conversation_by_id("conv_one".parse().unwrap())
        .events(after, wait)
}
async fn query(client: &Client) {
    let saved = json!({"schema":"connect-operation-reference/1","operation_id":"control","command":"message.send","scope":{"conversation_id":"conv_one"}});
    assert_eq!(
        client
            .restore(saved, client.operation_kinds().message_send)
            .unwrap()
            .status(options())
            .await
            .unwrap()
            .state,
        sdk::OperationState::Unknown
    );
}
async fn close(client: Client, host: tokio::task::JoinHandle<()>) {
    client.close(options()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), host)
        .await
        .unwrap()
        .unwrap();
}
#[tokio::test]
async fn event_bootstrap_drains_frozen_snapshot_pages_and_survives_a_dropped_wait_before_subscription(
) {
    let (client, host, signal) = client(Case::Bootstrap).await;
    let mut events = stream(&client, None, options());
    {
        let next = events.next();
        tokio::pin!(next);
        tokio::select! {result=&mut next=>panic!("premature snapshot: {result:?}"),_=signal.notified()=>()}
    }
    signal.notify_one();
    let snapshot = events.snapshot().await.unwrap().unwrap();
    assert_eq!(snapshot.facts.len(), 2);
    assert_eq!(snapshot.conversation.participants.len(), 1);
    assert_eq!(events.cursor().unwrap().as_str(), "base");
    let event = events.next().await.unwrap().unwrap();
    assert_eq!(event.event_id, "event-1");
    assert_eq!(event.source.kind, "connect-journal");
    assert_eq!(event.audience[0].membership_epoch, 1);
    assert!(!format!("{event:?}").contains("never in Debug"));
    assert_eq!(events.cursor().unwrap(), event.cursor);
    query(&client).await;
    events.close().await.unwrap();
    close(client, host).await;
}
#[tokio::test]
async fn dropping_initial_event_wait_preserves_the_one_owned_subscription_and_ack_boundary() {
    let (client, host, signal) = client(Case::DropStart).await;
    let mut events = stream(&client, Some("base".parse().unwrap()), options());
    {
        let next = events.next();
        tokio::pin!(next);
        tokio::select! {result=&mut next=>panic!("event before ACK: {result:?}"),_=signal.notified()=>()}
    }
    assert_eq!(events.cursor().unwrap().as_str(), "base");
    signal.notify_one();
    assert_eq!(events.next().await.unwrap().unwrap().event_id, "event-1");
    events.close().await.unwrap();
    close(client, host).await;
}
#[tokio::test]
async fn incomplete_live_batch_and_next_timeout_do_not_advance_cursor_consume_events_or_block_control(
) {
    let (client, host, _) = client(Case::Incomplete).await;
    let mut wait = options();
    wait.timeout = Duration::from_millis(150);
    let mut events = stream(&client, Some("base".parse().unwrap()), wait);
    assert!(events.snapshot().await.unwrap().is_none());
    query(&client).await;
    assert_eq!(events.next().await.unwrap_err().kind, ErrorKind::Deadline);
    assert_eq!(events.cursor().unwrap().as_str(), "base");
    query(&client).await;
    assert_eq!(events.next().await.unwrap().unwrap().event_id, "event-1");
    events.close().await.unwrap();
    close(client, host).await;
}
#[tokio::test]
async fn live_renewal_uses_ack_watermark_keeps_returned_cursor_and_discards_retired_source_frames()
{
    let (client, host, _) = client(Case::Renew).await;
    let mut events = stream(&client, Some("base".parse().unwrap()), options());
    assert_eq!(events.next().await.unwrap().unwrap().event_id, "event-1");
    assert_eq!(events.cursor().unwrap().as_str(), "cursor-1");
    assert_eq!(events.next().await.unwrap().unwrap().event_id, "event-2");
    query(&client).await;
    events.close().await.unwrap();
    close(client, host).await;
}
#[tokio::test]
async fn duplicate_journal_identity_ignores_new_opaque_cursor_but_refuses_changed_immutable_fact() {
    let (client, host, _) = client(Case::Duplicate).await;
    let mut events = stream(&client, Some("base".parse().unwrap()), options());
    assert_eq!(events.next().await.unwrap().unwrap().event_id, "event-1");
    assert_eq!(events.next().await.unwrap().unwrap().event_id, "event-2");
    events.close().await.unwrap();
    close(client, host).await;
    let (client, host, _) = self::client(Case::ChangedDuplicate).await;
    let mut events = stream(&client, Some("base".parse().unwrap()), options());
    let result = events.next().await;
    let failure = match result {
        Err(error) => error,
        Ok(Some(first)) => {
            assert_eq!(first.event_id, "event-1");
            events.next().await.unwrap_err()
        }
        other => panic!("unexpected changed replay: {other:?}"),
    };
    assert_eq!(failure.kind, ErrorKind::Protocol);
    query(&client).await;
    events.close().await.unwrap();
    close(client, host).await;
}
#[tokio::test]
async fn event_resync_authority_and_scope_shape_failures_retire_only_the_reader_with_its_original_cursor(
) {
    for case in [
        Case::WrongScope,
        Case::WrongWrapper,
        Case::WrongAudience,
        Case::Resync,
        Case::Refused,
    ] {
        let (client, host, _) = client(case).await;
        let mut events = stream(&client, Some("base".parse().unwrap()), options());
        let error = events.next().await.unwrap_err();
        assert_eq!(
            error.kind,
            if matches!(case, Case::Resync) {
                ErrorKind::ResyncRequired
            } else if matches!(case, Case::Refused) {
                ErrorKind::Authentication
            } else {
                ErrorKind::Protocol
            }
        );
        assert_eq!(events.cursor().unwrap().as_str(), "base");
        query(&client).await;
        events.close().await.unwrap();
        close(client, host).await;
    }
}
#[tokio::test]
async fn event_count_and_byte_overflow_report_resync_without_advancing_unreturned_cursor_or_stalling_control(
) {
    for case in [Case::Overflow, Case::Bytes] {
        let (client, host, _) = client(case).await;
        let mut events = stream(&client, Some("base".parse().unwrap()), options());
        events.snapshot().await.unwrap();
        for _ in 0..if matches!(case, Case::Overflow) { 9 } else { 4 } {
            query(&client).await;
        }
        assert_eq!(
            events.next().await.unwrap_err().kind,
            ErrorKind::ResyncRequired
        );
        assert_eq!(events.cursor().unwrap().as_str(), "base");
        events.close().await.unwrap();
        close(client, host).await;
    }
}
#[tokio::test]
async fn event_reader_conflict_close_replacement_and_cancellation_preserve_other_work() {
    let (client, host, _) = client(Case::Close).await;
    let mut first = stream(&client, Some("base".parse().unwrap()), options());
    first.snapshot().await.unwrap();
    let mut second = stream(&client, Some("base".parse().unwrap()), options());
    assert_eq!(
        second.snapshot().await.unwrap_err().kind,
        ErrorKind::Conflict
    );
    second.close().await.unwrap();
    first.close().await.unwrap();
    assert!(first.next().await.unwrap().is_none());
    let wait = options();
    let signal = wait.signal.clone();
    let mut third = stream(&client, Some("base".parse().unwrap()), wait);
    assert_eq!(third.next().await.unwrap().unwrap().event_id, "event-2");
    signal.cancel();
    assert_eq!(third.next().await.unwrap_err().kind, ErrorKind::Cancelled);
    query(&client).await;
    third.close().await.unwrap();
    close(client, host).await;
}

#[tokio::test]
async fn eight_live_readers_are_bounded_and_closing_one_releases_capacity_without_closing_the_peer()
{
    let (client, host, _) = client(Case::Capacity).await;
    let mut readers = Vec::new();
    for index in 0..8 {
        let cid = if index == 0 {
            "conv_one".to_owned()
        } else {
            format!("room-{index}")
        };
        let mut reader = client
            .conversation_by_id(cid.parse().unwrap())
            .events(Some("base".parse().unwrap()), options());
        reader.snapshot().await.unwrap();
        readers.push(reader);
    }
    let room = client.conversation_by_id("ninth-room".parse().unwrap());
    let mut refused = room.events(Some("base".parse().unwrap()), options());
    assert_eq!(
        refused.snapshot().await.unwrap_err().kind,
        ErrorKind::Configuration
    );
    refused.close().await.unwrap();
    readers[0].close().await.unwrap();
    let mut replacement = room.events(Some("base".parse().unwrap()), options());
    assert_eq!(
        replacement.next().await.unwrap().unwrap().event_id,
        "event-2"
    );
    query(&client).await;
    for reader in &mut readers {
        reader.close().await.unwrap();
    }
    replacement.close().await.unwrap();
    close(client, host).await;
}
