//! Signed server callbacks must prove their original journal before confirmation.
use super::*;
use sdk::{NativeReconciliationPhase as Phase, StreamSelector};
mod journal;
#[derive(Clone, Copy)]
pub(in crate::canonical) enum Case {
    Normal,
    Delay,
    WrongParent,
    WrongRecord,
    WrongExpected,
    WrongOperation,
    NoCapture,
    NoCaptureRetire,
    Journal(journal::Case),
    Unsettled,
    Route,
    RouteEqual,
    RouteEqualNoCapture,
    RouteDelayed,
    RouteBadRecord,
    RouteBadCounter,
    RouteBadBindings,
    RouteStale,
    RouteForeignScope,
    RouteUnknownParent,
    RouteMalformed,
    RouteUnsettled,
    RouteNoCapture,
    ReadRefused,
    ReadTimeout,
    Unsupported,
    Duplicate,
}
fn selection() -> Value {
    let mut ports = selections::streams();
    ports.as_array_mut().unwrap().truncate(3);
    json!({"connection_id":"connection_one","command_type":"stream.subscribe","connection_generation":1,"expected_revision":1,"stream_revision":2,"resources_changed":false,"stream_bindings":selections::bindings(&ports)})
}
fn routing(case: Case) -> bool {
    matches!(
        case,
        Case::Route
            | Case::RouteEqual
            | Case::RouteEqualNoCapture
            | Case::RouteDelayed
            | Case::RouteBadRecord
            | Case::RouteBadCounter
            | Case::RouteBadBindings
            | Case::RouteStale
            | Case::RouteForeignScope
            | Case::RouteUnknownParent
            | Case::RouteMalformed
            | Case::RouteNoCapture
    )
}
fn captured(case: Case) -> Value {
    let mut ports = selections::streams();
    if routing(case) {
        ports.as_array_mut().unwrap().truncate(3);
    }
    let mut out = json!({"connection_id":"connection_one","command_type":"stream.reconcile","connection_generation":1,"expected_revision":2,"stream_revision":3,"resources_changed":!routing(case),"stream_bindings":if matches!(case,Case::NoCapture|Case::NoCaptureRetire|Case::Unsettled|Case::RouteUnsettled|Case::RouteNoCapture|Case::RouteEqualNoCapture) {Value::Null}else{selections::bindings(&ports)},"server_reconciliation":true,"subscription_record_id":"opr_selector"});
    if matches!(case, Case::RouteEqual | Case::RouteEqualNoCapture) {
        out["stream_revision"] = json!(2);
    }
    if matches!(case, Case::RouteBadBindings) {
        out["stream_bindings"][1]["stream_local_id"] = json!(16);
    }
    out
}
async fn hint(writer: &mut quinn::SendStream, signer: &Sig9421Signer, case: Case) {
    let later = matches!(case, Case::RouteUnsettled);
    let mut payload = json!({"operation_id":if later {"route-later"}else{"auto-selection"},"record_id":"opr_auto","command_type":"stream.reconcile","expected_revision":if later {3}else{2},"state":"completed","revision":1,"subscription_record_id":"opr_selector","stream_revision":if later {4}else{3},"journal_hint":true,"resources_changed":false});
    if matches!(case, Case::RouteEqual | Case::RouteEqualNoCapture) {
        payload["stream_revision"] = json!(2);
    }
    if matches!(case, Case::RouteUnknownParent) {
        payload["subscription_record_id"] = json!("opr_not_issued");
    }
    if matches!(case, Case::RouteStale) {
        payload["expected_revision"] = json!(1);
        payload["stream_revision"] = json!(2);
    }
    if matches!(case, Case::RouteMalformed) {
        payload["resources_changed"] = json!(true);
    }
    let mut frame = envelope("operation.updated", payload);
    for (field, value) in [
        ("cid", "conv_one"),
        ("sid", "session_one"),
        ("connid", "connection_one"),
    ] {
        frame[field] = json!(value);
    }
    frame["operation_id"] = frame["payload"]["operation_id"].clone();
    if matches!(case, Case::RouteForeignScope) {
        frame["sid"] = json!("another-session");
    }
    let frame = signer.sign(&frame).unwrap();
    let bytes = serde_json::to_vec(&frame).unwrap();
    writer
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .await
        .unwrap();
    writer.write_all(&bytes).await.unwrap();
}
async fn reply(
    writer: &mut quinn::SendStream,
    signer: &Sig9421Signer,
    request: &Envelope,
    selector: bool,
    state: &str,
    case: Case,
    second: bool,
) {
    let id = request.original()["payload"]["operation_id"]
        .as_str()
        .or_else(|| request.scope(Scope::Operation))
        .unwrap();
    let mut p = json!({"operation_id":id,"record_id":if selector {"opr_selector"}else{"opr_auto"},"command_type":if selector {"stream.subscribe"}else{"stream.reconcile"},"expected_revision":if selector {1}else{2},"state":state,"revision":1});
    if !selector {
        p["subscription_record_id"] = json!(if matches!(case, Case::WrongParent) {
            "opr_unissued"
        } else {
            "opr_selector"
        });
        if matches!(case, Case::WrongRecord) && second {
            p["record_id"] = json!("opr_changed");
        }
        if matches!(case, Case::WrongExpected) {
            p["expected_revision"] = json!(3);
        }
        if matches!(case, Case::WrongOperation) {
            p["operation_id"] = json!("another-logical-operation");
        }
    }
    if !selector && matches!(case, Case::RouteBadRecord) {
        p["record_id"] = json!("opr_changed");
    }
    if !selector && matches!(case, Case::RouteBadCounter) {
        p["expected_revision"] = json!(3);
    }
    if state == "completed" {
        p["result"] = if selector {
            selection()
        } else {
            captured(case)
        };
    }
    write(writer, signer, request, "operation.updated", p, true).await;
    if !(matches!(case, Case::RouteDelayed) && !selector && !second) {
        write(writer, signer, request, "ack", json!({}), true).await;
    }
}
async fn send_callback(
    writer: &mut quinn::SendStream,
    signer: &Sig9421Signer,
    caps: &Value,
    case: Case,
) -> Envelope {
    // Construct once here rather than treating the proposal ID as a callback ID.
    let mut ports = selections::streams();
    if matches!(case, Case::Unsupported) {
        ports[3]["codec"]["name"] = json!("h264");
    }
    let offered=ports.as_array().unwrap().iter().map(|s|json!({"id":s["strm_id"],"kind":s["kind"],"direction":s["direction"],"codec_preferences":[s["codec"]["name"]]})).collect::<Vec<_>>();
    let mut v = envelope(
        "connection.offer",
        json!({"by_participant":"part_one","substrate":"quic","capabilities":caps,"streams_offered":offered,"substrate_setup":{"kind":"native-prepared/1","revision":3,"offer_id":"internal-proposal-distinct-from-callback","streams":ports}}),
    );
    for (field, value) in [
        ("cid", "conv_one"),
        ("sid", "session_one"),
        ("connid", "connection_one"),
        ("operation_id", "auto-selection"),
    ] {
        v[field] = json!(value);
    }
    v["expected_revision"] = json!(2);
    let signed = signer.sign(&v).unwrap();
    let bytes = serde_json::to_vec(&signed).unwrap();
    writer
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .await
        .unwrap();
    writer.write_all(&bytes).await.unwrap();
    Envelope::parse(&bytes).unwrap()
}
async fn resend(writer: &mut quinn::SendStream, signer: &Sig9421Signer, offer: &Envelope) {
    let mut value = offer.original().clone();
    value.as_object_mut().unwrap().remove("signature");
    value["id"] = envelope("connection.offer", json!({}))["id"].clone();
    value["ts"] = json!(chrono::Utc::now());
    let signed = signer.sign(&value).unwrap();
    let bytes = serde_json::to_vec(&signed).unwrap();
    writer
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .await
        .unwrap();
    writer.write_all(&bytes).await.unwrap();
}
pub(super) async fn emit(
    connection: &quinn::Connection,
    case: Case,
    reader: &mut quinn::RecvStream,
    writer: &mut quinn::SendStream,
    signer: &Sig9421Signer,
    signal: &Notify,
    caps: &Value,
) {
    if let Case::Journal(case) = case {
        journal::emit(case, reader, writer, signer, signal).await;
        return;
    }
    let mut offer: Option<Envelope> = None;
    let mut late_query: Option<Envelope> = None;
    let mut authorized = false;
    let mut confirmed = 0;
    let mut queries = 0;
    let mut released = false;
    let mut sent_next = false;
    let mut started = false;
    while let Some(request) = selections::maybe_read(reader).await {
        if matches!(case, Case::NoCaptureRetire) {
            if request.kind() == "connection.offer" {
                fresh_after_retirement(connection, reader, writer, signer, &request).await;
                continue;
            }
            if request.kind() == "connection.end"
                || request.original()["payload"]["operation_id"] == "retire-missing-capture"
            {
                assert_eq!(request.scope(Scope::Connection), Some("connection_one"));
                if request.kind() == "connection.end" {
                    assert_eq!(
                        request.scope(Scope::Operation),
                        Some("retire-missing-capture")
                    );
                    assert!(request.expected_revision().is_none());
                    assert_eq!(
                        request.original()["payload"],
                        json!({"reason_code":200,"reason":"Unavailable capture; establish a fresh Connection"})
                    );
                }
                write(writer, signer, &request, "operation.updated",json!({"operation_id":"retire-missing-capture","state":"completed","revision":1,"result":{"connection_id":"connection_one","ended":true}}),true).await;
                write(writer, signer, &request, "ack", json!({}), true).await;
                continue;
            }
        }
        if request.kind() == "stream.subscribe" {
            assert_eq!(request.scope(Scope::Operation), Some("selector"));
            reply(writer, signer, &request, true, "completed", case, false).await;
            continue;
        }
        if request.kind() == "connection.answer" {
            assert!(authorized, "No signing before current journal read");
            confirmed += 1;
            assert_eq!(confirmed, 1, "Duplicate callback repeated native effect");
            assert_eq!(
                request.scope(Scope::Reply),
                Some(offer.as_ref().unwrap().id())
            );
            assert_eq!(request.scope(Scope::Operation), Some("auto-selection"));
            assert!(request.expected_revision().is_none());
            continue;
        }
        if request.kind() == "error" {
            if matches!(case, Case::Unsettled) {
                assert_eq!(request.scope(Scope::Operation), Some("next-auto-selection"));
                assert_eq!(request.original()["payload"]["code"], 429);
                signal.notify_one();
            } else {
                assert!(matches!(case, Case::Unsupported));
                assert_eq!(request.original()["payload"]["code"], 488);
            }
            continue;
        }
        assert_eq!(request.kind(), "operation.get");
        let id = request.original()["payload"]["operation_id"]
            .as_str()
            .unwrap();
        if id == "selector" {
            if let Some(late) = late_query.take() {
                if matches!(case, Case::RouteDelayed) {
                    write(writer, signer, &late, "ack", json!({}), true).await;
                } else {
                    reply(writer, signer, &late, false, "completed", case, true).await;
                }
            }
            reply(writer, signer, &request, true, "completed", case, false).await;
            if !started && routing(case) {
                hint(writer, signer, case).await;
                started = true;
            }
            if !started && offer.is_none() {
                let callback = send_callback(writer, signer, caps, case).await;
                if matches!(case, Case::Duplicate) {
                    resend(writer, signer, &callback).await;
                }
                offer = Some(callback);
                started = true;
            }
        } else {
            assert_eq!(id, "auto-selection");
            queries += 1;
            if matches!(case, Case::ReadTimeout) {
                late_query = Some(request);
                continue;
            }
            if matches!(case, Case::ReadRefused) {
                write(writer,signer,&request,"error",json!({"code":403,"category":"authorization","reason":"current-authority-refused"}),true).await;
                continue;
            }
            if matches!(case, Case::Delay) && confirmed > 0 && !released {
                tokio::select! {biased;_=signal.notified()=>released=true,_=tokio::time::sleep(Duration::from_millis(1))=>()}
            }
            let completed =
                routing(case) || confirmed > 0 && (!matches!(case, Case::Delay) || released);
            reply(
                writer,
                signer,
                &request,
                false,
                if completed { "completed" } else { "preparing" },
                case,
                queries > 1,
            )
            .await;
            authorized = true;
            if matches!(case, Case::RouteDelayed) && queries == 1 {
                late_query = Some(request.clone());
                signal.notify_one();
            }
            if completed && matches!(case, Case::RouteUnsettled) && !sent_next {
                hint(writer, signer, case).await;
                signal.notify_one();
                sent_next = true;
            }
            if completed && matches!(case, Case::Unsettled) && !sent_next {
                let mut next = offer.as_ref().unwrap().original().clone();
                next.as_object_mut().unwrap().remove("signature");
                next["id"] = envelope("connection.offer", json!({}))["id"].clone();
                next["ts"] = json!(chrono::Utc::now());
                next["operation_id"] = json!("next-auto-selection");
                next["expected_revision"] = json!(3);
                next["payload"]["substrate_setup"]["revision"] = json!(4);
                let signed = signer.sign(&next).unwrap();
                let bytes = serde_json::to_vec(&signed).unwrap();
                writer
                    .write_all(&(bytes.len() as u32).to_be_bytes())
                    .await
                    .unwrap();
                writer.write_all(&bytes).await.unwrap();
                sent_next = true;
            }
            if completed && matches!(case, Case::Duplicate) {
                resend(writer, signer, offer.as_ref().unwrap()).await;
            }
        }
    }
}
async fn fresh_after_retirement(
    connection: &quinn::Connection,
    reader: &mut quinn::RecvStream,
    writer: &mut quinn::SendStream,
    signer: &Sig9421Signer,
    request: &Envelope,
) {
    assert_eq!(request.scope(Scope::Connection), Some("connection_fresh"));
    assert_eq!(request.scope(Scope::Operation), Some("fresh-offer"));
    assert_eq!(request.expected_revision(), Some(0));
    let streams = json!([
        {"strm_id":"audio","kind":"audio","direction":"recvonly","stream_local_id":47,"codec":{"name":"pcmu","params":{"sample_rate":8000,"channels":1,"payload_type":0}}},
        {"strm_id":"video","kind":"video","direction":"sendonly","stream_local_id":48,"codec":{"name":"vp8","params":{"sample_rate":90000,"channels":0,"payload_type":96}}},
        {"strm_id":"data","kind":"data","direction":"sendrecv","stream_local_id":49,"codec":{"name":"json","params":{"reliability":"reliable_ordered"}}}
    ]);
    let setup =
        json!({"kind":"native-prepared/1","revision":1,"offer_id":request.id(),"streams":streams});
    let caps = request.original()["payload"]["capabilities"].clone();
    let answered=streams.as_array().unwrap().iter().map(|s|json!({"id":s["strm_id"],"kind":s["kind"],"direction":s["direction"],"codec":s["codec"]})).collect::<Vec<_>>();
    write(writer,signer,request,"connection.answer",json!({"by_participant":"part_one","substrate":"quic","capabilities":caps,"streams_answered":answered,"substrate_setup":setup}),true).await;
    let query = read(reader).await;
    assert_eq!(query.kind(), "operation.get");
    write(
        writer,
        signer,
        &query,
        "operation.updated",
        json!({"operation_id":"fresh-offer","state":"preparing","revision":1}),
        true,
    )
    .await;
    write(writer, signer, &query, "ack", json!({}), true).await;
    let offered=streams.as_array().unwrap().iter().map(|s|json!({"id":s["strm_id"],"kind":s["kind"],"direction":s["direction"],"codec_preferences":[s["codec"]["name"]]})).collect::<Vec<_>>();
    let callback=write(writer,signer,request,"connection.offer",json!({"by_participant":"part_one","substrate":"quic","capabilities":caps,"streams_offered":offered,"substrate_setup":setup}),false).await;
    loop {
        let next = read(reader).await;
        assert_eq!(next.scope(Scope::Connection), Some("connection_fresh"));
        if next.kind() == "operation.get" {
            write(
                writer,
                signer,
                &next,
                "operation.updated",
                json!({"operation_id":"fresh-offer","state":"preparing","revision":1}),
                true,
            )
            .await;
            write(writer, signer, &next, "ack", json!({}), true).await;
            continue;
        }
        assert_eq!(next.kind(), "connection.answer");
        assert_eq!(next.scope(Scope::Reply), Some(callback.id()));
        assert_eq!(next.scope(Scope::Operation), Some("fresh-offer"));
        assert_eq!(next.original()["payload"]["substrate_setup"], setup);
        break;
    }
    let query = read(reader).await;
    assert_eq!(query.kind(), "operation.get");
    assert_eq!(query.original()["payload"]["operation_id"], "fresh-offer");
    write(writer,signer,&query,"operation.updated",json!({"operation_id":"fresh-offer","state":"completed","revision":1,"result":{"streams":["audio","video","data"],"stream_revision":1}}),true).await;
    write(writer, signer, &query, "ack", json!({}), true).await;
    let bytes = tokio::time::timeout(Duration::from_secs(2), connection.read_datagram())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bytes.as_ref(), datagram(47, 40, &raw_rtp(0)));
}
async fn ready(
    case: Case,
) -> (
    Client,
    sdk::Connection,
    sdk::Operation<sdk::StreamSelectionReceipt>,
    tokio::task::JoinHandle<()>,
    Arc<Notify>,
) {
    let (peer, host, signal) =
        controlled(Fault::SdkNative(super::Case::Reconciliation(case))).await;
    let client = Client::from_authenticated_peer(peer, "tenant".parse().unwrap()).unwrap();
    let session = client
        .conversation_by_id("conv_one".parse().unwrap())
        .session_by_id("session_one".parse().unwrap());
    let c = session
        .offer_connection("connection_one".parse().unwrap(), offered(), mutation())
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap()
        .value;
    let selector = c
        .subscribe(
            NonEmpty {
                first: StreamSelector::Participant {
                    participant_id: "part_remote".parse().unwrap(),
                    kinds: None,
                },
                rest: vec![],
            },
            sdk::StreamRevision::try_from(1).unwrap(),
            MutationOptions {
                operation_id: "selector".parse().unwrap(),
                request: options(),
                extensions: None,
            },
        )
        .await
        .unwrap();
    selector.wait(options()).await.unwrap();
    (client, c, selector, host, signal)
}
async fn phase(c: &sdk::Connection, wanted: Phase) -> sdk::NativeReconciliation {
    let until = tokio::time::Instant::now() + Duration::from_secs(4);
    loop {
        if let Some(s) = c.native_reconciliation().unwrap() {
            if s.phase == wanted {
                return s;
            }
        }
        assert!(
            tokio::time::Instant::now() < until,
            "wanted {wanted:?}, observed {:?}",
            c.native_reconciliation().unwrap()
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}
fn count(c: &sdk::Connection) -> usize {
    c.native_negotiation().unwrap().unwrap().streams.len()
}
#[tokio::test]
async fn automatic_reconciliation_reads_original_journal_before_signing_then_adopts_only_captured_completion(
) {
    let (client, c, _, host, _) = ready(Case::Normal).await;
    let s = phase(&c, Phase::Recorded).await;
    assert!(s.bindings_adopted);
    assert_eq!(s.record_id.unwrap().as_str(), "opr_auto");
    assert_eq!(s.subscription_record_id.unwrap().as_str(), "opr_selector");
    assert_eq!(count(&c), 5);
    assert_eq!(
        c.native_negotiation()
            .unwrap()
            .unwrap()
            .stream_revision
            .get(),
        3
    );
    close(client, host).await;
}
#[tokio::test]
async fn delayed_completion_preserves_local_bindings_and_control_while_another_mutation_refuses_before_submission(
) {
    let (client, c, selector, host, signal) = ready(Case::Delay).await;
    phase(&c, Phase::AwaitingCompletion).await;
    assert_eq!(count(&c), 3);
    selector.wait(options()).await.unwrap();
    let error = c
        .unsubscribe(
            NonEmpty {
                first: "video".parse().unwrap(),
                rest: vec![],
            },
            sdk::StreamRevision::try_from(2).unwrap(),
            MutationOptions {
                operation_id: "competing".parse().unwrap(),
                request: options(),
                extensions: None,
            },
        )
        .await
        .err()
        .unwrap();
    assert_eq!(error.kind, sdk::ErrorKind::Conflict);
    assert_eq!(error.certainty, sdk::Certainty::NotSubmitted);
    signal.notify_one();
    let s = phase(&c, Phase::Recorded).await;
    assert!(s.bindings_adopted);
    s.operation(&client).wait(options()).await.unwrap();
    assert_eq!(count(&c), 5);
    close(client, host).await;
}
#[tokio::test]
async fn wrong_journal_parent_counter_logical_or_changed_host_identity_never_adopts_and_preserves_control(
) {
    for case in [
        Case::WrongParent,
        Case::WrongExpected,
        Case::WrongOperation,
        Case::WrongRecord,
    ] {
        let (client, c, selector, host, _) = ready(case).await;
        let s = phase(&c, Phase::Unresolved).await;
        assert_eq!(s.error, Some(sdk::ErrorKind::Protocol));
        assert_eq!(count(&c), 3);
        selector.wait(options()).await.unwrap();
        close(client, host).await;
    }
}
#[tokio::test]
async fn refused_current_journal_read_keeps_original_native_media_and_other_operation_reads_usable()
{
    let (client, c, selector, host, _) = ready(Case::ReadRefused).await;
    phase(&c, Phase::Unresolved).await;
    assert_eq!(count(&c), 3);
    selector.wait(options()).await.unwrap();
    close(client, host).await;
}
#[tokio::test]
async fn unsupported_receive_format_is_declined_without_retiring_the_healthy_owned_peer() {
    let (client, c, selector, host, _) = ready(Case::Unsupported).await;
    phase(&c, Phase::Refused).await;
    assert_eq!(count(&c), 3);
    selector.wait(options()).await.unwrap();
    close(client, host).await;
}
#[tokio::test]
async fn unavailable_completion_capture_is_recorded_without_inventing_empty_bindings_or_adopting_preparation(
) {
    let (client, c, _, host, _) = ready(Case::NoCapture).await;
    let s = phase(&c, Phase::Recorded).await;
    assert!(!s.bindings_adopted);
    let receipt = s.operation(&client).wait(options()).await.unwrap().value;
    assert!(receipt.bindings.is_none());
    assert_eq!(count(&c), 3);
    close(client, host).await;
}
#[tokio::test]
async fn unavailable_resource_capture_can_end_original_connection_and_negotiate_fresh_handles_without_adopting_it(
) {
    let (client, c, _, host, _) = ready(Case::NoCaptureRetire).await;
    let original = phase(&c, Phase::Recorded).await;
    assert!(!original.bindings_adopted);
    let receipt = original
        .operation(&client)
        .wait(options())
        .await
        .unwrap()
        .value;
    assert!(receipt.bindings.is_none());
    let original_ref = original.operation(&client).reference_json();
    let mut retire = mutation();
    retire.operation_id = "retire-missing-capture".parse().unwrap();
    let ended = c
        .end(
            200,
            "Unavailable capture; establish a fresh Connection".into(),
            None,
            retire,
        )
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap()
        .value;
    assert_eq!(ended.id(), c.id());
    assert!(ended.native_negotiation().unwrap().is_none());
    assert!(c
        .stream_by_id("audio".parse().unwrap())
        .send_rtp(sdk::RtpPacket::new(40, raw_rtp(0)).unwrap(), options())
        .await
        .is_err());
    let mut fresh = mutation();
    fresh.operation_id = "fresh-offer".parse().unwrap();
    let session = client
        .conversation_by_id("conv_one".parse().unwrap())
        .session_by_id("session_one".parse().unwrap());
    let fresh = session
        .offer_connection("connection_fresh".parse().unwrap(), offered(), fresh)
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap()
        .value;
    assert_eq!(
        fresh
            .native_negotiation()
            .unwrap()
            .unwrap()
            .streams
            .iter()
            .map(|s| s.stream_local_id)
            .collect::<Vec<_>>(),
        vec![Some(47), Some(48), None]
    );
    fresh
        .stream_by_id("audio".parse().unwrap())
        .send_rtp(sdk::RtpPacket::new(40, raw_rtp(0)).unwrap(), options())
        .await
        .unwrap();
    // Original receipt restoration after teardown stays historical and cannot
    // reacquire old native custody or fill in the unavailable capture.
    let restored = client
        .restore(original_ref, client.operation_kinds().stream_reconcile)
        .unwrap()
        .wait(options())
        .await
        .unwrap()
        .value;
    assert!(restored.bindings.is_none());
    assert!(c.native_negotiation().unwrap().is_none());
    assert_eq!(count(&fresh), 3);
    close(client, host).await;
}
#[tokio::test]
async fn duplicate_server_callback_never_reconfirms_or_reapplies_the_known_original_effect() {
    let (client, c, selector, host, _) = ready(Case::Duplicate).await;
    let s = phase(&c, Phase::Recorded).await;
    selector.wait(options()).await.unwrap();
    s.operation(&client).wait(options()).await.unwrap();
    assert!(c.native_reconciliation().unwrap().unwrap().bindings_adopted);
    assert_eq!(count(&c), 5);
    close(client, host).await;
}

#[tokio::test]
async fn internal_query_timeout_and_late_reply_preserve_peer_control_without_adopting_or_repeating_media(
) {
    let (client, c, selector, host, _) = ready(Case::ReadTimeout).await;
    let s = phase(&c, Phase::Unresolved).await;
    assert_eq!(s.error, Some(sdk::ErrorKind::Deadline));
    selector.wait(options()).await.unwrap();
    selector.wait(options()).await.unwrap();
    assert_eq!(count(&c), 3);
    close(client, host).await;
}

#[tokio::test]
async fn another_server_callback_cannot_replace_recovery_identity_for_an_unsettled_original_preparation(
) {
    let (client, c, _, host, signal) = ready(Case::Unsettled).await;
    phase(&c, Phase::Recorded).await;
    tokio::time::timeout(Duration::from_secs(1), signal.notified())
        .await
        .unwrap();
    let original = c.native_reconciliation().unwrap().unwrap();
    assert_eq!(original.key.operation_id.as_str(), "auto-selection");
    assert_eq!(original.record_id.as_ref().unwrap().as_str(), "opr_auto");
    assert!(!original.bindings_adopted);
    original.operation(&client).wait(options()).await.unwrap();
    assert_eq!(count(&c), 3);
    close(client, host).await;
}

#[tokio::test]
async fn routing_hint_reads_current_journal_before_advancing_owned_counter_without_any_parameter_answer(
) {
    let (client, c, selector, host, _) = ready(Case::Route).await;
    let s = phase(&c, Phase::Recorded).await;
    assert!(s.bindings_adopted);
    assert_eq!(count(&c), 3);
    assert_eq!(
        c.native_negotiation()
            .unwrap()
            .unwrap()
            .stream_revision
            .get(),
        3
    );
    selector.wait(options()).await.unwrap();
    assert_eq!(
        c.native_negotiation()
            .unwrap()
            .unwrap()
            .stream_revision
            .get(),
        3
    );
    close(client, host).await;
}
#[tokio::test]
async fn routing_hint_and_incomplete_current_query_cannot_advance_counter_until_correlated_ack() {
    let (client, c, selector, host, signal) = ready(Case::RouteDelayed).await;
    tokio::time::timeout(Duration::from_secs(1), signal.notified())
        .await
        .unwrap();
    assert_eq!(
        c.native_negotiation()
            .unwrap()
            .unwrap()
            .stream_revision
            .get(),
        2
    );
    selector.wait(options()).await.unwrap();
    phase(&c, Phase::Recorded).await;
    assert_eq!(
        c.native_negotiation()
            .unwrap()
            .unwrap()
            .stream_revision
            .get(),
        3
    );
    close(client, host).await;
}
#[tokio::test]
async fn routing_hint_mismatched_record_or_counter_refuses_and_unmatched_bindings_never_install() {
    for case in [
        Case::RouteBadRecord,
        Case::RouteBadCounter,
        Case::RouteBadBindings,
    ] {
        let (client, c, selector, host, _) = ready(case).await;
        let wanted = if matches!(case, Case::RouteBadBindings) {
            Phase::Recorded
        } else {
            Phase::Unresolved
        };
        let status = phase(&c, wanted).await;
        assert!(!status.bindings_adopted);
        assert_eq!(
            c.native_negotiation()
                .unwrap()
                .unwrap()
                .stream_revision
                .get(),
            2
        );
        assert_eq!(count(&c), 3);
        selector.wait(options()).await.unwrap();
        close(client, host).await;
    }
}
#[tokio::test]
async fn stale_foreign_unissued_or_nonrouting_hints_do_not_acquire_custody_or_disturb_control() {
    for case in [
        Case::RouteStale,
        Case::RouteForeignScope,
        Case::RouteUnknownParent,
        Case::RouteMalformed,
    ] {
        let (client, c, selector, host, _) = ready(case).await;
        selector.wait(options()).await.unwrap();
        assert!(c.native_reconciliation().unwrap().is_none());
        assert_eq!(
            c.native_negotiation()
                .unwrap()
                .unwrap()
                .stream_revision
                .get(),
            2
        );
        close(client, host).await;
    }
}
#[tokio::test]
async fn pending_routing_hint_cannot_replace_identity_or_settle_an_unavailable_resource_capture() {
    let (client, c, selector, host, signal) = ready(Case::RouteUnsettled).await;
    tokio::time::timeout(Duration::from_secs(1), signal.notified())
        .await
        .unwrap();
    selector.wait(options()).await.unwrap();
    let s = phase(&c, Phase::Recorded).await;
    assert_eq!(s.key.operation_id.as_str(), "auto-selection");
    assert!(!s.bindings_adopted);
    assert_eq!(
        c.native_negotiation()
            .unwrap()
            .unwrap()
            .stream_revision
            .get(),
        2
    );
    close(client, host).await;
}
#[tokio::test]
async fn recorded_routing_only_null_capture_advances_exact_predecessor_counter_without_inventing_bindings(
) {
    let (client, c, _, host, _) = ready(Case::RouteNoCapture).await;
    let status = phase(&c, Phase::Recorded).await;
    assert!(!status.bindings_adopted);
    assert_eq!(count(&c), 3);
    assert_eq!(
        c.native_negotiation()
            .unwrap()
            .unwrap()
            .stream_revision
            .get(),
        3
    );
    let receipt = status
        .operation(&client)
        .wait(options())
        .await
        .unwrap()
        .value;
    assert!(receipt.bindings.is_none());
    assert!(!receipt.resources_changed);
    close(client, host).await;
}

#[tokio::test]
async fn routing_hint_observes_new_journal_identity_when_native_ports_and_revision_are_unchanged() {
    for case in [Case::RouteEqual, Case::RouteEqualNoCapture] {
        let (client, c, selector, host, _) = ready(case).await;
        let status = phase(&c, Phase::Recorded).await;
        assert_eq!(status.key.operation_id.as_str(), "auto-selection");
        assert_eq!(status.bindings_adopted, matches!(case, Case::RouteEqual));
        assert_eq!(count(&c), 3);
        assert_eq!(
            c.native_negotiation()
                .unwrap()
                .unwrap()
                .stream_revision
                .get(),
            2
        );
        let result = status
            .operation(&client)
            .wait(options())
            .await
            .unwrap()
            .value;
        assert!(!result.resources_changed);
        assert_eq!(result.stream_revision.get(), 2);
        selector.wait(options()).await.unwrap();
        close(client, host).await;
    }
}
