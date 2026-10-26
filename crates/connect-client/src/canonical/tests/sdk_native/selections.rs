//! Signed selection preparation/result pairing, route-only revisions and recovery.
use super::*;
use crate::sdk::StreamSelector;
#[derive(Clone, Copy, Debug)]
pub(in crate::canonical) enum Case {
    Normal,
    Recycle,
    Unknown,
    Dropped,
    RoutingOnly,
    NoCapture,
    Failed,
    BadRevision,
    BadDirection,
    BadCodec,
    BadCaps,
    BadParticipant,
    Collision,
    DataZero,
    RemovedPublisher,
    ChangedData,
    BadResultHandle,
    BadResultGeneration,
    BadResultKind,
    BadResultSource,
    BadResultExpected,
    BadPayloadOperation,
}
fn revision(n: u64) -> sdk::StreamRevision {
    sdk::StreamRevision::try_from(n).unwrap()
}
fn opts(id: &str) -> MutationOptions {
    MutationOptions {
        operation_id: id.parse().unwrap(),
        request: options(),
        extensions: None,
    }
}
fn selectors() -> NonEmpty<StreamSelector> {
    NonEmpty {
        first: StreamSelector::Stream("camera".parse().unwrap()),
        rest: vec![StreamSelector::Stream("remote-data".parse().unwrap())],
    }
}
pub(super) fn streams() -> Value {
    json!([
     {"strm_id":"audio","kind":"audio","direction":"recvonly","stream_local_id":7,"codec":{"name":"pcmu","params":{"sample_rate":8000,"channels":1,"payload_type":0}}},
     {"strm_id":"video","kind":"video","direction":"sendonly","stream_local_id":8,"codec":{"name":"vp8","params":{"sample_rate":90000,"channels":0,"payload_type":96}}},
     {"strm_id":"data","kind":"data","direction":"sendrecv","stream_local_id":9,"codec":{"name":"json","params":{"reliability":"reliable_ordered"}}},
     {"strm_id":"camera","kind":"video","direction":"sendonly","stream_local_id":17,"codec":{"name":"vp8","params":{"sample_rate":90000,"channels":0,"payload_type":96}}},
     {"strm_id":"remote-data","kind":"data","direction":"sendonly","stream_local_id":18,"codec":{"name":"json","params":{"reliability":"reliable_ordered"}}}
    ])
}
pub(super) fn bindings(v: &Value) -> Value {
    json!(v.as_array().unwrap().iter().map(|s|json!({"stream_id":s["strm_id"],"medium":s["kind"],"direction":s["direction"],"codec":s["codec"],"stream_local_id":s["stream_local_id"],"stream_generation":1,"connection_generation":1})).collect::<Vec<_>>())
}
fn ports(request: &Envelope, case: Case) -> Value {
    let mut v = streams();
    if matches!(case, Case::Recycle) {
        match request.original()["expected_revision"].as_u64().unwrap() {
            2 => v.as_array_mut().unwrap().truncate(3),
            3 => {
                v[3]["stream_local_id"] = json!(19);
                v[4]["stream_local_id"] = json!(20);
            }
            _ => (),
        }
    }
    v
}
fn result(request: &Envelope, case: Case) -> Value {
    let expected = request.original()["expected_revision"].as_u64().unwrap();
    let routing = matches!(case, Case::RoutingOnly);
    let unchanged = routing && request.scope(Scope::Operation) == Some("selection-noop");
    let mut ports = ports(request, case);
    if routing {
        ports.as_array_mut().unwrap().truncate(3);
    }
    let mut out = json!({"connection_id":"connection_one","command_type":request.kind(),"connection_generation":1,"expected_revision":expected,"stream_revision":expected+u64::from(!unchanged),"resources_changed":!routing,"stream_bindings":if matches!(case,Case::NoCapture){Value::Null}else{bindings(&ports)}});
    if matches!(case, Case::Recycle) && expected == 3 {
        out["stream_bindings"][3]["stream_generation"] = json!(2);
        out["stream_bindings"][4]["stream_generation"] = json!(2);
    }
    match case {
        Case::BadResultHandle => out["stream_bindings"][3]["stream_local_id"] = json!(19),
        Case::BadResultGeneration => out["stream_bindings"][3]["stream_generation"] = json!(2),
        Case::BadResultKind => out["command_type"] = json!("stream.unsubscribe"),
        Case::BadResultSource => out["connection_id"] = json!("foreign"),
        Case::BadResultExpected => {
            out["expected_revision"] = json!(3);
            out["stream_revision"] = json!(4);
        }
        _ => (),
    };
    out
}
pub(super) async fn maybe_read(reader: &mut quinn::RecvStream) -> Option<Envelope> {
    let mut prefix = [0; 4];
    tokio::time::timeout(Duration::from_secs(3), reader.read_exact(&mut prefix))
        .await
        .ok()?
        .ok()?;
    let n = u32::from_be_bytes(prefix) as usize;
    assert!(n > 0 && n <= MAX_ENVELOPE_BYTES);
    let mut b = vec![0; n];
    reader.read_exact(&mut b).await.ok()?;
    Some(Envelope::parse(&b).unwrap())
}
async fn respond(
    writer: &mut quinn::SendStream,
    signer: &Sig9421Signer,
    request: &Envelope,
    state: &str,
    result: Option<Value>,
) {
    let id = request
        .scope(Scope::Operation)
        .map(str::to_owned)
        .unwrap_or_else(|| {
            request.original()["payload"]["operation_id"]
                .as_str()
                .unwrap()
                .into()
        });
    let mut p = json!({"operation_id":id,"state":state,"revision":1});
    if let Some(v) = result {
        p["result"] = v;
    }
    if state == "failed" {
        p["problem"] =
            json!({"code":409,"category":"state","reason":"controlled failed selection"});
    }
    write(writer, signer, request, "operation.updated", p, true).await;
    write(writer, signer, request, "ack", json!({}), true).await;
}
fn malformed(case: Case) -> bool {
    matches!(
        case,
        Case::BadRevision
            | Case::BadDirection
            | Case::BadCodec
            | Case::BadCaps
            | Case::BadParticipant
            | Case::Collision
            | Case::DataZero
            | Case::RemovedPublisher
            | Case::ChangedData
    )
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
    let mut root: Option<Envelope> = None;
    let mut captured = Value::Null;
    let mut queries = 0;
    let mut requests = 0;
    let mut history = std::collections::BTreeMap::new();
    while let Some(request) = maybe_read(reader).await {
        if request.kind() == "operation.get" {
            if request.original()["payload"]["operation_id"] == "native-original" {
                if matches!(case, Case::Dropped) && root.is_some() {
                    respond(
                        writer,
                        signer,
                        root.as_ref().unwrap(),
                        "completed",
                        Some(captured.clone()),
                    )
                    .await;
                }
                status(writer, signer, &request, "completed").await;
                continue;
            }
            queries += 1;
            let state = if matches!(case, Case::Unknown) && queries == 1 {
                "unknown"
            } else if matches!(case, Case::Failed) {
                "failed"
            } else {
                "completed"
            };
            respond(
                writer,
                signer,
                &request,
                state,
                (state == "completed").then(|| {
                    history
                        .get(
                            request.original()["payload"]["operation_id"]
                                .as_str()
                                .unwrap(),
                        )
                        .cloned()
                        .unwrap_or_else(|| captured.clone())
                }),
            )
            .await;
            if matches!(case, Case::Normal) {
                connection
                    .send_datagram(datagram(17, 44, &raw_rtp(96)).into())
                    .unwrap();
            }
            continue;
        }
        assert!(matches!(
            request.kind(),
            "stream.subscribe" | "stream.unsubscribe"
        ));
        requests += 1;
        if !matches!(case, Case::RoutingOnly | Case::Recycle) {
            assert_eq!(requests, 1, "recovery never repeats mutation");
        }
        if request.original()["payload"].get("opaque").is_some() {
            assert_eq!(
                request.original()["payload"]["opaque"],
                json!({"explicit":null,"ordered":["b","a"]})
            );
            assert!(request.original()["payload"]["subscriptions"][0]
                .get("kinds")
                .is_none());
        }
        captured = result(&request, case);
        history.insert(
            request.scope(Scope::Operation).unwrap().to_owned(),
            captured.clone(),
        );
        if !matches!(case, Case::RoutingOnly) {
            let mut ports = ports(&request, case);
            match case {
                Case::BadDirection => ports[3]["direction"] = json!("recvonly"),
                Case::BadCodec => ports[3]["codec"]["name"] = json!("h264"),
                Case::Collision => ports[3]["stream_local_id"] = json!(7),
                Case::DataZero => ports[4]["stream_local_id"] = json!(0),
                Case::ChangedData => ports[2]["stream_local_id"] = json!(10),
                Case::RemovedPublisher => {
                    ports.as_array_mut().unwrap().remove(0);
                }
                _ => (),
            };
            let setup = json!({"kind":"native-prepared/1","revision":if matches!(case,Case::BadRevision){99}else{request.original()["expected_revision"].as_u64().unwrap()+1},"offer_id":request.id(),"streams":ports});
            let offered=ports.as_array().unwrap().iter().map(|s|json!({"id":s["strm_id"],"kind":s["kind"],"direction":s["direction"],"codec_preferences":[s["codec"]["name"]]})).collect::<Vec<_>>();
            let callback=write(writer,signer,&request,"connection.offer",json!({"by_participant":if matches!(case,Case::BadParticipant){"foreign"}else{"part_one"},"substrate":"quic","capabilities":if matches!(case,Case::BadCaps){json!({})}else{caps.clone()},"streams_offered":offered,"substrate_setup":setup}),false).await;
            if malformed(case)
                || matches!(case, Case::Recycle) && request.original()["expected_revision"] == 4
            {
                while let Some(reply) = maybe_read(reader).await {
                    assert_ne!(
                        reply.kind(),
                        "connection.answer",
                        "malformed selection must never be signed"
                    );
                }
                return;
            }
            loop {
                let reply = maybe_read(reader).await.unwrap();
                if reply.kind() == "operation.get" {
                    respond(writer, signer, &reply, "preparing", None).await;
                    continue;
                }
                assert_eq!(reply.kind(), "connection.answer");
                assert_eq!(reply.scope(Scope::Reply), Some(callback.id()));
                assert_eq!(reply.original()["payload"]["substrate_setup"], setup);
                if ports.as_array().unwrap().len() > 3 {
                    assert_eq!(
                        reply.original()["payload"]["streams_answered"][3]["direction"],
                        "recvonly"
                    );
                }
                break;
            }
        }
        if matches!(case, Case::BadPayloadOperation) {
            write(writer,signer,&request,"operation.updated",json!({"operation_id":"foreign-operation","state":"completed","revision":1,"result":captured}),true).await;
            while maybe_read(reader).await.is_some() {}
            return;
        }
        if matches!(case, Case::Dropped) {
            signal.notify_one();
            root = Some(request);
            continue;
        }
        if matches!(case, Case::Unknown) {
            respond(writer, signer, &request, "unknown", None).await;
        } else if matches!(case, Case::Failed) {
            respond(writer, signer, &request, "failed", None).await;
        } else {
            respond(
                writer,
                signer,
                &request,
                "completed",
                Some(captured.clone()),
            )
            .await;
        }
    }
}
async fn ready(
    case: Case,
) -> (
    Client,
    sdk::Connection,
    tokio::task::JoinHandle<()>,
    Arc<Notify>,
) {
    let (peer, host, signal) = controlled(Fault::SdkNative(super::Case::Selection(case))).await;
    let client = Client::from_authenticated_peer(peer, "tenant".parse().unwrap()).unwrap();
    let session = client
        .conversation_by_id("conv_one".parse().unwrap())
        .session_by_id("session_one".parse().unwrap());
    let op = session
        .offer_connection("connection_one".parse().unwrap(), offered(), mutation())
        .await
        .unwrap();
    let conn = op.wait(options()).await.unwrap().value;
    (client, conn, host, signal)
}
fn count(c: &sdk::Connection) -> usize {
    c.native_negotiation().unwrap().unwrap().streams.len()
}
#[tokio::test]
async fn signed_selection_adds_receive_only_video_and_data_keeps_publication_and_receipt_binding_identity(
) {
    let (client, c, host, _) = ready(Case::Normal).await;
    let op = c
        .subscribe(selectors(), revision(1), opts("selection"))
        .await
        .unwrap();
    let receipt = op.wait(options()).await.unwrap().value;
    assert_eq!(count(&c), 5);
    assert_eq!(receipt.stream_revision.get(), 2);
    let data = receipt
        .bindings
        .unwrap()
        .into_iter()
        .find(|b| b.parameters.id.as_str() == "remote-data")
        .unwrap();
    assert_eq!(data.parameters.stream_local_id, None);
    assert_eq!(data.parameters.direction, Direction::Recvonly);
    assert!(matches!(
        c.stream_by_id("camera".parse().unwrap())
            .receive_packet(options())
            .await
            .unwrap(),
        sdk::MediaPacket::Rtp(_)
    ));
    assert_eq!(
        c.native_negotiation().unwrap().unwrap().streams[0].stream_local_id,
        Some(7)
    );
    close(client, host).await;
}
#[tokio::test]
async fn unknown_selection_keeps_original_binding_until_same_operation_recovers_without_reapplication(
) {
    let (client, c, host, _) = ready(Case::Unknown).await;
    let op = c
        .subscribe(selectors(), revision(1), opts("selection"))
        .await
        .unwrap();
    assert_eq!(count(&c), 3);
    let restored = client
        .restore(
            op.reference_json(),
            client.operation_kinds().stream_subscribe,
        )
        .unwrap();
    assert_eq!(
        restored.status(options()).await.unwrap().state,
        sdk::OperationState::Unknown
    );
    assert_eq!(count(&c), 3);
    restored.wait(options()).await.unwrap();
    assert_eq!(count(&c), 5);
    close(client, host).await;
}
#[tokio::test]
async fn dropped_selection_caller_retains_owned_confirmation_and_recovers_original_logical_id() {
    let (client, c, host, signal) = ready(Case::Dropped).await;
    let conn = c.clone();
    let task = tokio::spawn(async move {
        conn.subscribe(selectors(), revision(1), opts("selection"))
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), signal.notified())
        .await
        .unwrap();
    task.abort();
    let _ = task.await;
    assert_eq!(count(&c), 3);
    let saved = json!({"schema":"connect-operation-reference/1","operation_id":"native-original","command":"connection.offer","scope":{"conversation_id":"conv_one","session_id":"session_one","connection_id":"connection_one"}});
    client
        .restore(saved, client.operation_kinds().connection_offer)
        .unwrap()
        .status(options())
        .await
        .unwrap();
    let saved = json!({"schema":"connect-operation-reference/1","operation_id":"selection","command":"stream.subscribe","scope":{"conversation_id":"conv_one","session_id":"session_one","connection_id":"connection_one"}});
    client
        .restore(saved, client.operation_kinds().stream_subscribe)
        .unwrap()
        .wait(options())
        .await
        .unwrap();
    assert_eq!(count(&c), 5);
    close(client, host).await;
}
#[tokio::test]
async fn routing_only_selection_unsubscribe_noop_and_old_receipt_never_rewind_local_native_revision(
) {
    let (client, c, host, _) = ready(Case::RoutingOnly).await;
    let first = c
        .subscribe(selectors(), revision(1), opts("selection"))
        .await
        .unwrap();
    assert!(!first.wait(options()).await.unwrap().value.resources_changed);
    let op = c
        .unsubscribe(
            NonEmpty {
                first: "camera".parse().unwrap(),
                rest: vec![],
            },
            revision(2),
            opts("unselect"),
        )
        .await
        .unwrap();
    assert_eq!(
        op.wait(options())
            .await
            .unwrap()
            .value
            .stream_revision
            .get(),
        3
    );
    let op = c
        .unsubscribe(
            NonEmpty {
                first: "camera".parse().unwrap(),
                rest: vec![],
            },
            revision(3),
            opts("selection-noop"),
        )
        .await
        .unwrap();
    assert_eq!(
        op.wait(options())
            .await
            .unwrap()
            .value
            .stream_revision
            .get(),
        3
    );
    assert_eq!(
        c.native_negotiation()
            .unwrap()
            .unwrap()
            .stream_revision
            .get(),
        3
    );
    assert_eq!(
        first
            .wait(options())
            .await
            .unwrap()
            .value
            .stream_revision
            .get(),
        2
    );
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
async fn malformed_selection_reoffers_refuse_before_signing_or_changing_local_parameters() {
    for case in [
        Case::BadRevision,
        Case::BadDirection,
        Case::BadCodec,
        Case::BadCaps,
        Case::BadParticipant,
        Case::Collision,
        Case::DataZero,
        Case::RemovedPublisher,
        Case::ChangedData,
    ] {
        let (client, c, host, _) = ready(case).await;
        let error = c
            .subscribe(selectors(), revision(1), opts("selection"))
            .await
            .err()
            .expect("malformed reoffer must fail");
        assert_eq!(error.kind, sdk::ErrorKind::Protocol, "{case:?}");
        close(client, host).await;
    }
}
#[tokio::test]
async fn malformed_selection_captures_cannot_adopt_unmatched_native_preparation() {
    for case in [
        Case::BadResultHandle,
        Case::BadResultGeneration,
        Case::BadResultKind,
        Case::BadResultSource,
        Case::BadResultExpected,
        Case::BadPayloadOperation,
    ] {
        let (client, c, host, _) = ready(case).await;
        let error = c
            .subscribe(selectors(), revision(1), opts("selection"))
            .await
            .err()
            .expect("unmatched capture must fail");
        assert_eq!(error.kind, sdk::ErrorKind::Protocol, "{case:?}");
        close(client, host).await;
    }
}
#[tokio::test]
async fn unavailable_capture_stays_distinct_from_empty_and_never_installs_or_reuses_prepared_ports()
{
    let (client, c, host, _) = ready(Case::NoCapture).await;
    let op = c
        .subscribe(selectors(), revision(1), opts("selection"))
        .await
        .unwrap();
    assert!(op.wait(options()).await.unwrap().value.bindings.is_none());
    assert_eq!(count(&c), 3);
    let error = c
        .subscribe(selectors(), revision(2), opts("another-selection"))
        .await
        .err()
        .expect("unsettled adoption remains reserved");
    assert_eq!(error.kind, sdk::ErrorKind::Conflict);
    assert_eq!(error.certainty, sdk::Certainty::NotSubmitted);
    close(client, host).await;
}
#[tokio::test]
async fn failed_selection_reports_recorded_outcome_and_keeps_original_negotiation() {
    let (client, c, host, _) = ready(Case::Failed).await;
    let op = c
        .subscribe(selectors(), revision(1), opts("selection"))
        .await
        .unwrap();
    let error = op.wait(options()).await.err().expect("recorded failure");
    assert_eq!(error.certainty, sdk::Certainty::Recorded);
    assert_eq!(count(&c), 3);
    close(client, host).await;
}

#[tokio::test]
async fn selection_selectors_extensions_and_cancelled_preflight_preserve_exact_bounded_intent() {
    let (client, c, host, _) = ready(Case::RoutingOnly).await;
    let duplicate = NonEmpty {
        first: StreamSelector::Stream("camera".parse().unwrap()),
        rest: vec![StreamSelector::Stream("camera".parse().unwrap())],
    };
    let error = c
        .subscribe(duplicate, revision(1), opts("duplicate"))
        .await
        .err()
        .expect("duplicate selector");
    assert_eq!(error.kind, sdk::ErrorKind::Validation);
    assert_eq!(error.certainty, sdk::Certainty::NotSubmitted);
    let mut cancelled = opts("cancelled");
    cancelled.request.signal.cancel();
    assert_eq!(
        c.subscribe(selectors(), revision(1), cancelled)
            .await
            .err()
            .unwrap()
            .certainty,
        sdk::Certainty::NotSubmitted
    );
    let mut options = opts("opaque-selection");
    options.extensions = Some(
        serde_json::from_value(json!({"opaque":{"explicit":null,"ordered":["b","a"]}})).unwrap(),
    );
    let selectors = NonEmpty {
        first: StreamSelector::Participant {
            participant_id: "part_remote".parse().unwrap(),
            kinds: None,
        },
        rest: vec![StreamSelector::Stream("camera".parse().unwrap())],
    };
    c.subscribe(selectors, revision(1), options)
        .await
        .unwrap()
        .wait(super::options())
        .await
        .unwrap();
    close(client, host).await;
}

#[tokio::test]
async fn reintroduced_receive_ids_keep_generation_history_and_cannot_reuse_retired_hop_handles() {
    let (client, c, host, _) = ready(Case::Recycle).await;
    let original = c
        .subscribe(selectors(), revision(1), opts("selection-first"))
        .await
        .unwrap();
    original.wait(options()).await.unwrap();
    assert_eq!(count(&c), 5);
    c.subscribe(
        NonEmpty {
            first: StreamSelector::Participant {
                participant_id: "part_remote".parse().unwrap(),
                kinds: Some(NonEmpty {
                    first: Medium::Audio,
                    rest: vec![],
                }),
            },
            rest: vec![],
        },
        revision(2),
        opts("selection-remove"),
    )
    .await
    .unwrap()
    .wait(options())
    .await
    .unwrap();
    assert_eq!(count(&c), 3);
    let value = c
        .subscribe(selectors(), revision(3), opts("selection-reintroduce"))
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap()
        .value;
    let bindings = value.bindings.unwrap();
    let video = bindings
        .iter()
        .find(|b| b.parameters.id.as_str() == "camera")
        .unwrap();
    assert_eq!(video.stream_generation, 2);
    assert_eq!(video.parameters.stream_local_id, Some(19));
    let data = bindings
        .iter()
        .find(|b| b.parameters.id.as_str() == "remote-data")
        .unwrap();
    assert_eq!(data.stream_generation, 2);
    assert!(data.parameters.stream_local_id.is_none());
    original.wait(options()).await.unwrap();
    assert_eq!(
        c.native_negotiation()
            .unwrap()
            .unwrap()
            .stream_revision
            .get(),
        4
    );
    let error = c
        .subscribe(selectors(), revision(4), opts("selection-retired-handle"))
        .await
        .err()
        .expect("retired handle cannot be adopted again");
    assert_eq!(error.kind, sdk::ErrorKind::Protocol);
    close(client, host).await;
}
