//! Real signed frames and delayed exchanges test SDK ownership, not a fake SDK.
use super::*;
use crate::sdk::{self, Client, MutationOptions, NonEmpty, RequestOptions};
#[derive(Clone, Copy)]
pub(in crate::canonical) enum Traffic {
    Query,
    Mutation,
}
pub(super) async fn write(
    writer: &mut quinn::SendStream,
    signer: &Sig9421Signer,
    request: &Envelope,
    kind: &str,
    payload: Value,
) {
    let mut frame = envelope(kind, payload);
    frame["cid"] = json!("conv_one");
    frame["in_reply_to"] = json!(request.id());
    if let Some(operation) = request.scope(Scope::Operation) {
        frame["operation_id"] = json!(operation);
    }
    let signed = signer.sign(&frame).unwrap();
    Envelope::from_value(signed.clone()).unwrap();
    let raw = serde_json::to_vec(&signed).unwrap();
    writer
        .write_all(&(raw.len() as u32).to_be_bytes())
        .await
        .unwrap();
    writer.write_all(&raw).await.unwrap();
}
pub(super) async fn emit(
    traffic: Traffic,
    first: Envelope,
    writer: &mut quinn::SendStream,
    reader: &mut quinn::RecvStream,
    signer: &Sig9421Signer,
    ready: &Notify,
) {
    assert_eq!(
        first.kind(),
        if matches!(traffic, Traffic::Query) {
            "operation.get"
        } else {
            "message.send"
        }
    );
    let receipt = connect_protocol::transitions::MutationReceipt::committed(
        "original-send".into(),
        "conv_one".into(),
    );
    let result = serde_json::to_value(connect_protocol::Outcome::Accepted {
        receipt: Some(receipt),
        request_id: "original-send".into(),
        message_id: "original-message".into(),
        conversation_id: "conv_one".into(),
    })
    .unwrap();
    ready.notify_one(); // First request is actually on the wire before cancellation.
    tokio::time::sleep(Duration::from_millis(120)).await;
    if matches!(traffic, Traffic::Query) {
        write(
            writer,
            signer,
            &first,
            "operation.updated",
            json!({"operation_id":"original-send","state":"unknown","revision":1}),
        )
        .await;
        write(writer, signer, &first, "ack", json!({})).await;
    } else {
        assert_eq!(first.scope(Scope::Operation), Some("original-send"));
        assert_eq!(first.original()["payload"]["msg_id"], "original-message");
        write(writer, signer, &first, "ack", json!({"details":result})).await;
    }
    let mut length = [0; 4];
    reader.read_exact(&mut length).await.unwrap();
    let mut raw = vec![0; u32::from_be_bytes(length) as usize];
    assert!(raw.len() <= MAX_ENVELOPE_BYTES);
    reader.read_exact(&mut raw).await.unwrap();
    let second = Envelope::parse(&raw).unwrap();
    // No queued cancelled send, duplicate send, or hidden re-submission may arrive.
    assert_eq!(second.kind(), "operation.get");
    assert!(second.scope(Scope::Operation).is_none());
    assert_eq!(
        second.original()["payload"]["operation_id"],
        "original-send"
    );
    write(
        writer,
        signer,
        &second,
        "operation.updated",
        json!({"operation_id":"original-send","state":"completed","revision":2,"result":result}),
    )
    .await;
    write(writer, signer, &second, "ack", json!({})).await;
}
fn options() -> RequestOptions {
    RequestOptions::new(Duration::from_secs(3)).unwrap()
}
fn reference() -> Value {
    json!({"schema":"connect-operation-reference/1","operation_id":"original-send","command":"message.send","scope":{"conversation_id":"conv_one"}})
}
#[tokio::test]
async fn cancelling_a_started_mutation_keeps_original_reference_and_finishes_receipt_exchange() {
    let (peer, host, ready) = controlled(Fault::SdkWorker(Traffic::Mutation)).await;
    let client = Client::from_authenticated_peer(peer, "tenant".parse().unwrap()).unwrap();
    let room = client.conversation_by_id("conv_one".parse().unwrap());
    let signal = sdk::CancellationToken::new();
    let cancel = signal.clone();
    let waiting = tokio::spawn(async move {
        room.send(
            "original-message".parse().unwrap(),
            NonEmpty {
                first: "part_one".parse().unwrap(),
                rest: vec![],
            },
            "private fixture".into(),
            MutationOptions {
                operation_id: "original-send".parse().unwrap(),
                request: RequestOptions {
                    signal,
                    ..options()
                },
                extensions: None,
            },
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(2), ready.notified())
        .await
        .unwrap();
    cancel.cancel();
    let error = waiting.await.unwrap().unwrap_err();
    assert_eq!(error.kind, sdk::ErrorKind::Cancelled);
    assert_eq!(error.certainty, sdk::Certainty::Uncertain);
    assert_eq!(
        error.operation.unwrap().operation_id.as_str(),
        "original-send"
    );
    let original = client
        .restore(reference(), client.operation_kinds().message_send)
        .unwrap();
    let result = original.wait(options()).await.unwrap();
    assert_eq!(result.value.message_id.as_str(), "original-message");
    assert_eq!(result.snapshot.revision, 2);
    client.close(options()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), host)
        .await
        .unwrap()
        .unwrap();
}
#[tokio::test]
async fn local_query_cancellation_and_presubmit_mutation_cancellation_leave_peer_usable() {
    let (peer, host, ready) = controlled(Fault::SdkWorker(Traffic::Query)).await;
    let client = Client::from_authenticated_peer(peer, "tenant".parse().unwrap()).unwrap();
    let original = client
        .restore(reference(), client.operation_kinds().message_send)
        .unwrap();
    let signal = sdk::CancellationToken::new();
    let cancel = signal.clone();
    let queried = original.clone();
    let first = tokio::spawn(async move {
        queried
            .status(RequestOptions {
                signal,
                ..options()
            })
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), ready.notified())
        .await
        .unwrap();
    let queued_signal = sdk::CancellationToken::new();
    queued_signal.cancel(); // Deterministically cancelled before submission, independent of scheduling.
    let room = client.conversation_by_id("conv_one".parse().unwrap());
    let queued = tokio::spawn(async move {
        room.send(
            "never-written-message".parse().unwrap(),
            NonEmpty {
                first: "part_one".parse().unwrap(),
                rest: vec![],
            },
            "never written".into(),
            MutationOptions {
                operation_id: "never-written".parse().unwrap(),
                request: RequestOptions {
                    signal: queued_signal,
                    ..options()
                },
                extensions: None,
            },
        )
        .await
    });
    cancel.cancel();
    let error = first.await.unwrap().unwrap_err();
    assert_eq!(error.kind, sdk::ErrorKind::Cancelled);
    assert_eq!(error.certainty, sdk::Certainty::ReadOnly);
    assert!(matches!(queued.await.unwrap(),Err(e) if e.kind==sdk::ErrorKind::Cancelled));
    let current = original.status(options()).await.unwrap();
    assert_eq!(current.revision, 2);
    assert_eq!(current.state, sdk::OperationState::Completed);
    let closing = RequestOptions {
        signal: sdk::CancellationToken::new(),
        ..options()
    };
    closing.signal.cancel();
    assert!(
        matches!(client.close(closing).await,Err(e) if e.kind==sdk::ErrorKind::CleanupIncomplete)
    );
    client.close(options()).await.unwrap();
    client.close(options()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), host)
        .await
        .unwrap()
        .unwrap();
}
