//! Real signed control validates handoff/move captures, never foreign media custody.
use super::*;
use crate::sdk::{
    self, Client, HandoffInput, HandoffRole, MoveInput, MoveTarget, MutationOptions, Outgoing,
    RequestOptions,
};
#[derive(Clone, Copy, Debug)]
pub(in crate::canonical) enum Case {
    Handoff,
    Move,
    Candidate,
    Unknown,
    Slow,
    WrongKind,
    WrongSource,
    WrongEndpoint,
    ZeroGeneration,
    Duplicate,
    MissingTopology,
    WrongRole,
    ParentRead,
    ParentRefused,
    Failed,
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
fn rev() -> sdk::TopologyRevision {
    sdk::TopologyRevision::try_from(3).unwrap()
}
fn is_handoff(case: Case) -> bool {
    matches!(
        case,
        Case::Handoff | Case::WrongRole | Case::ParentRead | Case::ParentRefused
    )
}
fn parent(known: bool) -> Value {
    let mut v = json!({"schema":"connect-operation-reference/1","operation_id":"consult_original","command":"session.consult","scope":{"conversation_id":"conv_one","session_id":"session_one","connection_id":"source"},"continuation":{"kind":"consultation","retained_connection_id":"remote"}});
    if known {
        v["continuation"]["record_id"] = json!("host_record_consult");
    }
    v
}
fn handoff_input(
    client: &Client,
    conversation: bool,
    outgoing: Outgoing,
    parent_known: Option<bool>,
) -> HandoffInput {
    HandoffInput {
        via: "source".parse().unwrap(),
        from_participant: "p_from".parse().unwrap(),
        to: "p_to".parse().unwrap(),
        retain: "remote".parse().unwrap(),
        outgoing,
        role: if conversation {
            HandoffRole::Conversation {
                expected_conversation_revision: sdk::ConversationRevision::try_from(7).unwrap(),
            }
        } else {
            HandoffRole::Session
        },
        expected_topology_revision: rev(),
        candidate: Some("candidate".parse().unwrap()),
        consultation: parent_known.map(|known| {
            client
                .restore(parent(known), client.operation_kinds().session_consult)
                .unwrap()
                .reference()
        }),
    }
}
fn move_input(candidate: bool) -> MoveInput {
    MoveInput {
        retain: "remote".parse().unwrap(),
        target: if candidate {
            MoveTarget::Candidate("candidate".parse().unwrap())
        } else {
            MoveTarget::Endpoint("endpoint_new".parse().unwrap())
        },
        expected_topology_revision: rev(),
    }
}
fn saved_move(candidate: bool) -> Value {
    json!({"schema":"connect-operation-reference/1","operation_id":"move_original","command":"connection.replace","scope":{"conversation_id":"conv_one","session_id":"session_one","connection_id":"source"},"continuation":{"kind":"replacement","retained_connection_id":"remote","target":{"kind":if candidate{"candidate"}else{"endpoint"},"id":if candidate{"candidate"}else{"endpoint_new"}}}})
}
fn completion(case: Case, request_body: &Value) -> Value {
    let handoff = is_handoff(case);
    let conversation = request_body["role_scope"] == "conversation";
    let leave = request_body["outgoing"] == "leave";
    let mut v = json!({"retained_connection_ids":["remote"],"admitted_connection_ids":["candidate"],"retired_connection_ids":if !handoff||leave{json!(["source"])}else{json!([])},"session_revision":4,"conversation_revision":if conversation{json!(8)}else{Value::Null}});
    if handoff {
        v["native_handoff"] = json!({"participant_revision":55,"role_scope":if matches!(case,Case::WrongRole){"conversation"}else if conversation{"conversation"}else{"session"},"outgoing":if leave{"leave"}else{"observe"},"source_participant":"p_from","target_participant":"p_to","cleanup":if leave{"pending"}else{"not_requested"}});
    } else {
        for (k,value) in json!({"connection_id":"candidate","replaces":"source","generation":2,"participant_id":"p_from","membership_epoch":9,"endpoint_id":"endpoint_new","device_id":"device_new","readiness":{"streams":["voice"],"stream_revision":11,"participant_revision":55}}).as_object().unwrap(){v[k]=value.clone();}
    }
    match case {
        Case::WrongSource => v["replaces"] = json!("other"),
        Case::WrongEndpoint => v["endpoint_id"] = json!("other"),
        Case::ZeroGeneration => v["generation"] = json!(0),
        Case::Duplicate => v["retired_connection_ids"] = json!(["candidate"]),
        Case::MissingTopology => {
            v.as_object_mut().unwrap().remove("retained_connection_ids");
        }
        _ => (),
    };
    v
}
async fn read(reader: &mut quinn::RecvStream) -> Option<Envelope> {
    let mut p = [0; 4];
    reader.read_exact(&mut p).await.ok()?;
    let n = u32::from_be_bytes(p) as usize;
    assert!(n <= MAX_ENVELOPE_BYTES);
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
    for field in ["cid", "sid", "connid", "operation_id"] {
        if let Some(v) = request.original().get(field) {
            frame[field] = v.clone();
        }
    }
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
pub(super) async fn emit(
    case: Case,
    first: Envelope,
    writer: &mut quinn::SendStream,
    reader: &mut quinn::RecvStream,
    signer: &Sig9421Signer,
    signal: &Notify,
) {
    let mut next = Some(first);
    let mut count = 0;
    let mut gets = 0;
    let mut body = json!({});
    while let Some(request) = next.take() {
        if request.kind() == "operation.get"
            && request.original()["payload"]["operation_id"] == "consult_original"
        {
            assert!(
                matches!(case, Case::ParentRead | Case::ParentRefused),
                "known parent must need no read"
            );
            if matches!(case, Case::ParentRefused) {
                write(
                    writer,
                    signer,
                    &request,
                    "error",
                    json!({"code":403,"category":"auth","reason":"Parent read refused"}),
                )
                .await;
            } else {
                write(writer,signer,&request,"operation.updated",json!({"operation_id":"consult_original","record_id":"host_record_consult","command_type":"session.consult","expected_revision":2,"state":"unknown","revision":1})).await;
                write(writer, signer, &request, "ack", json!({})).await;
            }
            next = read(reader).await;
            continue;
        }
        let query = request.kind() == "operation.get";
        let command = if is_handoff(case) {
            "session.handoff"
        } else {
            "connection.replace"
        };
        let op = if is_handoff(case) {
            "handoff_original"
        } else {
            "move_original"
        };
        assert_eq!(request.scope(Scope::Connection), Some("source"));
        if query {
            gets += 1;
            assert_eq!(request.original()["payload"]["operation_id"], op);
        } else {
            count += 1;
            assert_eq!(
                count, 1,
                "status and restoration never repeat a logical mutation"
            );
            assert_eq!(request.kind(), command);
            assert_eq!(request.scope(Scope::Operation), Some(op));
            assert_eq!(request.original()["expected_revision"], 3);
            body = request.original()["payload"].clone();
            if body.get("opaque").is_some() {
                assert_eq!(body["opaque"], json!([2, 1]));
                assert!(body.as_object().unwrap().contains_key("nullable"));
                assert!(body["nullable"].is_null());
            }
            assert_eq!(body["retained_connid"], "remote");
            if is_handoff(case) {
                assert_eq!(body["source_participant"], "p_from");
                assert_eq!(body["target_participant"], "p_to");
                assert_eq!(body["candidate_connid"], "candidate");
                assert_eq!(body["consultation_record_id"], "host_record_consult");
            } else if matches!(case, Case::Candidate) {
                assert_eq!(body["candidate_connid"], "candidate");
                assert!(body["endpoint_id"].is_null());
            } else {
                assert_eq!(body["endpoint_id"], "endpoint_new");
                assert!(body["candidate_connid"].is_null());
            }
            if matches!(case, Case::Slow) {
                signal.notify_one();
                tokio::time::sleep(Duration::from_millis(120)).await;
            }
        }
        let completed = query && (!matches!(case, Case::Unknown) || gets > 1)
            || matches!(
                case,
                Case::WrongSource
                    | Case::WrongEndpoint
                    | Case::ZeroGeneration
                    | Case::Duplicate
                    | Case::MissingTopology
                    | Case::WrongRole
            );
        let mut value = json!({"operation_id":op,"record_id":"host_record_move","command_type":if matches!(case,Case::WrongKind){"session.consult"}else{command},"expected_revision":3,"state":if matches!(case,Case::Failed){"failed"}else if completed{"completed"}else if query{"unknown"}else{"accepted"},"revision":1});
        if completed && !matches!(case, Case::Failed) {
            value["result"] = completion(case, &body);
        }
        if matches!(case, Case::Failed) {
            value["problem"] = json!({"code":"forbidden","message":"Recorded preflight refusal"});
        }
        write(writer, signer, &request, "operation.updated", value).await;
        write(writer, signer, &request, "ack", json!({})).await;
        next = read(reader).await;
    }
}
async fn client(case: Case) -> (Client, tokio::task::JoinHandle<()>, Arc<Notify>) {
    let (peer, host, signal) = controlled(Fault::SdkMovement(case)).await;
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
fn connection(client: &Client) -> sdk::Connection {
    session(client).connection_by_id("source".parse().unwrap())
}
async fn close(client: Client, host: tokio::task::JoinHandle<()>) {
    client.close(options()).await.unwrap();
    host.await.unwrap();
}
#[tokio::test]
async fn handoff_preserves_role_scope_outgoing_and_saved_parent_without_borrowing_media() {
    for conversation in [false, true] {
        for outgoing in [Outgoing::Leave, Outgoing::Observe] {
            let (client, host, _) = client(Case::Handoff).await;
            let mut opts = mutation("handoff_original");
            opts.extensions = Some(std::collections::BTreeMap::from([
                ("policy_ref".into(), json!("policy_record")),
                ("opaque".into(), json!([2, 1])),
                ("nullable".into(), Value::Null),
            ]));
            let op = session(&client)
                .handoff(
                    handoff_input(&client, conversation, outgoing, Some(true)),
                    opts,
                )
                .await
                .unwrap();
            let restored = client
                .restore(
                    op.reference_json(),
                    client.operation_kinds().session_handoff,
                )
                .unwrap();
            let value = restored.wait(options()).await.unwrap().value;
            assert_eq!(value.topology.topology_revision.get(), 4);
            assert_eq!(
                value.topology.conversation_revision.map(|v| v.get()),
                conversation.then_some(8)
            );
            let role = value.role_change.unwrap();
            assert_eq!(role.participant_revision, 55);
            assert_eq!(role.outgoing, outgoing);
            assert!(value.topology.admitted[0]
                .native_negotiation()
                .unwrap()
                .is_none());
            close(client, host).await;
        }
    }
}
#[tokio::test]
async fn device_move_captures_exact_target_topology_and_distinct_generation() {
    for candidate in [false, true] {
        let (client, host, _) = client(if candidate {
            Case::Candidate
        } else {
            Case::Move
        })
        .await;
        let op = connection(&client)
            .replace_device(move_input(candidate), mutation("move_original"))
            .await
            .unwrap();
        assert_eq!(op.reference_json(), saved_move(candidate));
        let restored = client
            .restore(
                op.reference_json(),
                client.operation_kinds().connection_replace,
            )
            .unwrap();
        let result = restored.wait(options()).await.unwrap().value;
        assert_eq!(result.connection.id().as_str(), "candidate");
        assert_eq!(result.binding.generation, 2);
        assert_eq!(result.binding.membership_epoch, 9);
        assert_eq!(result.topology.topology_revision.get(), 4);
        assert_eq!(result.topology.retained[0].id().as_str(), "remote");
        assert_eq!(result.topology.retired[0].as_str(), "source");
        assert!(result.connection.native_negotiation().unwrap().is_none());
        close(client, host).await;
    }
}
#[tokio::test]
async fn movement_unknown_status_recovers_original_without_repeating_effect() {
    let (client, host, _) = client(Case::Unknown).await;
    let op = connection(&client)
        .replace_device(move_input(false), mutation("move_original"))
        .await
        .unwrap();
    assert_eq!(
        op.status(options()).await.unwrap().state,
        sdk::OperationState::Unknown
    );
    assert_eq!(
        op.wait(options()).await.unwrap().value.binding.generation,
        2
    );
    close(client, host).await;
}
#[tokio::test]
async fn cancelled_move_caller_keeps_submission_owned_and_restored_reference_recovers() {
    let (client, host, signal) = client(Case::Slow).await;
    let source = connection(&client);
    let pending = tokio::spawn(async move {
        source
            .replace_device(move_input(false), mutation("move_original"))
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), signal.notified())
        .await
        .expect("move must actually reach the signed peer");
    pending.abort();
    let _ = pending.await;
    let recovered = client
        .restore(
            saved_move(false),
            client.operation_kinds().connection_replace,
        )
        .unwrap();
    assert_eq!(
        recovered
            .wait(options())
            .await
            .unwrap()
            .value
            .connection
            .id()
            .as_str(),
        "candidate"
    );
    close(client, host).await;
}
#[tokio::test]
async fn malformed_signed_movement_captures_preserve_original_uncertainty() {
    for case in [
        Case::WrongKind,
        Case::WrongSource,
        Case::WrongEndpoint,
        Case::ZeroGeneration,
        Case::Duplicate,
        Case::MissingTopology,
        Case::WrongRole,
    ] {
        let (client, host, _) = client(case).await;
        let error = if is_handoff(case) {
            session(&client)
                .handoff(
                    handoff_input(&client, false, Outgoing::Leave, Some(true)),
                    mutation("handoff_original"),
                )
                .await
                .unwrap_err()
        } else {
            connection(&client)
                .replace_device(move_input(false), mutation("move_original"))
                .await
                .unwrap_err()
        };
        assert_eq!(error.kind, sdk::ErrorKind::Protocol, "{case:?}");
        assert_eq!(error.certainty, sdk::Certainty::Uncertain);
        close(client, host).await;
    }
}
#[tokio::test]
async fn movement_intent_and_reference_validation_precede_any_mutation() {
    let (client, host, _) = client(Case::Move).await;
    for target in [
        MoveTarget::Candidate("source".parse().unwrap()),
        MoveTarget::Candidate("remote".parse().unwrap()),
    ] {
        let error = connection(&client)
            .replace_device(
                MoveInput {
                    target,
                    retain: "remote".parse().unwrap(),
                    expected_topology_revision: rev(),
                },
                mutation("invalid"),
            )
            .await
            .unwrap_err();
        assert_eq!(error.certainty, sdk::Certainty::NotSubmitted);
    }
    let mut input = handoff_input(&client, false, Outgoing::Observe, Some(true));
    input.to = input.from_participant.clone();
    assert!(
        matches!(session(&client).handoff(input,mutation("invalid")).await,Err(e) if e.certainty==sdk::Certainty::NotSubmitted)
    );
    let mut wrong = saved_move(false);
    wrong["continuation"]["retained_connection_id"] = json!("source");
    assert!(client
        .restore(wrong, client.operation_kinds().connection_replace)
        .is_err());
    let mut wrong = saved_move(false);
    wrong["continuation"]["kind"] = json!("consultation");
    assert!(client
        .restore(wrong, client.operation_kinds().connection_replace)
        .is_err());
    let op = connection(&client)
        .replace_device(move_input(false), mutation("move_original"))
        .await
        .unwrap();
    assert_eq!(
        op.wait(options())
            .await
            .unwrap()
            .value
            .topology
            .topology_revision
            .get(),
        4
    );
    close(client, host).await;
}
#[tokio::test]
async fn handoff_unobserved_parent_identity_uses_authorized_read_or_refuses_before_submission() {
    for case in [Case::ParentRead, Case::ParentRefused] {
        let (client, host, _) = client(case).await;
        let result = session(&client)
            .handoff(
                handoff_input(&client, false, Outgoing::Leave, Some(false)),
                mutation("handoff_original"),
            )
            .await;
        if matches!(case, Case::ParentRead) {
            let op = result.unwrap();
            assert!(op
                .wait(options())
                .await
                .unwrap()
                .value
                .role_change
                .is_some());
        } else {
            let e = result.unwrap_err();
            assert_eq!(e.certainty, sdk::Certainty::NotSubmitted);
            assert_eq!(
                e.operation.unwrap().operation_id.as_str(),
                "handoff_original"
            );
        }
        close(client, host).await;
    }
}
#[tokio::test]
async fn recorded_movement_failure_remains_a_recorded_observation() {
    let (client, host, _) = client(Case::Failed).await;
    let op = connection(&client)
        .replace_device(move_input(false), mutation("move_original"))
        .await
        .unwrap();
    let error = op.wait(options()).await.err().expect("recorded refusal");
    assert_eq!(error.certainty, sdk::Certainty::Recorded);
    assert_eq!(error.kind, sdk::ErrorKind::Application);
    assert_eq!(
        error.observation.unwrap().problem.unwrap()["code"],
        "forbidden"
    );
    close(client, host).await;
}
