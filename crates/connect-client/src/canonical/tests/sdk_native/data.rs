//! Signed native data shares the control reader without borrowing fresh custody.
use super::*;
use sdk::{Certainty, DataAcceptanceBoundary, DataPayload, ErrorKind};
#[derive(Clone, Copy)]
pub(in crate::canonical) enum Case {
    Json,
    Text,
    Binary,
    Unknown,
    CancelSend,
    Overflow,
    Malformed,
    Lifecycle,
}
fn options() -> RequestOptions {
    super::options()
}
fn mutation(id: &str) -> MutationOptions {
    MutationOptions {
        operation_id: id.parse().unwrap(),
        request: options(),
        extensions: None,
    }
}
fn to() -> NonEmpty<sdk::ParticipantId> {
    NonEmpty {
        first: "part_target".parse().unwrap(),
        rest: vec![],
    }
}
fn payload(binary: bool) -> DataPayload {
    if binary {
        DataPayload::Binary(vec![0, 255, 128, 1])
    } else {
        DataPayload::Json(json!({"sequence":7,"private":"data-content"}))
    }
}
fn saved() -> Value {
    json!({"schema":"connect-operation-reference/1","operation_id":"data-original","command":"message.send","scope":{"conversation_id":"conv_one","session_id":"session_one","connection_id":"connection_one"}})
}
async fn incoming(
    writer: &mut quinn::SendStream,
    signer: &Sig9421Signer,
    index: usize,
    case: Case,
) {
    let mut frame = envelope(
        "message.send",
        json!({"msg_id":format!("incoming-{index}"),"from":"part_source","to":["part_one"],"label":"data","content_type":"application/json","body":format!("{{\"index\":{index}}}"),"body_encoding":"utf8","reliability":{"mode":"reliable_ordered"}}),
    );
    frame["cid"] = json!("conv_one");
    frame["sid"] = json!("session_one");
    frame["connid"] = json!("connection_one");
    frame["operation_id"] = json!(format!("incoming-operation-{index}"));
    if matches!(case, Case::Text) {
        frame["payload"]["content_type"] = json!("text/plain");
        frame["payload"]["body"] = json!("hello 🦀");
    }
    if matches!(case, Case::Binary) {
        frame["payload"]["content_type"] = json!("application/octet-stream");
        frame["payload"]["body_encoding"] = json!("base64");
        frame["payload"]["body"] = json!("AP+AAQ==");
    }
    if matches!(case, Case::Malformed) {
        match index {
            0 => frame["sid"] = json!("foreign_session"),
            1 => frame["payload"]["label"] = json!("unknown_data"),
            2 => frame["payload"]["body"] = json!("not-json"),
            3 => frame["payload"]["to"] = json!(["part_someone_else"]),
            4 => frame["payload"]["in_reply_to_msg"] = json!("threaded-message"),
            5 => frame["payload"]["content_type"] = json!("application/octet-stream"),
            _ => (),
        }
    }
    Envelope::from_value(frame.clone()).expect("fixture must remain a valid wire envelope");
    let bytes = serde_json::to_vec(&signer.sign(&frame).unwrap()).unwrap();
    writer
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .await
        .unwrap();
    writer.write_all(&bytes).await.unwrap();
}
async fn response(
    writer: &mut quinn::SendStream,
    signer: &Sig9421Signer,
    r: &Envelope,
    state: &str,
    msg: &str,
) {
    let operation = r
        .scope(Scope::Operation)
        .or_else(|| r.original()["payload"]["operation_id"].as_str())
        .unwrap();
    let mut p = json!({"operation_id":operation,"command_type":"message.send","record_id":"opr_data","state":state,"revision":1});
    if state == "completed" {
        p["result"] =
            json!({"connection_id":"connection_one","message_id":msg,"core_queue_admission":true});
    }
    write(writer, signer, r, "operation.updated", p, true).await;
    write(writer, signer, r, "ack", json!({}), true).await;
}
pub(super) async fn emit(
    case: Case,
    reader: &mut quinn::RecvStream,
    writer: &mut quinn::SendStream,
    signer: &Sig9421Signer,
    signal: &Notify,
) {
    let count = match case {
        Case::Overflow => 40,
        Case::Malformed => 7,
        Case::Lifecycle => 0,
        _ => 1,
    };
    for index in 0..count {
        incoming(writer, signer, index, case).await;
    }
    let mut sends = 0;
    let mut message = String::new();
    while let Some(request) = selections::maybe_read(reader).await {
        match request.kind() {
            "message.send" => {
                sends += 1;
                assert_eq!(sends, 1, "recovery must not submit another message");
                assert_eq!(request.scope(Scope::Operation), Some("data-original"));
                assert_eq!(request.scope(Scope::Connection), Some("connection_one"));
                assert!(request.expected_revision().is_none());
                let p = &request.original()["payload"];
                assert_eq!(p["label"], "data");
                assert_eq!(p["to"], json!(["part_target"]));
                assert_eq!(p["reliability"], json!({"mode":"reliable_ordered"}));
                if matches!(case, Case::Binary) {
                    assert_eq!(p["body"], "AP+AAQ==");
                    assert_eq!(p["body_encoding"], "base64");
                } else if matches!(case, Case::Text) {
                    assert_eq!(p["body"], "hello 🦀");
                } else {
                    assert_eq!(
                        serde_json::from_str::<Value>(p["body"].as_str().unwrap()).unwrap(),
                        json!({"sequence":7,"private":"data-content"})
                    );
                }
                message = p["msg_id"].as_str().unwrap().into();
                if matches!(case, Case::CancelSend) {
                    signal.notify_one();
                    signal.notified().await;
                }
                response(
                    writer,
                    signer,
                    &request,
                    if matches!(case, Case::Unknown) {
                        "unknown"
                    } else {
                        "completed"
                    },
                    &message,
                )
                .await;
            }
            "operation.get" => {
                let id = request.original()["payload"]["operation_id"]
                    .as_str()
                    .unwrap();
                if id == "native-original" {
                    status(writer, signer, &request, "completed").await;
                } else if id == "data-end" {
                    write(writer,signer,&request,"operation.updated",json!({"operation_id":id,"state":"completed","revision":1,"result":{"connection_id":"connection_one","ended":true}}),true).await;
                    write(writer, signer, &request, "ack", json!({}), true).await;
                } else {
                    assert_eq!(id, "data-original");
                    assert_eq!(sends, 1);
                    response(writer, signer, &request, "completed", &message).await;
                }
            }
            "connection.end" => {
                assert_eq!(request.scope(Scope::Operation), Some("data-end"));
                write(writer,signer,&request,"operation.updated",json!({"operation_id":"data-end","state":"completed","revision":1,"result":{"connection_id":"connection_one","ended":true}}),true).await;
                write(writer, signer, &request, "ack", json!({}), true).await;
            }
            _ => panic!("Unexpected data fixture command {}", request.kind()),
        }
    }
}
async fn ready(
    case: Case,
) -> (
    Client,
    sdk::Connection,
    sdk::OperationReference<sdk::Connection>,
    tokio::task::JoinHandle<()>,
    Arc<Notify>,
) {
    let (peer, host, signal) = controlled(Fault::SdkNative(super::Case::Data(case))).await;
    let client = Client::from_authenticated_peer(peer, "tenant".parse().unwrap()).unwrap();
    let session = client
        .conversation_by_id("conv_one".parse().unwrap())
        .session_by_id("session_one".parse().unwrap());
    let mut streams = offered();
    if matches!(case, Case::Binary | Case::Text) {
        streams.rest[1].codecs.first = if matches!(case, Case::Text) {
            "text"
        } else {
            "binary"
        }
        .into();
    }
    let operation = session
        .offer_connection(
            "connection_one".parse().unwrap(),
            streams,
            super::mutation(),
        )
        .await
        .unwrap();
    let reference = operation.reference();
    let connection = operation.wait(options()).await.unwrap().value;
    (client, connection, reference, host, signal)
}
async fn received(client: &Client, n: u64) {
    let until = tokio::time::Instant::now() + Duration::from_secs(3);
    while client.data_stats().unwrap().received_frames < n {
        assert!(tokio::time::Instant::now() < until);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}
#[tokio::test]
async fn signed_native_json_and_binary_data_preserve_bytes_original_ids_and_queue_admission_only() {
    for case in [Case::Json, Case::Text, Case::Binary, Case::Unknown] {
        let (client, c, _, host, _) = ready(case).await;
        let stream = c.stream_by_id("data".parse().unwrap());
        let incoming = stream.receive_data(options()).await.unwrap();
        assert_eq!(incoming.from.as_str(), "part_source");
        assert_eq!(incoming.operation_id.as_str(), "incoming-operation-0");
        assert_eq!(
            incoming.payload,
            if matches!(case, Case::Text) {
                DataPayload::Text("hello 🦀".into())
            } else if matches!(case, Case::Binary) {
                DataPayload::Binary(vec![0, 255, 128, 1])
            } else {
                DataPayload::Json(json!({"index":0}))
            }
        );
        let op = stream
            .send_data(
                "message-original".parse().unwrap(),
                to(),
                if matches!(case, Case::Text) {
                    DataPayload::Text("hello 🦀".into())
                } else {
                    payload(matches!(case, Case::Binary))
                },
                mutation("data-original"),
            )
            .await
            .unwrap();
        let saved = op.reference_json();
        assert!(client
            .restore(saved.clone(), client.operation_kinds().message_send)
            .is_err());
        let op = client
            .restore(saved, client.operation_kinds().data_send)
            .unwrap();
        let completed = op.wait(options()).await.unwrap().value;
        assert_eq!(completed.message_id.as_str(), "message-original");
        assert_eq!(
            completed.boundary,
            DataAcceptanceBoundary::CoreQueueAdmission
        );
        assert!(!format!("{incoming:?}").contains("index"));
        close(client, host).await;
    }
}
#[tokio::test]
async fn native_data_wait_cancellation_deadline_and_clones_do_not_consume_messages_or_block_control(
) {
    let (client, c, reference, host, _) = ready(Case::Json).await;
    received(&client, 1).await;
    let stream = c.stream_by_id("data".parse().unwrap());
    let cancelled = options();
    cancelled.signal.cancel();
    assert_eq!(
        stream.receive_data(cancelled).await.unwrap_err().kind,
        ErrorKind::Cancelled
    );
    assert_eq!(
        stream
            .clone()
            .receive_data(options())
            .await
            .unwrap()
            .message_id
            .as_str(),
        "incoming-0"
    );
    let wait = stream.receive_data(RequestOptions::new(Duration::from_millis(30)).unwrap());
    let original = client.recover(reference);
    let control = original.status(options());
    let (wait, control) = tokio::join!(wait, control);
    assert_eq!(wait.unwrap_err().kind, ErrorKind::Deadline);
    control.unwrap();
    close(client, host).await;
}
#[tokio::test]
async fn native_data_overflow_is_explicit_sticky_bounded_and_leaves_other_control_usable() {
    let (client, c, reference, host, _) = ready(Case::Overflow).await;
    received(&client, 40).await;
    let stream = c.stream_by_id("data".parse().unwrap());
    for _ in 0..2 {
        assert_eq!(
            stream.receive_data(options()).await.unwrap_err().kind,
            ErrorKind::ResyncRequired
        );
    }
    let stats = client.data_stats().unwrap();
    assert_eq!(stats.overflow_drops, 40);
    assert_eq!(stats.queued_messages, 0);
    assert_eq!(stats.queued_bytes, 0);
    client.recover(reference).status(options()).await.unwrap();
    close(client, host).await;
}
#[tokio::test]
async fn signed_foreign_malformed_or_unowned_native_data_cannot_enter_an_owned_stream_queue() {
    let (client, c, reference, host, _) = ready(Case::Malformed).await;
    received(&client, 7).await;
    let stream = c.stream_by_id("data".parse().unwrap());
    assert_eq!(
        stream
            .receive_data(options())
            .await
            .unwrap()
            .message_id
            .as_str(),
        "incoming-6"
    );
    let stats = client.data_stats().unwrap();
    assert_eq!(stats.unowned_frames, 2);
    assert_eq!(stats.malformed_frames, 4);
    assert_eq!(stats.accepted_messages, 1);
    client.recover(reference).status(options()).await.unwrap();
    close(client, host).await;
}
#[tokio::test]
async fn native_data_send_cancellation_preserves_original_uncertainty_and_recovers_without_resubmission(
) {
    let (client, c, _, host, signal) = ready(Case::CancelSend).await;
    let stream = c.stream_by_id("data".parse().unwrap());
    let opts = mutation("data-original");
    let cancel = opts.request.signal.clone();
    let send = stream.send_data(
        "message-original".parse().unwrap(),
        to(),
        payload(false),
        opts,
    );
    tokio::pin!(send);
    tokio::select! {result=&mut send=>panic!("premature send: {result:?}"),_=signal.notified()=>()}
    cancel.cancel();
    let error = send.await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::Cancelled);
    assert_eq!(error.certainty, Certainty::Uncertain);
    assert_eq!(
        error.operation.unwrap().operation_id.as_str(),
        "data-original"
    );
    signal.notify_one();
    let operation = client
        .restore(saved(), client.operation_kinds().data_send)
        .unwrap();
    assert_eq!(
        operation
            .wait(options())
            .await
            .unwrap()
            .value
            .message_id
            .as_str(),
        "message-original"
    );
    close(client, host).await;
}
#[tokio::test]
async fn native_data_preflight_and_lifecycle_refuse_without_fabricating_custody_or_blocking_cleanup(
) {
    let (client, c, reference, host, _) = ready(Case::Lifecycle).await;
    let stream = c.stream_by_id("data".parse().unwrap());
    let invalid = stream
        .send_data(
            "large".parse().unwrap(),
            to(),
            DataPayload::Binary(vec![0; 64 * 1024 + 1]),
            mutation("too-large"),
        )
        .await
        .unwrap_err();
    assert_eq!(invalid.kind, ErrorKind::Validation);
    assert_eq!(invalid.certainty, Certainty::NotSubmitted);
    assert_eq!(
        stream
            .send_data(
                "wrong-codec".parse().unwrap(),
                to(),
                payload(true),
                mutation("wrong-codec")
            )
            .await
            .unwrap_err()
            .certainty,
        Certainty::NotSubmitted
    );
    let cancelled = mutation("cancelled");
    cancelled.request.signal.cancel();
    assert_eq!(
        stream
            .send_data(
                "cancelled".parse().unwrap(),
                to(),
                payload(false),
                cancelled
            )
            .await
            .unwrap_err()
            .certainty,
        Certainty::NotSubmitted
    );
    assert_eq!(
        c.stream_by_id("audio".parse().unwrap())
            .receive_data(options())
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Unsupported
    );
    let waiting = stream.receive_data(options());
    let ending = async {
        c.end(200, "normal-clearing".into(), None, mutation("data-end"))
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap();
    };
    let (waiting, _) = tokio::join!(waiting, ending);
    assert_eq!(waiting.unwrap_err().kind, ErrorKind::Unsupported);
    assert_eq!(client.data_stats().unwrap().queued_bytes, 0);
    client.recover(reference).status(options()).await.unwrap();
    close(client, host).await;
}
