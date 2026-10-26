//! Signed controls exercise local queue policy and original-ID uncertainty.
use super::*;
use crate::sdk::{ConnectionUpdate, ConnectionUpdateOutcome, GateAction, GateState, HoldDirection};
#[derive(Clone, Copy)]
pub(in crate::canonical) enum Case {
    Normal,
    Unknown,
    Cancelled,
    WrongTarget,
    WrongInterval,
}
fn mutation(id: &str) -> MutationOptions {
    MutationOptions {
        operation_id: id.parse().unwrap(),
        request: options(),
        extensions: None,
    }
}
fn targets(id: &str) -> NonEmpty<sdk::StreamId> {
    NonEmpty {
        first: id.parse().unwrap(),
        rest: vec![],
    }
}
fn revision(n: u64) -> sdk::StreamRevision {
    sdk::StreamRevision::try_from(n).unwrap()
}
fn saved() -> Value {
    json!({"schema":"connect-operation-reference/1","operation_id":"control-hold","command":"connection.update","scope":{"conversation_id":"conv_one","session_id":"session_one","connection_id":"connection_one"}})
}
fn gate(action: &str, stream: &str, revision: u64) -> Value {
    let video = stream == "video";
    let binding = json!({"stream_id":stream,"medium":if video{"video"}else{"audio"},"direction":if video{"sendonly"}else{"recvonly"},"codec":if video{json!({"name":"vp8","params":{"sample_rate":90000,"channels":0,"payload_type":96}})}else{json!({"name":"pcmu","params":{"sample_rate":8000,"channels":1,"payload_type":0}})},"stream_local_id":if video{8}else{7},"stream_generation":1,"connection_generation":1});
    json!({"connection_id":"connection_one","connection_generation":1,"stream_revision":revision,"resources_changed":false,"action":action,"gate_streams":[{"binding":binding,"state":if action=="hold"{"paused"}else{"active"},"source_muted":action=="mute","held_direction":null}]})
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
    let mut update = json!({"operation_id":id,"state":state,"revision":1});
    if let Some(result) = result {
        update["result"] = result;
    }
    write(writer, signer, request, "operation.updated", update, true).await;
    write(writer, signer, request, "ack", json!({}), true).await;
}
async fn step(
    reader: &mut quinn::RecvStream,
    writer: &mut quinn::SendStream,
    signer: &Sig9421Signer,
    id: &str,
    expected: u64,
    input: Value,
    result: Value,
) {
    let request = read(reader).await;
    assert_eq!(request.kind(), "connection.update");
    assert_eq!(request.scope(Scope::Operation), Some(id));
    assert_eq!(request.original()["expected_revision"], expected);
    assert_eq!(request.original()["payload"], input);
    respond(writer, signer, &request, "completed", Some(result.clone())).await;
    let query = read(reader).await;
    assert_eq!(query.kind(), "operation.get");
    assert_eq!(query.original()["payload"]["operation_id"], id);
    respond(writer, signer, &query, "completed", Some(result)).await;
}
pub(super) async fn emit(
    connection: &quinn::Connection,
    case: Case,
    writer: &mut quinn::SendStream,
    reader: &mut quinn::RecvStream,
    signer: &Sig9421Signer,
    signal: &Notify,
) {
    if matches!(case, Case::Normal | Case::Unknown) {
        connection
            .send_datagram(datagram(8, 10, &raw_rtp(96)).into())
            .unwrap();
    }
    if matches!(case, Case::WrongInterval) {
        let request = read(reader).await;
        assert_eq!(
            request.original()["payload"],
            json!({"action":"quality","quality_interval_ms":250})
        );
        respond(writer,signer,&request,"completed",Some(json!({"connection_id":"connection_one","connection_generation":1,"stream_revision":1,"resources_changed":false,"quality_interval_ms":500}))).await;
        return;
    }
    let request = read(reader).await;
    assert_eq!(
        request.original()["payload"],
        json!({"action":"hold","details":{"direction":"inactive"},"streams":["video"]})
    );
    let result = gate(
        "hold",
        if matches!(case, Case::WrongTarget) {
            "audio"
        } else {
            "video"
        },
        2,
    );
    if matches!(case, Case::WrongTarget) {
        respond(writer, signer, &request, "completed", Some(result)).await;
        return;
    }
    if matches!(case, Case::Cancelled) {
        signal.notify_one();
        let query = read(reader).await;
        assert_eq!(query.kind(), "operation.get");
        respond(writer, signer, &request, "completed", Some(result.clone())).await;
        respond(writer, signer, &query, "completed", Some(result.clone())).await;
    } else {
        respond(
            writer,
            signer,
            &request,
            if matches!(case, Case::Unknown) {
                "unknown"
            } else {
                "completed"
            },
            if matches!(case, Case::Unknown) {
                None
            } else {
                Some(result.clone())
            },
        )
        .await;
        let query = read(reader).await;
        assert_eq!(query.kind(), "operation.get");
        respond(
            writer,
            signer,
            &query,
            if matches!(case, Case::Unknown) {
                "unknown"
            } else {
                "completed"
            },
            if matches!(case, Case::Unknown) {
                None
            } else {
                Some(result.clone())
            },
        )
        .await;
    }
    if !matches!(case, Case::Normal) {
        let query = read(reader).await;
        assert_eq!(
            query.kind(),
            "operation.get",
            "uncertainty recovery must never resubmit the effect"
        );
        respond(writer, signer, &query, "completed", Some(result)).await;
        return;
    }
    step(
        reader,
        writer,
        signer,
        "control-resume",
        2,
        json!({"action":"resume","streams":["video"]}),
        gate("resume", "video", 3),
    )
    .await;
    let query = read(reader).await;
    assert_eq!(query.kind(), "operation.get");
    status(writer, signer, &query, "completed").await;
    connection
        .send_datagram(datagram(8, 12, &raw_rtp(96)).into())
        .unwrap();
    step(
        reader,
        writer,
        signer,
        "control-mute",
        3,
        json!({"action":"mute","streams":["audio"]}),
        gate("mute", "audio", 4),
    )
    .await;
    step(
        reader,
        writer,
        signer,
        "control-unmute",
        4,
        json!({"action":"unmute","streams":["audio"]}),
        gate("unmute", "audio", 5),
    )
    .await;
    let bytes = connection.read_datagram().await.unwrap();
    assert_eq!(bytes.as_ref(), datagram(7, 42, &raw_rtp(0)));
    step(reader,writer,signer,"control-quality",5,json!({"action":"quality","quality_interval_ms":250}),json!({"connection_id":"connection_one","connection_generation":1,"stream_revision":5,"resources_changed":false,"quality_interval_ms":250})).await;
    step(
        reader,
        writer,
        signer,
        "control-hold",
        1,
        json!({"action":"hold","details":{"direction":"inactive"},"streams":["video"]}),
        gate("hold", "video", 2),
    )
    .await;
    connection
        .send_datagram(datagram(8, 13, &raw_rtp(96)).into())
        .unwrap();
}
async fn ready(
    case: Case,
) -> (
    Client,
    sdk::Connection,
    sdk::Operation<sdk::Connection>,
    tokio::task::JoinHandle<()>,
    std::sync::Arc<Notify>,
) {
    let (peer, host, signal) = controlled(Fault::SdkNative(super::Case::Controls(case))).await;
    let client = Client::from_authenticated_peer(peer, "tenant".parse().unwrap()).unwrap();
    let session = client
        .conversation_by_id("conv_one".parse().unwrap())
        .session_by_id("session_one".parse().unwrap());
    let operation = session
        .offer_connection(
            "connection_one".parse().unwrap(),
            offered(),
            super::mutation(),
        )
        .await
        .unwrap();
    operation.status(options()).await.unwrap();
    let connection = operation.wait(options()).await.unwrap().value;
    (client, connection, operation, host, signal)
}
fn hold() -> ConnectionUpdate {
    ConnectionUpdate::Hold {
        streams: Some(targets("video")),
        direction: HoldDirection::Inactive,
    }
}
async fn queued(c: &Client) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while c.media_stats().unwrap().queued_packets == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn native_runtime_controls_preserve_queue_gaps_direction_and_revision_monotonicity() {
    let (c, connection, offer, host, _) = ready(Case::Normal).await;
    queued(&c).await;
    let video = connection.stream_by_id("video".parse().unwrap());
    let audio = connection.stream_by_id("audio".parse().unwrap());
    let error = connection
        .update(
            ConnectionUpdate::Quality { interval_ms: 1 },
            revision(1),
            mutation("invalid-quality"),
        )
        .await
        .unwrap_err();
    assert_eq!(error.certainty, sdk::Certainty::NotSubmitted);
    assert_eq!(
        error.operation.unwrap().operation_id.as_str(),
        "invalid-quality"
    );
    let error = connection
        .update(
            ConnectionUpdate::Mute {
                streams: NonEmpty {
                    first: "audio".parse().unwrap(),
                    rest: vec!["audio".parse().unwrap()],
                },
            },
            revision(1),
            mutation("duplicate-target"),
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind, sdk::ErrorKind::Validation);
    let op = connection
        .update(hold(), revision(1), mutation("control-hold"))
        .await
        .unwrap();
    assert_eq!(op.reference_json(), saved());
    let receipt = op.wait(options()).await.unwrap().value;
    assert!(
        matches!(receipt.outcome,ConnectionUpdateOutcome::Gates{action:GateAction::Hold,ref streams} if streams[0].state==GateState::Paused && streams[0].parameters.direction==Direction::Recvonly)
    );
    assert_eq!(c.media_stats().unwrap().queued_bytes, 0);
    assert_eq!(c.media_stats().unwrap().control_drops, 1);
    assert!(
        matches!(video.receive_packet(options()).await,Err(e) if e.kind==sdk::ErrorKind::Unsupported)
    );
    connection
        .update(
            ConnectionUpdate::Resume {
                streams: Some(targets("video")),
            },
            revision(2),
            mutation("control-resume"),
        )
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap();
    assert!(matches!(
        video.receive_packet(options()).await.unwrap(),
        sdk::MediaPacket::Gap { dropped_packets: 1 }
    ));
    offer.status(options()).await.unwrap();
    assert!(
        matches!(video.receive_packet(options()).await.unwrap(),sdk::MediaPacket::Rtp(p) if p.datagram_sequence==12)
    );
    connection
        .update(
            ConnectionUpdate::Mute {
                streams: targets("audio"),
            },
            revision(3),
            mutation("control-mute"),
        )
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap();
    let error = audio
        .send_rtp(sdk::RtpPacket::new(41, raw_rtp(0)).unwrap(), options())
        .await
        .unwrap_err();
    assert_eq!(error.kind, sdk::ErrorKind::Validation);
    assert_eq!(error.certainty, sdk::Certainty::NotSubmitted);
    connection
        .update(
            ConnectionUpdate::Unmute {
                streams: targets("audio"),
            },
            revision(4),
            mutation("control-unmute"),
        )
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap();
    audio
        .send_rtp(sdk::RtpPacket::new(42, raw_rtp(0)).unwrap(), options())
        .await
        .unwrap();
    let quality = connection
        .update(
            ConnectionUpdate::Quality { interval_ms: 250 },
            revision(5),
            mutation("control-quality"),
        )
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap()
        .value;
    assert_eq!(quality.stream_revision.get(), 5);
    assert!(matches!(
        quality.outcome,
        ConnectionUpdateOutcome::Quality { interval_ms: 250 }
    ));
    connection
        .update(hold(), revision(1), mutation("control-hold"))
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap();
    assert_eq!(
        connection
            .native_negotiation()
            .unwrap()
            .unwrap()
            .stream_revision
            .get(),
        5
    );
    assert!(
        matches!(video.receive_packet(options()).await.unwrap(),sdk::MediaPacket::Rtp(p) if p.datagram_sequence==13)
    );
    close(c, host).await;
}
#[tokio::test]
async fn native_runtime_unknown_control_recovers_only_from_original_receipt() {
    let (c, connection, _, host, _) = ready(Case::Unknown).await;
    queued(&c).await;
    let video = connection.stream_by_id("video".parse().unwrap());
    let op = connection
        .update(hold(), revision(1), mutation("control-hold"))
        .await
        .unwrap();
    assert_eq!(
        op.status(options()).await.unwrap().state,
        sdk::OperationState::Unknown
    );
    assert_eq!(
        connection
            .native_negotiation()
            .unwrap()
            .unwrap()
            .stream_revision
            .get(),
        1
    );
    assert!(
        matches!(video.receive_packet(options()).await.unwrap(),sdk::MediaPacket::Rtp(p) if p.datagram_sequence==10)
    );
    c.restore(saved(), c.operation_kinds().connection_update)
        .unwrap()
        .wait(options())
        .await
        .unwrap();
    assert!(
        matches!(video.receive_packet(options()).await,Err(e) if e.kind==sdk::ErrorKind::Unsupported)
    );
    close(c, host).await;
}
#[tokio::test]
async fn native_runtime_cancelled_caller_does_not_abandon_owned_gate_completion() {
    let (c, connection, _, host, signal) = ready(Case::Cancelled).await;
    let input = mutation("control-hold");
    let cancellation = input.request.signal.clone();
    let request = connection.update(hold(), revision(1), input);
    tokio::pin!(request);
    tokio::select! {_=&mut request=>panic!("control must remain pending"),_=signal.notified()=>{}}
    cancellation.cancel();
    let error = request.await.unwrap_err();
    assert_eq!(error.certainty, sdk::Certainty::Uncertain);
    assert_eq!(
        error.operation.unwrap().operation_id.as_str(),
        "control-hold"
    );
    let restored = c
        .restore(saved(), c.operation_kinds().connection_update)
        .unwrap();
    assert_eq!(
        restored.status(options()).await.unwrap().state,
        sdk::OperationState::Completed
    );
    let video = connection.stream_by_id("video".parse().unwrap());
    assert!(
        matches!(video.receive_packet(options()).await,Err(e) if e.kind==sdk::ErrorKind::Unsupported)
    );
    restored.wait(options()).await.unwrap();
    close(c, host).await;
}
#[tokio::test]
async fn native_runtime_signed_wrong_targets_and_quality_results_refuse_original_key() {
    for case in [Case::WrongTarget, Case::WrongInterval] {
        let (c, connection, _, host, _) = ready(case).await;
        let result = connection
            .update(
                if matches!(case, Case::WrongInterval) {
                    ConnectionUpdate::Quality { interval_ms: 250 }
                } else {
                    hold()
                },
                revision(1),
                mutation("control-hold"),
            )
            .await;
        let error = result.unwrap_err();
        assert_eq!(error.kind, sdk::ErrorKind::Protocol);
        assert_eq!(error.certainty, sdk::Certainty::Uncertain);
        assert_eq!(
            error.operation.unwrap().operation_id.as_str(),
            "control-hold"
        );
        close(c, host).await;
    }
}
