//! Signed stream rehearsals: responses deliberately depend on later requests.
use super::sdk_worker::write;
use super::*;
use crate::sdk::{self, Client, MutationOptions, NonEmpty, RequestOptions};

#[derive(Clone, Copy)]
pub(in crate::canonical) enum Case {
    Interleaved,
    Capacity,
    Close,
    ForeignScope,
    Deadline,
}

async fn read(reader: &mut quinn::RecvStream) -> Envelope {
    let mut prefix = [0; 4];
    reader.read_exact(&mut prefix).await.unwrap();
    let size = u32::from_be_bytes(prefix) as usize;
    assert!(size > 0 && size <= MAX_ENVELOPE_BYTES);
    let mut bytes = vec![0; size];
    reader.read_exact(&mut bytes).await.unwrap();
    Envelope::parse(&bytes).unwrap()
}
async fn observed(
    writer: &mut quinn::SendStream,
    signer: &Sig9421Signer,
    request: &Envelope,
    revision: u64,
) {
    write(
        writer,
        signer,
        request,
        "operation.updated",
        json!({"operation_id":"original-send","state":"unknown","revision":revision}),
    )
    .await;
    write(writer, signer, request, "ack", json!({})).await;
}
pub(super) async fn emit(
    case: Case,
    first: Envelope,
    writer: &mut quinn::SendStream,
    reader: &mut quinn::RecvStream,
    signer: &Sig9421Signer,
    ready: &Notify,
) {
    if matches!(case, Case::Capacity) {
        let mut held = vec![first];
        for _ in 1..16 {
            held.push(read(reader).await);
        }
        assert!(held.iter().all(|r| r.kind() == "operation.get"));
        ready.notify_one();
        // The seventeenth caller must expire outside the shared owned capacity.
        assert!(
            tokio::time::timeout(Duration::from_millis(180), read(reader))
                .await
                .is_err()
        );
        for request in held.iter().rev() {
            observed(writer, signer, request, 1).await;
        }
        let final_query = read(reader).await;
        assert_eq!(final_query.kind(), "operation.get"); // No expired mutation was written later.
        observed(writer, signer, &final_query, 2).await;
        return;
    }
    assert_eq!(first.kind(), "message.send");
    ready.notify_one();
    let second = read(reader).await; // Must arrive while first still has no receipt.
    assert_eq!(second.kind(), "operation.get");
    match case {
        Case::Close | Case::Deadline => {
            ready.notify_one();
            return;
        }
        Case::ForeignScope => {
            let mut foreign = first.original().clone();
            foreign["cid"] = json!("conv_foreign");
            // write() fixes cid, so sign the malicious correlated frame directly.
            foreign["type"] = json!("ack");
            foreign["payload"] = json!({});
            foreign["id"] = json!("foreign-reply");
            foreign["ts"] = json!(chrono::Utc::now());
            foreign["in_reply_to"] = json!(first.id());
            foreign.as_object_mut().unwrap().remove("sig");
            let bytes = serde_json::to_vec(&signer.sign(&foreign).unwrap()).unwrap();
            Envelope::parse(&bytes).unwrap();
            writer
                .write_all(&(bytes.len() as u32).to_be_bytes())
                .await
                .unwrap();
            writer.write_all(&bytes).await.unwrap();
            return;
        }
        Case::Interleaved => {}
        Case::Capacity => unreachable!(),
    }
    // Interleave a partial first batch with a complete second batch.
    let result = json!({"accepted":true});
    write(
        writer,
        signer,
        &first,
        "operation.updated",
        json!({"operation_id":"original-send","state":"unknown","revision":1}),
    )
    .await;
    observed(writer, signer, &second, 7).await;
    let decision = read(reader).await;
    assert_eq!(decision.kind(), "operation.cancel");
    assert_eq!(
        decision.original()["payload"]["operation_id"],
        "original-send"
    );
    write(writer,signer,&decision,"operation.updated",json!({"operation_id":"cancel-one","state":"completed","revision":1,"result":{"target_operation_id":"original-send","state_at_decision":"unknown","disposition":"cancellation_requested","evidence_id":null}})).await;
    write(writer, signer, &decision, "ack", json!({})).await;
    let third = read(reader).await;
    assert_eq!(third.kind(), "operation.get");
    observed(writer, signer, &third, 8).await;
    // The cancelled caller's reply still belongs to its original exchange.
    write(writer, signer, &first, "ack", json!({"details":result})).await;
    let fourth = read(reader).await;
    assert_eq!(fourth.kind(), "operation.get");
    observed(writer, signer, &fourth, 9).await;
}
fn options() -> RequestOptions {
    RequestOptions::new(Duration::from_secs(3)).unwrap()
}
fn original(client: &Client) -> sdk::Operation<sdk::MessageAcceptance> {
    client.restore(json!({"schema":"connect-operation-reference/1","operation_id":"original-send","command":"message.send","scope":{"conversation_id":"conv_one"}}),client.operation_kinds().message_send).unwrap()
}
async fn send(
    client: Client,
    signal: sdk::CancellationToken,
    timeout: Duration,
) -> sdk::Result<sdk::Operation<sdk::MessageAcceptance>> {
    client
        .conversation_by_id("conv_one".parse().unwrap())
        .send(
            "original-message".parse().unwrap(),
            NonEmpty {
                first: "part_one".parse().unwrap(),
                rest: vec![],
            },
            "private fixture".into(),
            MutationOptions {
                operation_id: "original-send".parse().unwrap(),
                extensions: None,
                request: RequestOptions { signal, timeout },
            },
        )
        .await
}
async fn joined(host: tokio::task::JoinHandle<()>) {
    tokio::time::timeout(Duration::from_secs(3), host)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn status_and_durable_cancel_complete_before_cancelled_submit_receipt_arrives() {
    let (peer, host, ready) = controlled(Fault::SdkMultiplex(Case::Interleaved)).await;
    let client = Client::from_authenticated_peer(peer, "tenant".parse().unwrap()).unwrap();
    let signal = sdk::CancellationToken::new();
    let cancel = signal.clone();
    let submitting = tokio::spawn(send(client.clone(), signal, Duration::from_secs(3)));
    ready.notified().await;
    cancel.cancel();
    let error = submitting.await.unwrap().unwrap_err();
    assert_eq!(error.kind, sdk::ErrorKind::Cancelled);
    assert_eq!(error.certainty, sdk::Certainty::Uncertain);
    assert_eq!(
        error.operation.unwrap().operation_id.as_str(),
        "original-send"
    );
    let target = original(&client);
    assert_eq!(target.status(options()).await.unwrap().revision, 7);
    let decision = target
        .cancel(MutationOptions {
            operation_id: "cancel-one".parse().unwrap(),
            request: options(),
            extensions: None,
        })
        .await
        .unwrap();
    assert_eq!(
        decision.reference().key().operation_id.as_str(),
        "cancel-one"
    );
    assert_eq!(target.status(options()).await.unwrap().revision, 8);
    assert_eq!(target.status(options()).await.unwrap().revision, 9);
    client.close(options()).await.unwrap();
    joined(host).await;
}
#[tokio::test]
async fn shared_capacity_bounds_active_requests_and_expired_waiter_never_writes() {
    let (peer, host, ready) = controlled(Fault::SdkMultiplex(Case::Capacity)).await;
    let client = Client::from_authenticated_peer(peer, "tenant".parse().unwrap()).unwrap();
    let mut queries = Vec::new();
    for _ in 0..16 {
        let target = original(&client);
        queries.push(tokio::spawn(async move { target.status(options()).await }));
    }
    ready.notified().await;
    let error = send(
        client.clone(),
        sdk::CancellationToken::new(),
        Duration::from_millis(50),
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind, sdk::ErrorKind::Deadline);
    assert_eq!(error.certainty, sdk::Certainty::NotSubmitted);
    for query in queries {
        assert_eq!(query.await.unwrap().unwrap().revision, 1);
    }
    assert_eq!(
        original(&client).status(options()).await.unwrap().revision,
        2
    );
    client.close(options()).await.unwrap();
    joined(host).await;
}
async fn retires(case: Case, kind: sdk::ErrorKind) {
    let (mut peer, host, ready) = controlled(Fault::SdkMultiplex(case)).await;
    if matches!(case, Case::Deadline) {
        peer.deadline = Duration::from_millis(300);
    }
    let client = Client::from_authenticated_peer(peer, "tenant".parse().unwrap()).unwrap();
    let submitting = tokio::spawn(send(
        client.clone(),
        sdk::CancellationToken::new(),
        Duration::from_secs(3),
    ));
    ready.notified().await;
    let target = original(&client);
    let queried = tokio::spawn(async move { target.status(options()).await });
    if matches!(case, Case::Close) {
        ready.notified().await;
        client.close(options()).await.unwrap();
    }
    let error = submitting.await.unwrap().unwrap_err();
    assert_eq!(error.kind, kind);
    assert_eq!(error.certainty, sdk::Certainty::Uncertain);
    assert_eq!(
        error.operation.unwrap().operation_id.as_str(),
        "original-send"
    );
    let error = queried.await.unwrap().unwrap_err();
    assert_eq!(error.kind, kind);
    assert_eq!(error.certainty, sdk::Certainty::ReadOnly);
    client.close(options()).await.unwrap();
    client.close(options()).await.unwrap();
    joined(host).await;
}
#[tokio::test]
async fn close_joins_all_active_exchanges_without_losing_mutation_identity() {
    retires(Case::Close, sdk::ErrorKind::Cancelled).await;
}
#[tokio::test]
async fn foreign_scope_retires_all_active_exchanges() {
    retires(Case::ForeignScope, sdk::ErrorKind::Protocol).await;
}
#[tokio::test]
async fn incomplete_exchange_deadline_retires_all_active_exchanges() {
    retires(Case::Deadline, sdk::ErrorKind::Deadline).await;
}
