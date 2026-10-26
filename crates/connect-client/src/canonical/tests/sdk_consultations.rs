//! Signed control receipts qualify original-record continuation, not physical private media.
use super::*;
use crate::sdk::{self, Client, ConsultInput, Hold, MutationOptions, NonEmpty, RequestOptions};
#[derive(Clone, Copy, Debug)]
pub(in crate::canonical) enum Case {
    Normal,
    Unknown,
    Slow,
    WrongKind,
    MissingRecord,
    Duplicate,
    WrongRetained,
    WrongParent,
    RemoteEof,
    WrongRevision,
}
fn options() -> RequestOptions {
    RequestOptions::new(Duration::from_secs(3)).unwrap()
}
fn mutation(id: &str) -> MutationOptions {
    MutationOptions {
        operation_id: id.parse().unwrap(),
        request: options(),
        extensions: None,
    }
}
fn rev(n: u64) -> sdk::TopologyRevision {
    sdk::TopologyRevision::try_from(n).unwrap()
}
fn input() -> ConsultInput {
    ConsultInput {
        via: "primary".parse().unwrap(),
        retain: "remote".parse().unwrap(),
        with_participants: NonEmpty {
            first: "p_two".parse().unwrap(),
            rest: vec!["p_one".parse().unwrap()],
        },
        hold: Hold::ConfiguredMedia,
        expected_topology_revision: rev(2),
    }
}
fn saved() -> Value {
    json!({"schema":"connect-operation-reference/1","operation_id":"consult_original","command":"session.consult","scope":{"conversation_id":"conv_one","session_id":"session_one","connection_id":"primary"},"continuation":{"kind":"consultation","retained_connection_id":"remote"}})
}
fn result(case: Case, returned: bool) -> Value {
    let mut value = json!({"retained_connection_ids":["primary","remote"],"admitted_connection_ids":if returned{json!([])}else{json!(["candidate"])},"retired_connection_ids":if returned{json!(["candidate"])}else{json!([])},"session_revision":if returned{4}else{3},"conversation_revision":null});
    if matches!(case, Case::Duplicate) {
        value["admitted_connection_ids"] = json!(["remote"]);
    }
    if matches!(case, Case::WrongRetained) {
        value["retained_connection_ids"] = json!(["primary"]);
    }
    if returned {
        value["consultation_record_id"] = json!(if matches!(case, Case::WrongParent) {
            "foreign_record"
        } else {
            "host_record_original"
        });
        value["remote_continuing"] = json!(!matches!(case, Case::RemoteEof));
        if matches!(case, Case::RemoteEof) {
            value["retained_connection_ids"] = json!(["primary"]);
            value["retired_connection_ids"] = json!(["candidate", "remote"]);
        }
    }
    value
}
async fn write(
    writer: &mut quinn::SendStream,
    signer: &Sig9421Signer,
    request: &Envelope,
    payload: Value,
    kind: &str,
) {
    let mut frame = envelope(kind, payload);
    for field in ["cid", "sid", "connid", "operation_id"] {
        if let Some(v) = request.original().get(field) {
            frame[field] = v.clone();
        }
    }
    frame["in_reply_to"] = json!(request.id());
    let frame = signer.sign(&frame).unwrap();
    Envelope::from_value(frame.clone()).unwrap();
    let raw = serde_json::to_vec(&frame).unwrap();
    writer
        .write_all(&(raw.len() as u32).to_be_bytes())
        .await
        .unwrap();
    writer.write_all(&raw).await.unwrap();
}
async fn read(reader: &mut quinn::RecvStream) -> Option<Envelope> {
    let mut p = [0; 4];
    reader.read_exact(&mut p).await.ok()?;
    let n = u32::from_be_bytes(p) as usize;
    assert!(n <= MAX_ENVELOPE_BYTES);
    let mut b = vec![0; n];
    reader.read_exact(&mut b).await.ok()?;
    Some(Envelope::parse(&b).unwrap())
}
pub(super) async fn emit(
    case: Case,
    first: Envelope,
    writer: &mut quinn::SendStream,
    reader: &mut quinn::RecvStream,
    signer: &Sig9421Signer,
    signal: &Notify,
) {
    let mut frame = Some(first);
    let mut consults = 0;
    let mut gets = 0;
    while let Some(request) = frame.take() {
        let query = request.kind() == "operation.get";
        let id = if query {
            request.original()["payload"]["operation_id"]
                .as_str()
                .unwrap()
        } else {
            request.scope(Scope::Operation).unwrap()
        };
        let returned = id == "return_original";
        let command = if returned {
            "session.consult.return"
        } else {
            "session.consult"
        };
        assert_eq!(request.scope(Scope::Connection), Some("primary"));
        if !query {
            assert_eq!(request.kind(), command);
            if returned {
                assert_eq!(
                    request.original()["payload"]["consultation_record_id"],
                    "host_record_original"
                );
                assert_eq!(request.original()["payload"]["retained_connid"], "remote");
                assert_eq!(request.original()["expected_revision"], 3);
            } else {
                consults += 1;
                assert_eq!(
                    consults, 1,
                    "status/return never resubmit the original consultation"
                );
                assert_eq!(
                    request.original()["payload"]["private_participants"],
                    json!(["p_one", "p_two"])
                );
                assert_eq!(
                    request.original()["payload"]["hold_treatment"],
                    "configured-media"
                );
                assert_eq!(request.original()["expected_revision"], 2);
                if matches!(case, Case::Slow) {
                    signal.notify_one();
                    tokio::time::sleep(Duration::from_millis(120)).await;
                }
            }
        } else {
            gets += 1;
        }
        let completed = returned
            || query
                && !matches!(case, Case::Unknown)
                && !matches!(
                    case,
                    Case::WrongKind | Case::MissingRecord | Case::WrongRevision
                )
            || query && gets > 1
            || matches!(case, Case::Duplicate | Case::WrongRetained);
        let mut value = json!({"operation_id":id,"record_id":if returned{"host_record_return"}else{"host_record_original"},"command_type":if matches!(case,Case::WrongKind){"session.handoff"}else{command},"expected_revision":if matches!(case,Case::WrongRevision){99}else if returned{3}else{2},"state":if completed{"completed"}else if query{"unknown"}else{"accepted"},"revision":1});
        if completed {
            value["result"] = result(case, returned);
        }
        if matches!(case, Case::MissingRecord) {
            value.as_object_mut().unwrap().remove("record_id");
        }
        write(writer, signer, &request, value, "operation.updated").await;
        write(writer, signer, &request, json!({}), "ack").await;
        frame = read(reader).await;
    }
}
async fn client(case: Case) -> (Client, tokio::task::JoinHandle<()>, Arc<Notify>) {
    let (peer, host, signal) = controlled(Fault::SdkConsultation(case)).await;
    (
        Client::from_authenticated_peer(peer, "tenant_one".parse().unwrap()).unwrap(),
        host,
        signal,
    )
}
fn session(client: &Client) -> sdk::Session {
    client
        .conversation_by_id("conv_one".parse().unwrap())
        .session_by_id("session_one".parse().unwrap())
}
async fn close(client: Client, host: tokio::task::JoinHandle<()>) {
    client.close(options()).await.unwrap();
    host.await.unwrap();
}
#[tokio::test]
async fn consultation_reference_roundtrip_recovers_original_record_then_returns_without_resubmission(
) {
    let (client, host, _) = client(Case::Normal).await;
    let mut opts = mutation("consult_original");
    opts.extensions = Some(serde_json::from_value(json!({"opaque":{"order":[2,1]}})).unwrap());
    let op = session(&client).consult(input(), opts).await.unwrap();
    assert_eq!(op.reference_json(), saved());
    let restored = client
        .restore(
            op.reference_json(),
            client.operation_kinds().session_consult,
        )
        .unwrap();
    let result = restored.wait(options()).await.unwrap();
    assert_eq!(
        result.snapshot.record_id.unwrap().as_str(),
        "host_record_original"
    );
    assert_eq!(result.value.topology.topology_revision.get(), 3);
    assert_eq!(result.value.topology.admitted[0].id().as_str(), "candidate");
    assert!(result.value.topology.admitted[0]
        .native_negotiation()
        .unwrap()
        .is_none());
    assert!(result.value.session.receipt_snapshot().is_none());
    let returned = restored
        .return_to_retained(rev(3), mutation("return_original"))
        .await
        .unwrap();
    let returned = client
        .restore(
            returned.reference_json(),
            client.operation_kinds().session_consult_return,
        )
        .unwrap();
    let result = returned.wait(options()).await.unwrap().value;
    let receipt = result.consultation_return_receipt().unwrap();
    assert!(receipt.remote_continuing);
    assert_eq!(receipt.topology.retired[0].as_str(), "candidate");
    assert_eq!(receipt.topology.topology_revision.get(), 4);
    close(client, host).await;
}
#[tokio::test]
async fn consultation_unknown_is_observation_and_original_completion_can_be_recovered() {
    let (client, host, _) = client(Case::Unknown).await;
    let op = session(&client)
        .consult(input(), mutation("consult_original"))
        .await
        .unwrap();
    assert_eq!(
        op.status(options()).await.unwrap().state,
        sdk::OperationState::Unknown
    );
    assert_eq!(
        op.wait(options())
            .await
            .unwrap()
            .value
            .topology
            .topology_revision
            .get(),
        3
    );
    close(client, host).await;
}
#[tokio::test]
async fn consultation_cancelled_caller_keeps_owned_submission_and_saved_intent_recovers() {
    let (client, host, signal) = client(Case::Slow).await;
    let room = session(&client);
    let pending =
        tokio::spawn(async move { room.consult(input(), mutation("consult_original")).await });
    signal.notified().await;
    pending.abort();
    let _ = pending.await;
    let restored = client
        .restore(saved(), client.operation_kinds().session_consult)
        .unwrap();
    assert_eq!(
        restored.wait(options()).await.unwrap().snapshot.state,
        sdk::OperationState::Completed
    );
    close(client, host).await;
}
#[tokio::test]
async fn consultation_malformed_signed_receipts_keep_original_uncertainty() {
    for case in [
        Case::WrongKind,
        Case::MissingRecord,
        Case::WrongRevision,
        Case::Duplicate,
        Case::WrongRetained,
    ] {
        let (client, host, _) = client(case).await;
        let error = session(&client)
            .consult(input(), mutation("consult_original"))
            .await
            .unwrap_err();
        assert_eq!(error.kind, sdk::ErrorKind::Protocol, "{case:?}");
        assert_eq!(error.certainty, sdk::Certainty::Uncertain);
        assert_eq!(
            error.operation.unwrap().operation_id.as_str(),
            "consult_original"
        );
        close(client, host).await;
    }
}
#[tokio::test]
async fn consultation_return_refuses_foreign_parent_and_preserves_remote_eof_as_observation() {
    for case in [Case::WrongParent, Case::RemoteEof] {
        let (client, host, _) = client(case).await;
        let op = session(&client)
            .consult(input(), mutation("consult_original"))
            .await
            .unwrap();
        let returned = op
            .return_to_retained(rev(3), mutation("return_original"))
            .await;
        if matches!(case, Case::WrongParent) {
            let e = returned.unwrap_err();
            assert_eq!(e.kind, sdk::ErrorKind::Protocol);
            assert_eq!(
                e.operation.unwrap().operation_id.as_str(),
                "return_original"
            );
        } else {
            let returned = returned.unwrap();
            let value = returned.wait(options()).await.unwrap().value;
            assert!(
                !value
                    .consultation_return_receipt()
                    .unwrap()
                    .remote_continuing
            );
            assert_eq!(
                value
                    .consultation_return_receipt()
                    .unwrap()
                    .topology
                    .retired
                    .len(),
                2
            );
        }
        close(client, host).await;
    }
}
#[tokio::test]
async fn consultation_local_intent_and_saved_continuation_validation_precede_mutation() {
    let (client, host, _) = client(Case::Normal).await;
    for kind in [0, 1, 2] {
        let mut v = input();
        let mut options = mutation("invalid_local");
        match kind {
            0 => v.retain = v.via.clone(),
            1 => v.with_participants.rest = vec![v.with_participants.first.clone()],
            _ => {
                options.extensions =
                    Some(serde_json::from_value(json!({"retained_connid":"override"})).unwrap())
            }
        };
        let e = session(&client).consult(v, options).await.unwrap_err();
        assert_eq!(e.certainty, sdk::Certainty::NotSubmitted);
    }
    for field in ["continuation", "scope"] {
        let mut v = saved();
        v.as_object_mut().unwrap().remove(field);
        assert!(client
            .restore(v, client.operation_kinds().session_consult)
            .is_err());
    }
    let mut wrong = saved();
    wrong["continuation"]["retained_connection_id"] = json!("primary");
    assert!(client
        .restore(wrong, client.operation_kinds().session_consult)
        .is_err());
    let op = session(&client)
        .consult(input(), mutation("consult_original"))
        .await
        .unwrap();
    assert!(
        matches!(op.return_to_retained(rev(3),mutation("consult_original")).await,Err(e) if e.certainty==sdk::Certainty::NotSubmitted)
    );
    close(client, host).await;
}
