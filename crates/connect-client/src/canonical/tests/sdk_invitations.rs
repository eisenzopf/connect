//! Signed incoming metadata stays bounded and independent of control ownership.
use super::*;
use crate::sdk::{self, Client, ErrorKind, RequestOptions};
#[derive(Clone, Copy, Debug)]
pub(in crate::canonical) enum Case {
    Early,
    Delayed,
    Cancel,
    Changed,
    NoFrom,
    ManyTargets,
    Scope,
    WrongCancel,
    Count,
    Bytes,
    Repeat,
}
fn options() -> RequestOptions {
    RequestOptions::new(Duration::from_secs(2)).unwrap()
}
fn short() -> RequestOptions {
    RequestOptions::new(Duration::from_millis(60)).unwrap()
}
async fn raw(writer: &mut quinn::SendStream, signer: &Sig9421Signer, value: Value) {
    let signed = signer.sign(&value).unwrap();
    Envelope::from_value(signed.clone()).unwrap();
    let b = serde_json::to_vec(&signed).unwrap();
    writer
        .write_all(&(b.len() as u32).to_be_bytes())
        .await
        .unwrap();
    writer.write_all(&b).await.unwrap();
}
fn invitation(index: usize, wire: usize, case: Case) -> Value {
    let mut value = envelope(
        "session.invite",
        json!({"from":"part_sender","to":["part_target"],"medium":"voice","intent":"conference","capabilities_offer":{"audio":["g.711-mu"],"private":"never in Debug"}}),
    );
    value["id"] = json!(format!("invite-wire-{wire}"));
    value["cid"] = json!("conv_one");
    value["sid"] = json!(format!("session-{index}"));
    value["operation_id"] = json!(format!("original-invite-{index}"));
    match case {
        Case::Changed => value["payload"]["capabilities_offer"]["audio"] = json!(["opus"]),
        Case::NoFrom => {
            value["payload"].as_object_mut().unwrap().remove("from");
        }
        Case::ManyTargets => value["payload"]["to"] = json!(["part_target", "part_other"]),
        Case::Scope => value["connid"] = json!("foreign-connection"),
        Case::Bytes => value["payload"]["extension"] = json!("x".repeat(600_000)),
        _ => (),
    };
    value
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
async fn control(writer: &mut quinn::SendStream, signer: &Sig9421Signer, request: &Envelope) {
    assert_eq!(request.kind(), "operation.get");
    sdk_worker::write(
        writer,
        signer,
        request,
        "operation.updated",
        json!({"operation_id":"control","state":"unknown","revision":1}),
    )
    .await;
    sdk_worker::write(writer, signer, request, "ack", json!({})).await;
}
pub(super) async fn emit(
    case: Case,
    first: Envelope,
    writer: &mut quinn::SendStream,
    reader: &mut quinn::RecvStream,
    signer: &Sig9421Signer,
    signal: &Notify,
) {
    if matches!(case, Case::Delayed) {
        control(writer, signer, &first).await;
        signal.notified().await;
        raw(writer, signer, invitation(0, 0, Case::Early)).await;
    } else {
        let n = if matches!(case, Case::Count) {
            33
        } else if matches!(case, Case::Bytes) {
            2
        } else {
            1
        };
        for i in 0..n {
            raw(
                writer,
                signer,
                invitation(
                    i,
                    i,
                    if matches!(case, Case::Changed) {
                        Case::Early
                    } else {
                        case
                    },
                ),
            )
            .await;
        }
        if matches!(case, Case::Changed | Case::Early) {
            raw(writer, signer, invitation(0, 100, case)).await;
        }
        if matches!(case, Case::Cancel | Case::WrongCancel) {
            let mut cancel = envelope(
                "session.cancel",
                json!({"reason_code":487,"reason":"cancelled"}),
            );
            cancel["cid"] = json!(if matches!(case, Case::WrongCancel) {
                "foreign"
            } else {
                "conv_one"
            });
            cancel["sid"] = json!("session-0");
            cancel["operation_id"] = json!("original-invite-0");
            cancel["in_reply_to"] = json!("invite-wire-0");
            raw(writer, signer, cancel).await;
            raw(writer, signer, invitation(1, 1, Case::Early)).await;
        }
        control(writer, signer, &first).await;
    }
    let mut calls = 0;
    while let Some(request) = read(reader).await {
        calls += 1;
        if matches!(case, Case::Repeat) && calls == 1 {
            raw(writer, signer, invitation(0, 100, Case::Early)).await;
        }
        control(writer, signer, &request).await;
    }
}
async fn setup(case: Case) -> (Client, tokio::task::JoinHandle<()>, Arc<Notify>) {
    let (peer, host, signal) = controlled(Fault::SdkInvitation(case)).await;
    (
        Client::from_authenticated_peer(peer, "tenant".parse().unwrap()).unwrap(),
        host,
        signal,
    )
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
    tokio::time::timeout(Duration::from_secs(2), host)
        .await
        .unwrap()
        .unwrap();
}
#[tokio::test]
async fn incoming_original_invitation_is_queued_before_reader_and_duplicate_wire_ids_do_not_repeat()
{
    let (client, host, _) = setup(Case::Early).await;
    query(&client).await;
    let mut s = client.incoming_invitations(short());
    let hint = s.next().await.unwrap().unwrap();
    assert_eq!(hint.session.id().as_str(), "session-0");
    assert_eq!(hint.session.conversation_id().as_str(), "conv_one");
    assert_eq!(hint.from_participant.as_str(), "part_sender");
    assert_eq!(hint.to_participant.as_str(), "part_target");
    assert_eq!(hint.operation_id.as_str(), "original-invite-0");
    assert_eq!(hint.source.correlation_id, "original-invite-0");
    assert_eq!(hint.intent, "conference");
    assert_eq!(hint.capabilities_offer["audio"][0], "g.711-mu");
    assert!(!format!("{hint:?}").contains("never in Debug"));
    assert!(matches!(s.next().await,Err(e) if e.kind==ErrorKind::Deadline));
    query(&client).await;
    s.close().await.unwrap();
    assert!(s.next().await.unwrap().is_none());
    close(client, host).await;
}
#[tokio::test]
async fn dropping_and_timing_out_invitation_waits_consumes_nothing_and_replacement_keeps_pending_hint(
) {
    let (client, host, signal) = setup(Case::Delayed).await;
    query(&client).await;
    let mut s = client.incoming_invitations(short());
    assert!(tokio::time::timeout(Duration::from_millis(20), s.next())
        .await
        .is_err());
    assert!(matches!(s.next().await,Err(e) if e.kind==ErrorKind::Deadline));
    s.close().await.unwrap();
    signal.notify_one();
    query(&client).await;
    let mut replacement = client.incoming_invitations(options());
    assert_eq!(
        replacement
            .next()
            .await
            .unwrap()
            .unwrap()
            .operation_id
            .as_str(),
        "original-invite-0"
    );
    replacement.close().await.unwrap();
    close(client, host).await;
}
#[tokio::test]
async fn invitation_reader_conflict_cancellation_drop_and_client_close_are_local_and_bounded() {
    let (client, host, signal) = setup(Case::Delayed).await;
    query(&client).await;
    let opts = short();
    let token = opts.signal.clone();
    let mut s = client.incoming_invitations(opts);
    assert!(matches!(s.next().await,Err(e) if e.kind==ErrorKind::Deadline));
    let mut second = client.clone().incoming_invitations(options());
    assert!(matches!(second.next().await,Err(e) if e.kind==ErrorKind::Conflict));
    token.cancel();
    assert!(matches!(s.next().await,Err(e) if e.kind==ErrorKind::Cancelled));
    drop(s);
    signal.notify_one();
    query(&client).await;
    assert!(second.next().await.unwrap().is_some());
    client.close(options()).await.unwrap();
    assert!(matches!(second.next().await,Err(e) if e.kind==ErrorKind::Cancelled));
    second.close().await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), host)
        .await
        .unwrap()
        .unwrap();
}
#[tokio::test]
async fn exact_server_cancellation_discards_pending_invitation_and_changed_scope_cannot_withdraw_another(
) {
    let (client, host, _) = setup(Case::Cancel).await;
    query(&client).await;
    let mut s = client.incoming_invitations(options());
    assert_eq!(
        s.next().await.unwrap().unwrap().operation_id.as_str(),
        "original-invite-1"
    );
    query(&client).await;
    s.close().await.unwrap();
    close(client, host).await;
    let (client, host, _) = setup(Case::WrongCancel).await;
    query(&client).await;
    let mut s = client.incoming_invitations(options());
    assert!(matches!(s.next().await,Err(e) if e.kind==ErrorKind::Protocol));
    query(&client).await;
    s.close().await.unwrap();
    close(client, host).await;
}
#[tokio::test]
async fn malformed_or_changed_invitation_metadata_fails_only_the_invitation_reader() {
    for case in [Case::Changed, Case::NoFrom, Case::ManyTargets, Case::Scope] {
        let (client, host, _) = setup(case).await;
        query(&client).await;
        let mut s = client.incoming_invitations(options());
        assert!(matches!(s.next().await,Err(e) if e.kind==ErrorKind::Protocol));
        query(&client).await;
        s.close().await.unwrap();
        close(client, host).await;
    }
}
#[tokio::test]
async fn invitation_count_and_byte_overflow_refuses_retained_work_without_stalling_other_control() {
    for case in [Case::Count, Case::Bytes] {
        let (client, host, _) = setup(case).await;
        query(&client).await;
        let mut s = client.incoming_invitations(options());
        assert!(matches!(s.next().await,Err(e) if e.kind==ErrorKind::ResyncRequired));
        s.close().await.unwrap();
        let mut fresh = client.incoming_invitations(options());
        assert!(matches!(fresh.next().await,Err(e) if e.kind==ErrorKind::ResyncRequired));
        query(&client).await;
        fresh.close().await.unwrap();
        close(client, host).await;
    }
}
#[tokio::test]
async fn invitation_repeated_after_application_delivery_is_suppressed_without_another_mutation() {
    let (client, host, _) = setup(Case::Repeat).await;
    query(&client).await;
    let mut s = client.incoming_invitations(short());
    s.next().await.unwrap().unwrap();
    query(&client).await;
    assert!(matches!(s.next().await,Err(e) if e.kind==ErrorKind::Deadline));
    query(&client).await;
    s.close().await.unwrap();
    close(client, host).await;
}
