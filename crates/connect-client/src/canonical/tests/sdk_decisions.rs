//! Signed controlled receipts check decoding/uncertainty, not backend reconciliation.
use super::*;
use crate::sdk::{self, Client, MutationOptions, RequestOptions};
#[derive(Clone, Copy)]
pub(in crate::canonical) enum Case {
    Cancel,
    Reconcile,
    WrongTarget,
    Unavailable,
    StatusRecover,
    StatusReady,
    StatusForbidden,
    StatusDeadline,
    StatusCancel,
}
async fn write(
    writer: &mut quinn::SendStream,
    signer: &Sig9421Signer,
    request: &Envelope,
    kind: &str,
    payload: Value,
) {
    let mut frame = envelope(kind, payload);
    frame["cid"] = json!("conv_one");
    frame["in_reply_to"] = json!(request.id());
    if let Some(id) = request.scope(Scope::Operation) {
        frame["operation_id"] = json!(id);
    }
    let frame = signer.sign(&frame).unwrap();
    Envelope::from_value(frame.clone()).unwrap();
    let bytes = serde_json::to_vec(&frame).unwrap();
    writer
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .await
        .unwrap();
    writer.write_all(&bytes).await.unwrap();
}
pub(super) async fn emit(
    case: Case,
    first: Envelope,
    writer: &mut quinn::SendStream,
    reader: &mut quinn::RecvStream,
    signer: &Sig9421Signer,
) {
    if matches!(case, Case::StatusReady) {
        status_reads(case, first, writer, reader, signer).await;
        return;
    }
    assert_eq!(
        first.kind(),
        if matches!(case, Case::Reconcile) {
            "operation.reconcile"
        } else {
            "operation.cancel"
        }
    );
    assert_eq!(first.scope(Scope::Operation), Some("decision-one"));
    assert_eq!(first.original()["payload"]["operation_id"], "original-send");
    if matches!(
        case,
        Case::StatusRecover
            | Case::StatusReady
            | Case::StatusForbidden
            | Case::StatusDeadline
            | Case::StatusCancel
    ) {
        status_reads(case, first, writer, reader, signer).await;
        return;
    }
    if matches!(case, Case::Unavailable) {
        write(
            writer,
            signer,
            &first,
            "error",
            json!({"code":503,"category":"server","reason":"Decision receipt unavailable"}),
        )
        .await;
        return;
    }
    let result = json!({"target_operation_id":if matches!(case,Case::WrongTarget){"foreign-target"}else{"original-send"},"state_at_decision":if matches!(case,Case::Reconcile){"completed"}else{"unknown"},"disposition":if matches!(case,Case::Reconcile){"reconciled"}else{"cancellation_requested"},"evidence_id":if matches!(case,Case::Reconcile){json!("valid-evidence")}else{Value::Null}});
    if matches!(case, Case::Reconcile) {
        assert_eq!(first.original()["payload"]["outcome"], "completed");
        assert_eq!(first.original()["payload"]["evidence_id"], "valid-evidence");
    }
    let update =
        json!({"operation_id":"decision-one","state":"completed","revision":1,"result":result});
    write(writer, signer, &first, "operation.updated", update.clone()).await;
    write(writer, signer, &first, "ack", json!({})).await;
    if matches!(case, Case::WrongTarget) {
        return;
    }
    let mut length = [0; 4];
    reader.read_exact(&mut length).await.unwrap();
    let mut bytes = vec![0; u32::from_be_bytes(length) as usize];
    assert!(bytes.len() <= MAX_ENVELOPE_BYTES);
    reader.read_exact(&mut bytes).await.unwrap();
    let query = Envelope::parse(&bytes).unwrap();
    assert_eq!(query.kind(), "operation.get");
    assert!(query.scope(Scope::Operation).is_none());
    assert_eq!(query.original()["payload"]["operation_id"], "decision-one");
    write(writer, signer, &query, "operation.updated", update).await;
    write(writer, signer, &query, "ack", json!({})).await;
}

async fn status_reads(
    case: Case,
    first: Envelope,
    writer: &mut quinn::SendStream,
    reader: &mut quinn::RecvStream,
    signer: &Sig9421Signer,
) {
    let ready = matches!(case, Case::StatusReady);
    let operation = if ready {
        "original-send"
    } else {
        "decision-one"
    };
    let mut pending = if ready {
        Some(first)
    } else {
        // A durable cancellation decision has a synchronous, typed receipt.
        // The later operation.get calls recover that original decision.
        write(
            writer,
            signer,
            &first,
            "operation.updated",
            json!({
            "operation_id":"decision-one","state":"completed","revision":1,
            "result":{"target_operation_id":"original-send","state_at_decision":"unknown",
                "disposition":"cancellation_requested","evidence_id":null}}),
        )
        .await;
        write(writer, signer, &first, "ack", json!({})).await;
        None
    };
    let mut reads = 0;
    loop {
        let query = if let Some(query) = pending.take() {
            query
        } else {
            let mut length = [0; 4];
            if reader.read_exact(&mut length).await.is_err() {
                break;
            }
            let mut bytes = vec![0; u32::from_be_bytes(length) as usize];
            assert!(bytes.len() <= MAX_ENVELOPE_BYTES);
            reader.read_exact(&mut bytes).await.unwrap();
            Envelope::parse(&bytes).unwrap()
        };
        // No second cancellation, replacement or fresh original is allowed.
        assert_eq!(query.kind(), "operation.get");
        assert!(query.scope(Scope::Operation).is_none());
        assert_eq!(query.scope(Scope::Conversation), Some("conv_one"));
        assert_eq!(query.original()["payload"]["operation_id"], operation);
        reads += 1;
        assert!(
            reads <= 16,
            "status reads exceeded the original bounded wait"
        );
        if reads > 1 && matches!(case, Case::StatusRecover | Case::StatusReady) {
            let result = json!({"target_operation_id":"original-send","state_at_decision":"unknown",
                "disposition":"cancellation_requested","evidence_id":null});
            let mut update = json!({"operation_id":operation,
                "state":if ready {"ready"} else {"completed"},"revision":1});
            if !ready {
                update["result"] = result;
            }
            write(writer, signer, &query, "operation.updated", update).await;
            write(writer, signer, &query, "ack", json!({})).await;
            return;
        }
        let code = if reads > 1 && matches!(case, Case::StatusForbidden) {
            403
        } else {
            503
        };
        write(
            writer,
            signer,
            &query,
            "error",
            json!({"code":code,"category":"server","reason":"Controlled original status refusal"}),
        )
        .await;
        if code == 403 {
            return;
        }
    }
    assert!(matches!(case, Case::StatusDeadline | Case::StatusCancel));
    assert!(reads > 0);
}
fn options() -> RequestOptions {
    RequestOptions::new(Duration::from_secs(3)).unwrap()
}
fn mutation() -> MutationOptions {
    MutationOptions {
        operation_id: "decision-one".parse().unwrap(),
        request: options(),
        extensions: None,
    }
}
async fn client(case: Case) -> (Client, tokio::task::JoinHandle<()>) {
    let (peer, host, _) = controlled(Fault::SdkDecision(case)).await;
    (
        Client::from_authenticated_peer(peer, "tenant".parse().unwrap()).unwrap(),
        host,
    )
}
fn target(client: &Client) -> sdk::Operation<sdk::MessageAcceptance> {
    client.restore(json!({"schema":"connect-operation-reference/1","operation_id":"original-send","command":"message.send","scope":{"conversation_id":"conv_one"}}),client.operation_kinds().message_send).unwrap()
}
#[tokio::test]
async fn cancellation_receipt_can_complete_while_original_target_stays_unknown() {
    let (client, host) = client(Case::Cancel).await;
    let decision = target(&client).cancel(mutation()).await.unwrap();
    let result = decision.wait(options()).await.unwrap();
    assert_eq!(result.snapshot.state, sdk::OperationState::Completed);
    assert_eq!(result.value.target.as_str(), "original-send");
    assert_eq!(result.value.observed_state, sdk::OperationState::Unknown);
    assert_eq!(result.value.disposition, "cancellation_requested");
    assert_eq!(result.value.decision.operation_id.as_str(), "decision-one");
    client.close(options()).await.unwrap();
    host.await.unwrap();
}
#[tokio::test]
async fn attributed_reconciliation_receipt_preserves_target_outcome_and_evidence() {
    let (client, host) = client(Case::Reconcile).await;
    let decision = target(&client)
        .reconcile(
            sdk::ReconcileOutcome::Completed,
            "valid-evidence".into(),
            mutation(),
        )
        .await
        .unwrap();
    let saved = decision.reference_json();
    let restored = client
        .restore(saved, client.operation_kinds().operation_reconcile)
        .unwrap();
    let result = restored.wait(options()).await.unwrap();
    assert_eq!(result.value.target.as_str(), "original-send");
    assert_eq!(result.value.observed_state, sdk::OperationState::Completed);
    assert_eq!(result.value.evidence_id.as_deref(), Some("valid-evidence"));
    client.close(options()).await.unwrap();
    host.await.unwrap();
}
#[tokio::test]
async fn wrong_target_receipt_and_server_failure_preserve_original_decision_uncertainty() {
    for case in [Case::WrongTarget, Case::Unavailable] {
        let (client, host) = client(case).await;
        let error = target(&client).cancel(mutation()).await.unwrap_err();
        assert_eq!(error.certainty, sdk::Certainty::Uncertain);
        assert_eq!(
            error.operation.as_ref().unwrap().operation_id.as_str(),
            "decision-one"
        );
        if matches!(case, Case::Unavailable) {
            assert_eq!(error.problem.unwrap().code, 503);
            assert_eq!(error.kind, sdk::ErrorKind::Application);
        } else {
            assert_eq!(error.kind, sdk::ErrorKind::Protocol);
        }
        client.close(options()).await.unwrap();
        host.await.unwrap();
    }
}

#[tokio::test]
async fn status_wait_recovers_signed_503_by_reading_only_the_original_decision() {
    let (client, host) = client(Case::StatusRecover).await;
    let operation = target(&client).cancel(mutation()).await.unwrap();
    let original = operation.reference_json();
    let result = operation.wait(options()).await.unwrap();
    assert_eq!(result.snapshot.state, sdk::OperationState::Completed);
    assert_eq!(result.value.target.as_str(), "original-send");
    assert_eq!(operation.reference_json(), original);
    client.close(options()).await.unwrap();
    host.await.unwrap();
}

#[tokio::test]
async fn status_wait_until_ready_retains_its_original_after_signed_503() {
    let (client, host) = client(Case::StatusReady).await;
    let operation = target(&client);
    assert_eq!(
        operation.wait_until_ready(options()).await.unwrap().state,
        sdk::OperationState::Ready
    );
    client.close(options()).await.unwrap();
    host.await.unwrap();
}

#[tokio::test]
async fn status_wait_preserves_current_forbidden_after_a_temporary_read_refusal() {
    let (client, host) = client(Case::StatusForbidden).await;
    let operation = target(&client).cancel(mutation()).await.unwrap();
    let error = operation.wait(options()).await.err().unwrap();
    assert_eq!(error.kind, sdk::ErrorKind::Authentication);
    assert_eq!(error.problem.unwrap().code, 403);
    assert_eq!(
        error.operation.unwrap().operation_id.as_str(),
        "decision-one"
    );
    client.close(options()).await.unwrap();
    host.await.unwrap();
}

#[tokio::test]
async fn status_wait_does_not_renew_its_deadline_during_repeated_signed_503_reads() {
    let (client, host) = client(Case::StatusDeadline).await;
    let operation = target(&client).cancel(mutation()).await.unwrap();
    let began = tokio::time::Instant::now();
    let error = operation
        .wait(RequestOptions::new(Duration::from_millis(300)).unwrap())
        .await
        .err()
        .unwrap();
    assert_eq!(error.kind, sdk::ErrorKind::Deadline);
    assert_eq!(error.certainty, sdk::Certainty::ReadOnly);
    assert!(began.elapsed() < Duration::from_secs(1));
    client.close(options()).await.unwrap();
    host.await.unwrap();
}

#[tokio::test]
async fn status_wait_cancellation_interrupts_temporary_refusals_without_resubmission() {
    let (client, host) = client(Case::StatusCancel).await;
    let operation = target(&client).cancel(mutation()).await.unwrap();
    let request = options();
    let signal = request.signal.clone();
    let cancel = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(180)).await;
        signal.cancel();
    });
    let error = operation.wait(request).await.err().unwrap();
    assert_eq!(error.kind, sdk::ErrorKind::Cancelled);
    assert_eq!(error.certainty, sdk::Certainty::ReadOnly);
    cancel.await.unwrap();
    client.close(options()).await.unwrap();
    host.await.unwrap();
}
