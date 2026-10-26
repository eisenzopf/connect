//! Signed end outcomes, caller cancellation and exact resource identity.
use super::*;
async fn reply(
    writer: &mut quinn::SendStream,
    signer: &Sig9421Signer,
    request: &Envelope,
    state: &str,
    foreign: bool,
) {
    let mut update = json!({"operation_id":"end-original","state":state,"revision":1});
    if state == "completed" {
        update["result"] = json!({"connection_id":if foreign{"foreign-connection"}else{"connection_one"},"ended":true});
    }
    write(writer, signer, request, "operation.updated", update, true).await;
    write(writer, signer, request, "ack", json!({}), true).await;
}
pub(super) async fn emit(
    case: Case,
    writer: &mut quinn::SendStream,
    reader: &mut quinn::RecvStream,
    signer: &Sig9421Signer,
    signal: &Notify,
) {
    let request = read(reader).await;
    assert_eq!(request.kind(), "connection.end");
    assert_eq!(
        request.original()["payload"],
        json!({"reason_code":200,"reason":"normal"})
    );
    assert_eq!(request.original()["expected_revision"], 1);
    assert_eq!(request.scope(Scope::Operation), Some("end-original"));
    if matches!(case, Case::EndRefused) {
        write(
            writer,
            signer,
            &request,
            "error",
            json!({"code":403,"category":"authorization","reason":"Current owner refused"}),
            true,
        )
        .await;
        return;
    }
    if matches!(case, Case::EndWrongRevision) {
        // Write signed invalid payload bytes directly: the ordinary fixture writer
        // parses its own output and correctly refuses this counter before I/O.
        let mut value = envelope(
            "operation.updated",
            json!({"operation_id":"end-original","state":"completed","revision":9_007_199_254_740_992u64,"result":{"connection_id":"connection_one","ended":true}}),
        );
        for field in ["cid", "sid", "connid", "operation_id"] {
            value[field] = request.original()[field].clone();
        }
        value["in_reply_to"] = json!(request.id());
        let bytes = serde_json::to_vec(&signer.sign(&value).unwrap()).unwrap();
        writer
            .write_all(&(bytes.len() as u32).to_be_bytes())
            .await
            .unwrap();
        writer.write_all(&bytes).await.unwrap();
        return;
    }
    if matches!(case, Case::EndWrongConnection) {
        reply(writer, signer, &request, "completed", true).await;
        return;
    }
    if matches!(case, Case::EndCancelled) {
        signal.notify_one();
        let query = read(reader).await;
        assert_eq!(query.kind(), "operation.get");
        reply(writer, signer, &request, "completed", false).await;
        reply(writer, signer, &query, "completed", false).await;
    } else {
        reply(
            writer,
            signer,
            &request,
            if matches!(case, Case::EndUnknown) {
                "unknown"
            } else {
                "completed"
            },
            false,
        )
        .await;
        let query = read(reader).await;
        assert_eq!(query.kind(), "operation.get");
        reply(
            writer,
            signer,
            &query,
            if matches!(case, Case::EndUnknown) {
                "unknown"
            } else {
                "completed"
            },
            false,
        )
        .await;
    }
    let query = read(reader).await;
    assert_eq!(
        query.kind(),
        "operation.get",
        "recovery must not submit another end"
    );
    assert_eq!(query.original()["payload"]["operation_id"], "end-original");
    reply(writer, signer, &query, "completed", false).await;
    if matches!(case, Case::EndHandleReuse) {
        let request = read(reader).await;
        assert_eq!(request.kind(), "connection.offer");
        assert_eq!(request.scope(Scope::Connection), Some("new-connection"));
        let streams = json!([{"strm_id":"new-audio","kind":"audio","direction":"recvonly","stream_local_id":7,"codec":{"name":"pcmu","params":{"sample_rate":8000,"channels":1,"payload_type":0}}}]);
        let setup = json!({"kind":"native-prepared/1","revision":1,"offer_id":request.id(),"streams":streams});
        let caps = request.original()["payload"]["capabilities"].clone();
        write(writer,signer,&request,"connection.answer",json!({"by_participant":"part_one","substrate":"quic","capabilities":caps,"streams_answered":[{"id":"new-audio","kind":"audio","direction":"recvonly","codec":streams[0]["codec"]}],"substrate_setup":setup}),true).await;
        let query = read(reader).await;
        assert_eq!(query.kind(), "operation.get");
        write(
            writer,
            signer,
            &query,
            "operation.updated",
            json!({"operation_id":"native-reuse","state":"preparing","revision":1}),
            true,
        )
        .await;
        write(writer, signer, &query, "ack", json!({}), true).await;
        write(writer,signer,&request,"connection.offer",json!({"by_participant":"part_one","substrate":"quic","capabilities":caps,"streams_offered":[{"id":"new-audio","kind":"audio","direction":"recvonly","codec_preferences":["pcmu"]}],"substrate_setup":setup}),false).await;
        loop {
            let mut prefix = [0; 4];
            if tokio::time::timeout(Duration::from_secs(3), reader.read_exact(&mut prefix))
                .await
                .unwrap()
                .is_err()
            {
                break;
            }
            let size = u32::from_be_bytes(prefix) as usize;
            assert!(size > 0 && size <= MAX_ENVELOPE_BYTES);
            let mut bytes = vec![0; size];
            if reader.read_exact(&mut bytes).await.is_err() {
                break;
            }
            assert_eq!(
                Envelope::parse(&bytes).unwrap().kind(),
                "operation.get",
                "a retired hop handle cannot receive a new signed confirmation"
            );
        }
    }
}
fn ending() -> MutationOptions {
    MutationOptions {
        operation_id: "end-original".parse().unwrap(),
        request: options(),
        extensions: None,
    }
}
fn saved() -> Value {
    json!({"schema":"connect-operation-reference/1","operation_id":"end-original","command":"connection.end","scope":{"conversation_id":"conv_one","session_id":"session_one","connection_id":"connection_one"}})
}
async fn ready(session: &sdk::Session) -> sdk::Connection {
    let operation = session
        .offer_connection("connection_one".parse().unwrap(), offered(), mutation())
        .await
        .unwrap();
    assert_eq!(
        operation.status(options()).await.unwrap().state,
        sdk::OperationState::Preparing
    );
    let connection = operation.wait(options()).await.unwrap().value;
    assert!(connection.native_negotiation().unwrap().is_some());
    connection
}
#[tokio::test]
async fn native_end_checks_original_receipt_and_restore_without_reacquiring_media() {
    let (c, session, host) = client(Case::End).await;
    let connection = ready(&session).await;
    let operation = connection
        .end(
            200,
            "normal".into(),
            Some(sdk::StreamRevision::try_from(1).unwrap()),
            ending(),
        )
        .await
        .unwrap();
    assert_eq!(operation.reference_json(), saved());
    assert!(connection.native_negotiation().unwrap().is_none());
    assert_eq!(
        operation.status(options()).await.unwrap().state,
        sdk::OperationState::Completed
    );
    let restored = c
        .restore(saved(), c.operation_kinds().connection_end)
        .unwrap();
    let ended = restored.wait(options()).await.unwrap().value;
    assert_eq!(ended.id(), connection.id());
    assert!(ended.native_negotiation().unwrap().is_none());
    close(c, host).await;
}
#[tokio::test]
async fn native_end_caller_cancellation_keeps_owned_completion_and_original_id() {
    let (peer, host, signal) = controlled(Fault::SdkNative(Case::EndCancelled)).await;
    let c = Client::from_authenticated_peer(peer, "tenant".parse().unwrap()).unwrap();
    let session = c
        .conversation_by_id("conv_one".parse().unwrap())
        .session_by_id("session_one".parse().unwrap());
    let connection = ready(&session).await;
    let mutation = ending();
    let cancelled = mutation.request.signal.clone();
    let ending = connection.end(
        200,
        "normal".into(),
        Some(sdk::StreamRevision::try_from(1).unwrap()),
        mutation,
    );
    tokio::pin!(ending);
    tokio::select! { _=&mut ending=>panic!("end must still be owned"),_=signal.notified()=>{} }
    cancelled.cancel();
    let error = ending.await.unwrap_err();
    assert_eq!(error.kind, sdk::ErrorKind::Cancelled);
    assert_eq!(error.certainty, sdk::Certainty::Uncertain);
    assert_eq!(
        error.operation.unwrap().operation_id.as_str(),
        "end-original"
    );
    let restored = c
        .restore(saved(), c.operation_kinds().connection_end)
        .unwrap();
    assert_eq!(
        restored.status(options()).await.unwrap().state,
        sdk::OperationState::Completed
    );
    // status does not decode the end value: the owned dropped-caller exchange retired it.
    assert!(connection.native_negotiation().unwrap().is_none());
    restored.wait(options()).await.unwrap();
    close(c, host).await;
}
#[tokio::test]
async fn native_end_unknown_does_not_invent_termination_or_repeat_effect() {
    let (c, session, host) = client(Case::EndUnknown).await;
    let connection = ready(&session).await;
    let operation = connection
        .end(
            200,
            "normal".into(),
            Some(sdk::StreamRevision::try_from(1).unwrap()),
            ending(),
        )
        .await
        .unwrap();
    assert_eq!(
        operation.status(options()).await.unwrap().state,
        sdk::OperationState::Unknown
    );
    assert!(connection.native_negotiation().unwrap().is_some());
    operation.wait(options()).await.unwrap();
    assert!(connection.native_negotiation().unwrap().is_none());
    close(c, host).await;
}
#[tokio::test]
async fn native_end_refuses_foreign_result_and_preserves_explicit_current_owner_refusal() {
    for case in [
        Case::EndWrongConnection,
        Case::EndWrongRevision,
        Case::EndRefused,
    ] {
        let (c, session, host) = client(case).await;
        let connection = ready(&session).await;
        // Invalid original reason is refused before wire I/O, retaining the logical key.
        let error = connection
            .end(200, "x".repeat(1025), None, ending())
            .await
            .unwrap_err();
        assert_eq!(error.kind, sdk::ErrorKind::Validation);
        assert_eq!(error.certainty, sdk::Certainty::NotSubmitted);
        assert_eq!(
            error.operation.unwrap().operation_id.as_str(),
            "end-original"
        );
        let error = connection
            .end(
                200,
                "normal".into(),
                Some(sdk::StreamRevision::try_from(1).unwrap()),
                ending(),
            )
            .await
            .unwrap_err();
        assert_eq!(
            error.operation.unwrap().operation_id.as_str(),
            "end-original"
        );
        if matches!(case, Case::EndRefused) {
            assert_eq!(error.kind, sdk::ErrorKind::Authentication);
            assert_eq!(error.certainty, sdk::Certainty::Refused);
            assert!(connection.native_negotiation().unwrap().is_some());
        } else {
            assert_eq!(error.kind, sdk::ErrorKind::Protocol);
            assert_eq!(error.certainty, sdk::Certainty::Uncertain);
        }
        close(c, host).await;
    }
}

#[tokio::test]
async fn native_end_keeps_retired_hop_handles_reserved_before_signing_new_confirmation() {
    let (c, session, host) = client(Case::EndHandleReuse).await;
    let connection = ready(&session).await;
    let operation = connection
        .end(
            200,
            "normal".into(),
            Some(sdk::StreamRevision::try_from(1).unwrap()),
            ending(),
        )
        .await
        .unwrap();
    operation.status(options()).await.unwrap();
    operation.wait(options()).await.unwrap();
    assert!(connection.native_negotiation().unwrap().is_none());
    let operation = session
        .offer_connection(
            "new-connection".parse().unwrap(),
            NonEmpty {
                first: OfferedStream {
                    id: "new-audio".parse().unwrap(),
                    kind: Medium::Audio,
                    direction: Direction::Sendonly,
                    codecs: NonEmpty {
                        first: "pcmu".into(),
                        rest: vec![],
                    },
                },
                rest: vec![],
            },
            MutationOptions {
                operation_id: "native-reuse".parse().unwrap(),
                request: options(),
                extensions: None,
            },
        )
        .await
        .unwrap();
    let _ = operation.status(options()).await;
    let Err(error) = operation.wait(options()).await else {
        panic!("retired handle reuse must refuse completion")
    };
    assert_eq!(
        error.operation.unwrap().operation_id.as_str(),
        "native-reuse"
    );
    assert!(connection.native_negotiation().is_err());
    close(c, host).await;
}
