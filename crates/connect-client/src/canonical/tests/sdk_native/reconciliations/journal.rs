//! No dedicated hint or native callback: only a resumed durable operation fact.
use super::*;
#[derive(Clone, Copy)]
pub(in crate::canonical) enum Case {
    Normal,
    NoCapture,
    EqualNoCapture,
    Duplicate,
    DelayedAck,
    CancelledWait,
    Refused,
    WrongRecord,
    WrongBinding,
    ForeignParent,
    ForeignTarget,
    ResourceChange,
    Unrelated,
}
async fn page(
    case: Case,
    writer: &mut quinn::SendStream,
    reader: &mut quinn::RecvStream,
    signer: &Sig9421Signer,
    request: &Envelope,
) {
    assert_eq!(request.original()["payload"]["after"], "journal-base");
    let fact = json!({"event_id":"durable-operation-event","cursor":"journal-after","conversation_id":"conv_one","session_id":"session_one","connection_id":null,"participant_id":null,"operation_id":"auto-selection","kind":"operation_updated","revision":1,"source":{"kind":"connect-journal","correlation_id":"conv_one"},"observed_at":1,"audience":[],"data":{"kind":"operation","data":{"operation_id":"auto-selection","record_id":"opr_auto","state":"completed"}}});
    let mut frame = envelope(
        "conversation.event",
        json!({"event_id":"durable-operation-event","cursor":"journal-after","kind":"operation_updated","revision":1,"data":fact}),
    );
    frame["cid"] = json!("conv_one");
    frame["sid"] = json!("session_one");
    frame["operation_id"] = json!("auto-selection");
    frame["in_reply_to"] = json!(request.id());
    for _ in 0..if matches!(case, Case::Duplicate) {
        2
    } else {
        1
    } {
        frame["id"] = envelope("conversation.event", json!({}))["id"].clone();
        frame["ts"] = json!(chrono::Utc::now());
        let bytes = serde_json::to_vec(&signer.sign(&frame).unwrap()).unwrap();
        Envelope::parse(&bytes).unwrap();
        writer
            .write_all(&(bytes.len() as u32).to_be_bytes())
            .await
            .unwrap();
        writer.write_all(&bytes).await.unwrap();
    }
    if matches!(case, Case::DelayedAck) {
        assert!(
            tokio::time::timeout(Duration::from_millis(80), selections::maybe_read(reader))
                .await
                .is_err(),
            "no discovery before complete live batch ACK"
        );
    }
    write(writer,signer,request,"ack",json!({"details":{"cursor":"journal-after","has_more":false,"resync_required":false,"lease":"journal-lease","expires_at":chrono::Utc::now().timestamp()+10,"live":true}}),true).await;
}
async fn current(
    case: Case,
    index: usize,
    writer: &mut quinn::SendStream,
    signer: &Sig9421Signer,
    request: &Envelope,
) {
    if matches!(case, Case::Refused) && index == 1 {
        write(
            writer,
            signer,
            request,
            "error",
            json!({"code":403,"category":"authorization","reason":"current read refused"}),
            true,
        )
        .await;
        return;
    }
    let base = match case {
        Case::NoCapture => super::Case::RouteNoCapture,
        Case::EqualNoCapture => super::Case::RouteEqualNoCapture,
        Case::ResourceChange => super::Case::Normal,
        _ => super::Case::Route,
    };
    let mut p = json!({"operation_id":"auto-selection","record_id":"opr_auto","command_type":"stream.reconcile","expected_revision":2,"subscription_record_id":"opr_selector","state":"completed","revision":1,"result":captured(base)});
    match case {
        Case::WrongRecord if index == 1 => p["record_id"] = json!("opr_changed"),
        Case::WrongBinding if index == 1 => {
            p["result"]["stream_bindings"][1]["stream_local_id"] = json!(16)
        }
        Case::ForeignParent => {
            p["subscription_record_id"] = json!("opr_unissued");
            p["result"]["subscription_record_id"] = json!("opr_unissued");
        }
        Case::ForeignTarget => p["result"]["connection_id"] = json!("foreign-connection"),
        Case::Unrelated => {
            p["command_type"] = json!("session.update");
            p["result"] = json!({});
        }
        _ => (),
    }
    write(writer, signer, request, "operation.updated", p, true).await;
    write(writer, signer, request, "ack", json!({}), true).await;
}
pub(super) async fn emit(
    case: Case,
    reader: &mut quinn::RecvStream,
    writer: &mut quinn::SendStream,
    signer: &Sig9421Signer,
    signal: &Notify,
) {
    let mut queries = 0;
    let mut late: Option<Envelope> = None;
    while let Some(request) = selections::maybe_read(reader).await {
        match request.kind() {
            "stream.subscribe" => {
                reply(
                    writer,
                    signer,
                    &request,
                    true,
                    "completed",
                    super::Case::Route,
                    false,
                )
                .await
            }
            "conversation.subscribe" => page(case, writer, reader, signer, &request).await,
            "operation.get" if request.original()["payload"]["operation_id"] == "selector" => {
                reply(
                    writer,
                    signer,
                    &request,
                    true,
                    "completed",
                    super::Case::Route,
                    false,
                )
                .await
            }
            "operation.get" => {
                assert_eq!(
                    request.original()["payload"]["operation_id"],
                    "auto-selection"
                );
                assert_eq!(request.scope(Scope::Conversation), Some("conv_one"));
                assert_eq!(request.scope(Scope::Session), Some("session_one"));
                assert!(request.scope(Scope::Connection).is_none());
                assert!(request.scope(Scope::Operation).is_none());
                queries += 1;
                if matches!(case, Case::CancelledWait) && queries == 1 {
                    late = Some(request);
                    signal.notify_one();
                    continue;
                }
                if let Some(previous) = late.take() {
                    assert_ne!(previous.id(), request.id());
                    current(case, 1, writer, signer, &previous).await;
                }
                current(case, queries, writer, signer, &request).await;
            }
            _ => panic!(
                "journal catch-up issued a mutation or native confirmation: {}",
                request.kind()
            ),
        }
    }
    assert_eq!(
        queries,
        if matches!(
            case,
            Case::CancelledWait | Case::Refused | Case::WrongRecord | Case::WrongBinding
        ) {
            2
        } else {
            1
        }
    );
}
async fn ready(
    case: Case,
) -> (
    Client,
    sdk::Connection,
    sdk::EventStream,
    sdk::Operation<sdk::StreamSelectionReceipt>,
    tokio::task::JoinHandle<()>,
    Arc<Notify>,
) {
    let (client, c, selector, host, signal) = super::ready(super::Case::Journal(case)).await;
    let mut events = client
        .conversation_by_id("conv_one".parse().unwrap())
        .events(Some("journal-base".parse().unwrap()), options());
    assert!(events.snapshot().await.unwrap().is_none());
    (client, c, events, selector, host, signal)
}
async fn finish(
    client: Client,
    mut events: sdk::EventStream,
    selector: sdk::Operation<sdk::StreamSelectionReceipt>,
    host: tokio::task::JoinHandle<()>,
) {
    selector.wait(options()).await.unwrap();
    events.close().await.unwrap();
    close(client, host).await;
}
#[tokio::test]
async fn resumed_durable_operation_facts_catch_up_owned_routes_without_any_live_hint_or_parameter_answer(
) {
    for case in [
        Case::Normal,
        Case::NoCapture,
        Case::EqualNoCapture,
        Case::Duplicate,
    ] {
        let (client, c, mut events, selector, host, _) = ready(case).await;
        assert!(c.native_reconciliation().unwrap().is_none());
        let event = events.next().await.unwrap().unwrap();
        assert_eq!(event.event_id, "durable-operation-event");
        assert_eq!(events.cursor().unwrap().as_str(), "journal-after");
        let state = c.native_reconciliation().unwrap().unwrap();
        assert_eq!(state.phase, Phase::Recorded);
        assert_eq!(state.key.operation_id.as_str(), "auto-selection");
        assert_eq!(
            state.bindings_adopted,
            !matches!(case, Case::NoCapture | Case::EqualNoCapture)
        );
        let native = c.native_negotiation().unwrap().unwrap();
        assert_eq!(native.streams.len(), 3);
        assert_eq!(
            native.stream_revision.get(),
            if matches!(case, Case::EqualNoCapture) {
                2
            } else {
                3
            }
        );
        if matches!(case, Case::Duplicate) {
            assert!(
                tokio::time::timeout(Duration::from_millis(20), events.next())
                    .await
                    .is_err()
            );
        }
        finish(client, events, selector, host).await;
    }
}
#[tokio::test]
async fn cancelled_event_next_keeps_the_fact_and_cursor_until_its_current_journal_read_finishes() {
    let (client, c, mut events, selector, host, signal) = ready(Case::CancelledWait).await;
    let mut next = Box::pin(events.next());
    tokio::select! {result=&mut next=>panic!("metadata read finished before release: {result:?}"),_=signal.notified()=>()};
    drop(next);
    assert_eq!(events.cursor().unwrap().as_str(), "journal-base");
    assert!(c.native_reconciliation().unwrap().is_none());
    let event = events.next().await.unwrap().unwrap();
    assert_eq!(event.cursor.as_str(), "journal-after");
    assert_eq!(
        c.native_negotiation()
            .unwrap()
            .unwrap()
            .stream_revision
            .get(),
        3
    );
    finish(client, events, selector, host).await;
}
#[tokio::test]
async fn failed_or_contradictory_current_reads_never_advance_the_event_cursor_and_can_be_retried() {
    for (case, kind) in [
        (Case::Refused, sdk::ErrorKind::Authentication),
        (Case::WrongRecord, sdk::ErrorKind::Protocol),
        (Case::WrongBinding, sdk::ErrorKind::ResyncRequired),
    ] {
        let (client, c, mut events, selector, host, _) = ready(case).await;
        assert_eq!(events.next().await.unwrap_err().kind, kind);
        assert_eq!(events.cursor().unwrap().as_str(), "journal-base");
        assert!(c.native_reconciliation().unwrap().is_none());
        assert_eq!(
            c.native_negotiation()
                .unwrap()
                .unwrap()
                .stream_revision
                .get(),
            2
        );
        events.next().await.unwrap().unwrap();
        assert_eq!(
            c.native_negotiation()
                .unwrap()
                .unwrap()
                .stream_revision
                .get(),
            3
        );
        finish(client, events, selector, host).await;
    }
}
#[tokio::test]
async fn unrelated_foreign_or_resource_replacement_facts_cannot_acquire_or_replace_owned_native_parameters(
) {
    for case in [
        Case::ForeignParent,
        Case::ForeignTarget,
        Case::ResourceChange,
        Case::Unrelated,
    ] {
        let (client, c, mut events, selector, host, _) = ready(case).await;
        events.next().await.unwrap().unwrap();
        assert!(c.native_reconciliation().unwrap().is_none());
        assert_eq!(
            c.native_negotiation()
                .unwrap()
                .unwrap()
                .stream_revision
                .get(),
            2
        );
        assert_eq!(count(&c), 3);
        finish(client, events, selector, host).await;
    }
}
#[tokio::test]
async fn durable_facts_wait_for_the_complete_live_batch_ack_before_native_discovery() {
    let (client, c, mut events, selector, host, _) = ready(Case::DelayedAck).await;
    events.next().await.unwrap().unwrap();
    assert_eq!(
        c.native_negotiation()
            .unwrap()
            .unwrap()
            .stream_revision
            .get(),
        3
    );
    finish(client, events, selector, host).await;
}
