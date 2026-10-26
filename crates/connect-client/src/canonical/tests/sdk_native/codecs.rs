//! Real signed codec reoffers, paired receipts and RTP binding replacement.
use super::*;
use crate::sdk::{CodecObservation, ConnectionUpdate, ConnectionUpdateOutcome};
#[derive(Clone, Copy, Debug)]
pub(in crate::canonical) enum Case {
    Normal,
    Unknown,
    GateAhead,
    BadSetup,
    BadDirection,
    BadCodec,
    BadParticipant,
    Collision,
    DataCollision,
    DataChanged,
    DataZero,
    BadResult,
    Failed,
}
fn options_for(id: &str) -> MutationOptions {
    MutationOptions {
        operation_id: id.parse().unwrap(),
        request: options(),
        extensions: None,
    }
}
fn revision(n: u64) -> sdk::StreamRevision {
    sdk::StreamRevision::try_from(n).unwrap()
}
fn input() -> ConnectionUpdate {
    ConnectionUpdate::CodecRenegotiate {
        streams: Some(NonEmpty {
            first: "audio".parse().unwrap(),
            rest: vec![],
        }),
        codec_preferences: NonEmpty {
            first: "pcma".into(),
            rest: vec![],
        },
    }
}
fn saved() -> Value {
    json!({"schema":"connect-operation-reference/1","operation_id":"codec-change","command":"connection.update","scope":{"conversation_id":"conv_one","session_id":"session_one","connection_id":"connection_one"}})
}
fn streams(handle: u16) -> Value {
    json!([
        {"strm_id":"audio","kind":"audio","direction":"sendrecv","stream_local_id":handle,"codec":{"name":"pcma","params":{"sample_rate":8000,"channels":1,"payload_type":8}}},
        {"strm_id":"video","kind":"video","direction":"sendonly","stream_local_id":8,"codec":{"name":"vp8","params":{"sample_rate":90000,"channels":0,"payload_type":96}}},
        {"strm_id":"data","kind":"data","direction":"sendrecv","stream_local_id":9,"codec":{"name":"json","params":{"reliability":"reliable_ordered"}}}
    ])
}
fn outcome(handle: u16) -> Value {
    let values = streams(handle);
    json!({"connection_id":"connection_one","stream_revision":2,"resources_changed":true,"codec_bindings":values.as_array().unwrap().iter().map(|s|json!({"stream_id":s["strm_id"],"medium":s["kind"],"direction":s["direction"],"codec":s["codec"],"stream_local_id":s["stream_local_id"],"stream_generation":if s["strm_id"]=="audio"{2}else{1},"connection_generation":1})).collect::<Vec<_>>()})
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
    let mut payload = json!({"operation_id":id,"state":state,"revision":1});
    if let Some(result) = result {
        payload["result"] = result;
    }
    write(writer, signer, request, "operation.updated", payload, true).await;
    write(writer, signer, request, "ack", json!({}), true).await;
}
async fn reoffer(
    reader: &mut quinn::RecvStream,
    writer: &mut quinn::SendStream,
    signer: &Sig9421Signer,
    caps: &Value,
    request: &Envelope,
    case: Case,
    handle: u16,
) -> bool {
    assert_eq!(request.kind(), "connection.update");
    assert_eq!(
        request.original()["payload"],
        json!({"action":"codec-renegotiate","streams":["audio"],"codec_preferences":["pcma"]})
    );
    assert_eq!(request.original()["expected_revision"], 1);
    let mut values = streams(handle);
    if matches!(case, Case::BadDirection) {
        values[0]["direction"] = json!("sendonly");
    }
    if matches!(case, Case::BadCodec) {
        values[0]["codec"] =
            json!({"name":"pcmu","params":{"sample_rate":8000,"channels":1,"payload_type":0}});
    }
    if matches!(case, Case::Collision) {
        values[0]["stream_local_id"] = json!(8);
    }
    if matches!(case, Case::DataCollision) {
        values[0]["stream_local_id"] = json!(9);
    }
    if matches!(case, Case::DataChanged) {
        values[2]["stream_local_id"] = json!(10);
    }
    if matches!(case, Case::DataZero) {
        values[2]["stream_local_id"] = json!(0);
    }
    let setup = json!({"kind":"native-prepared/1","revision":if matches!(case,Case::BadSetup){99}else{2},"offer_id":request.id(),"streams":values});
    let offered=values.as_array().unwrap().iter().map(|s|json!({"id":s["strm_id"],"kind":s["kind"],"direction":s["direction"],"codec_preferences":[s["codec"]["name"]]})).collect::<Vec<_>>();
    let callback=write(writer,signer,request,"connection.offer",json!({"by_participant":if matches!(case,Case::BadParticipant){"other-participant"}else{"part_one"},"substrate":"quic","capabilities":caps,"streams_offered":offered,"substrate_setup":setup}),false).await;
    if matches!(
        case,
        Case::BadSetup
            | Case::BadDirection
            | Case::BadCodec
            | Case::BadParticipant
            | Case::Collision
            | Case::DataCollision
            | Case::DataChanged
            | Case::DataZero
    ) {
        loop {
            let mut prefix = [0; 4];
            if reader.read_exact(&mut prefix).await.is_err() {
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
                "invalid reoffer must never receive signed confirmation"
            );
        }
        return false;
    }
    loop {
        let reply = read(reader).await;
        if reply.kind() == "operation.get" {
            respond(writer, signer, &reply, "preparing", None).await;
            continue;
        }
        assert_eq!(reply.kind(), "connection.answer");
        assert_eq!(reply.scope(Scope::Reply), Some(callback.id()));
        assert_eq!(reply.original()["payload"]["substrate_setup"], setup);
        assert_eq!(
            reply.original()["payload"]["streams_answered"][0]["direction"],
            "sendrecv"
        );
        assert_eq!(
            reply.original()["payload"]["streams_answered"][2]["kind"],
            "data"
        );
        break;
    }
    true
}
fn gated(stream: &str, action: &str, rev: u64) -> Value {
    let mut result = outcome(17);
    let binding = result["codec_bindings"]
        .as_array_mut()
        .unwrap()
        .iter()
        .find(|b| b["stream_id"] == stream)
        .unwrap()
        .clone();
    json!({"connection_id":"connection_one","connection_generation":1,"stream_revision":rev,"resources_changed":false,"action":action,"gate_streams":[{"binding":binding,"state":if action=="hold"{"paused"}else{"active"},"source_muted":action=="mute","held_direction":null}]})
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
    if matches!(case, Case::Normal) {
        connection
            .send_datagram(datagram(7, 49, &raw_rtp(0)).into())
            .unwrap();
        connection
            .send_datagram(datagram(8, 50, &raw_rtp(96)).into())
            .unwrap();
    }
    let request = read(reader).await;
    if !reoffer(reader, writer, signer, caps, &request, case, 17).await {
        return;
    }
    if matches!(case, Case::Normal | Case::BadResult) {
        signal.notify_one();
        let bytes = connection.read_datagram().await.unwrap();
        assert_eq!(bytes.as_ref(), datagram(7, 42, &raw_rtp(0)));
    }
    if matches!(case, Case::GateAhead) {
        signal.notify_one();
        let first = read(reader).await;
        let second = read(reader).await;
        let (mute, hold) = if first.scope(Scope::Operation) == Some("codec-later-mute") {
            (first, second)
        } else {
            (second, first)
        };
        assert_eq!(mute.original()["expected_revision"], 2);
        assert_eq!(hold.original()["expected_revision"], 3);
        respond(
            writer,
            signer,
            &hold,
            "completed",
            Some(gated("video", "hold", 4)),
        )
        .await;
        respond(
            writer,
            signer,
            &mute,
            "completed",
            Some(gated("audio", "mute", 3)),
        )
        .await;
        for _ in 0..2 {
            let query = read(reader).await;
            let id = query.original()["payload"]["operation_id"]
                .as_str()
                .unwrap();
            respond(
                writer,
                signer,
                &query,
                "completed",
                Some(if id == "codec-later-mute" {
                    gated("audio", "mute", 3)
                } else {
                    assert_eq!(id, "codec-later-hold");
                    gated("video", "hold", 4)
                }),
            )
            .await;
        }
        let barrier = read(reader).await;
        assert_eq!(
            barrier.original()["payload"]["operation_id"],
            "native-original"
        );
        respond(writer, signer, &request, "completed", Some(outcome(17))).await;
        status(writer, signer, &barrier, "completed").await;
        let query = read(reader).await;
        assert_eq!(query.original()["payload"]["operation_id"], "codec-change");
        respond(writer, signer, &query, "completed", Some(outcome(17))).await;
        return;
    }
    if matches!(case, Case::Unknown | Case::Failed) {
        respond(writer, signer, &request, "unknown", None).await;
        let query = read(reader).await;
        respond(writer, signer, &query, "unknown", None).await;
        let query = read(reader).await;
        respond(
            writer,
            signer,
            &query,
            if matches!(case, Case::Failed) {
                "failed"
            } else {
                "completed"
            },
            if matches!(case, Case::Failed) {
                None
            } else {
                Some(outcome(17))
            },
        )
        .await;
        if matches!(case, Case::Failed) {
            let retry = read(reader).await;
            assert_eq!(retry.scope(Scope::Operation), Some("codec-retry"));
            if reoffer(reader, writer, signer, caps, &retry, Case::Normal, 18).await {
                respond(writer, signer, &retry, "completed", Some(outcome(18))).await;
                let query = read(reader).await;
                respond(writer, signer, &query, "completed", Some(outcome(18))).await;
            }
        }
        return;
    }
    if matches!(case, Case::BadResult) {
        respond(writer, signer, &request, "completed", Some(outcome(19))).await;
        return;
    }
    respond(writer, signer, &request, "completed", Some(outcome(17))).await;
    let query = read(reader).await;
    respond(writer, signer, &query, "completed", Some(outcome(17))).await;
    let bytes = connection.read_datagram().await.unwrap();
    assert_eq!(bytes.as_ref(), datagram(17, 43, &raw_rtp(8)));
    connection
        .send_datagram(datagram(17, 44, &raw_rtp(8)).into())
        .unwrap();
    connection
        .send_datagram(datagram(7, 45, &raw_rtp(0)).into())
        .unwrap();
}
async fn ready(
    case: Case,
) -> (
    Client,
    sdk::Connection,
    tokio::task::JoinHandle<()>,
    Arc<Notify>,
) {
    let (peer, host, signal) = controlled(Fault::SdkNative(super::Case::Codec(case))).await;
    let client = Client::from_authenticated_peer(peer, "tenant".parse().unwrap()).unwrap();
    let session = client
        .conversation_by_id("conv_one".parse().unwrap())
        .session_by_id("session_one".parse().unwrap());
    let mut offer = offered();
    offer.first.direction = Direction::Sendrecv;
    offer.first.codecs.rest.push("pcma".into());
    let operation = session
        .offer_connection("connection_one".parse().unwrap(), offer, super::mutation())
        .await
        .unwrap();
    operation.status(options()).await.unwrap();
    let connection = operation.wait(options()).await.unwrap().value;
    (client, connection, host, signal)
}
fn audio(connection: &sdk::Connection) -> sdk::Stream {
    connection.stream_by_id("audio".parse().unwrap())
}
fn codec_name(connection: &sdk::Connection) -> String {
    connection
        .native_negotiation()
        .unwrap()
        .unwrap()
        .streams
        .iter()
        .find(|s| s.id.as_str() == "audio")
        .unwrap()
        .codec["name"]
        .as_str()
        .unwrap()
        .into()
}
fn audio_receipt(streams: &[CodecObservation]) -> &CodecObservation {
    streams
        .iter()
        .find(|s| s.parameters.id.as_str() == "audio")
        .unwrap()
}
#[tokio::test]
async fn native_codec_pairs_confirmation_and_receipt_preserving_data_video_and_explicit_gap() {
    let (client, connection, host, signal) = ready(Case::Normal).await;
    tokio::time::timeout(Duration::from_secs(1), async {
        while client.media_stats().unwrap().queued_packets < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let pending = connection.update(input(), revision(1), options_for("codec-change"));
    tokio::pin!(pending);
    tokio::select! {_=&mut pending=>panic!("confirmation alone must not complete"),_=signal.notified()=>{}}
    assert_eq!(codec_name(&connection), "pcmu");
    audio(&connection)
        .send_rtp(sdk::RtpPacket::new(42, raw_rtp(0)).unwrap(), options())
        .await
        .unwrap();
    let operation = pending.await.unwrap();
    assert_eq!(operation.reference_json(), saved());
    let receipt = operation.wait(options()).await.unwrap().value;
    let ConnectionUpdateOutcome::Codecs {
        streams,
        resources_changed,
    } = receipt.outcome
    else {
        panic!("codec outcome")
    };
    assert!(resources_changed);
    assert_eq!(audio_receipt(&streams).stream_generation, 2);
    assert!(streams
        .iter()
        .find(|s| s.parameters.id.as_str() == "data")
        .unwrap()
        .parameters
        .stream_local_id
        .is_none());
    assert_eq!(codec_name(&connection), "pcma");
    assert_eq!(client.media_stats().unwrap().binding_change_drops, 1);
    assert!(matches!(
        audio(&connection).receive_packet(options()).await.unwrap(),
        sdk::MediaPacket::Gap { dropped_packets: 1 }
    ));
    assert!(
        matches!(connection.stream_by_id("video".parse().unwrap()).receive_packet(options()).await.unwrap(),sdk::MediaPacket::Rtp(p) if p.datagram_sequence==50)
    );
    assert!(
        matches!(audio(&connection).send_rtp(sdk::RtpPacket::new(43,raw_rtp(0)).unwrap(),options()).await,Err(e) if e.kind==sdk::ErrorKind::Validation)
    );
    audio(&connection)
        .send_rtp(sdk::RtpPacket::new(43, raw_rtp(8)).unwrap(), options())
        .await
        .unwrap();
    assert!(
        matches!(audio(&connection).receive_packet(options()).await.unwrap(),sdk::MediaPacket::Rtp(p) if p.datagram_sequence==44)
    );
    tokio::time::timeout(Duration::from_secs(1), async {
        while client.media_stats().unwrap().unknown_handles == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    close(client, host).await;
}
#[tokio::test]
async fn native_codec_unknown_recovers_from_paired_original_receipt_without_new_command() {
    let (client, connection, host, _) = ready(Case::Unknown).await;
    let operation = connection
        .update(input(), revision(1), options_for("codec-change"))
        .await
        .unwrap();
    assert_eq!(
        operation.status(options()).await.unwrap().state,
        sdk::OperationState::Unknown
    );
    assert_eq!(codec_name(&connection), "pcmu");
    client
        .restore(saved(), client.operation_kinds().connection_update)
        .unwrap()
        .wait(options())
        .await
        .unwrap();
    assert_eq!(codec_name(&connection), "pcma");
    close(client, host).await;
}
#[tokio::test]
async fn native_codec_cancelled_wait_and_out_of_order_gates_preserve_newer_flow_controls() {
    let (client, connection, host, signal) = ready(Case::GateAhead).await;
    let mutation = options_for("codec-change");
    let cancel = mutation.request.signal.clone();
    let pending = connection.update(input(), revision(1), mutation);
    tokio::pin!(pending);
    tokio::select! {_=&mut pending=>panic!("codec must stay pending"),_=signal.notified()=>{}}
    cancel.cancel();
    let error = pending.await.unwrap_err();
    assert_eq!(error.certainty, sdk::Certainty::Uncertain);
    assert_eq!(
        error.operation.unwrap().operation_id.as_str(),
        "codec-change"
    );
    let mute = connection.update(
        ConnectionUpdate::Mute {
            streams: NonEmpty {
                first: "audio".parse().unwrap(),
                rest: vec![],
            },
        },
        revision(2),
        options_for("codec-later-mute"),
    );
    let hold = connection.update(
        ConnectionUpdate::Hold {
            streams: Some(NonEmpty {
                first: "video".parse().unwrap(),
                rest: vec![],
            }),
            direction: sdk::HoldDirection::Inactive,
        },
        revision(3),
        options_for("codec-later-hold"),
    );
    let (mute, hold) = tokio::join!(mute, hold);
    let mute = mute.unwrap();
    let hold = hold.unwrap();
    let (mute, hold) = tokio::join!(mute.wait(options()), hold.wait(options()));
    mute.unwrap();
    hold.unwrap();
    assert_eq!(codec_name(&connection), "pcmu");
    let busy = connection
        .update(input(), revision(1), options_for("codec-must-wait"))
        .await
        .unwrap_err();
    assert_eq!(busy.kind, sdk::ErrorKind::Conflict);
    assert_eq!(busy.certainty, sdk::Certainty::NotSubmitted);
    let original = client
        .restore(saved(), client.operation_kinds().connection_update)
        .unwrap();
    let barrier=client.restore(json!({"schema":"connect-operation-reference/1","operation_id":"native-original","command":"connection.offer","scope":{"conversation_id":"conv_one","session_id":"session_one","connection_id":"connection_one"}}),client.operation_kinds().connection_offer).unwrap();
    assert_eq!(
        barrier.status(options()).await.unwrap().state,
        sdk::OperationState::Completed
    );
    assert_eq!(codec_name(&connection), "pcma");
    assert_eq!(
        connection
            .native_negotiation()
            .unwrap()
            .unwrap()
            .stream_revision
            .get(),
        4
    );
    assert!(
        matches!(audio(&connection).send_rtp(sdk::RtpPacket::new(99,raw_rtp(8)).unwrap(),options()).await,Err(e) if e.kind==sdk::ErrorKind::Validation)
    );
    assert!(
        matches!(connection.stream_by_id("video".parse().unwrap()).receive_packet(options()).await,Err(e) if e.kind==sdk::ErrorKind::Unsupported)
    );
    let receipt = original.wait(options()).await.unwrap().value;
    assert_eq!(receipt.stream_revision.get(), 2);
    assert_eq!(
        connection
            .native_negotiation()
            .unwrap()
            .unwrap()
            .stream_revision
            .get(),
        4
    );
    close(client, host).await;
}
#[tokio::test]
async fn native_codec_signed_reoffers_refuse_unrequested_parameters_or_reserved_handles_before_signing(
) {
    for case in [
        Case::BadSetup,
        Case::BadDirection,
        Case::BadCodec,
        Case::BadParticipant,
        Case::Collision,
        Case::DataCollision,
        Case::DataChanged,
        Case::DataZero,
    ] {
        let (client, connection, host, _) = ready(case).await;
        let invalid = connection
            .update(
                ConnectionUpdate::CodecRenegotiate {
                    streams: None,
                    codec_preferences: NonEmpty {
                        first: "unknown-codec".into(),
                        rest: vec![],
                    },
                },
                revision(1),
                options_for("unsupported-local"),
            )
            .await
            .unwrap_err();
        assert_eq!(invalid.kind, sdk::ErrorKind::Unsupported);
        assert_eq!(invalid.certainty, sdk::Certainty::NotSubmitted);
        let error = connection
            .update(input(), revision(1), options_for("codec-change"))
            .await
            .unwrap_err();
        assert_eq!(error.kind, sdk::ErrorKind::Protocol, "{case:?}");
        assert_eq!(
            error.operation.unwrap().operation_id.as_str(),
            "codec-change"
        );
        close(client, host).await;
    }
}
#[tokio::test]
async fn native_codec_wrong_captured_result_cannot_activate_unconfirmed_hops() {
    let (client, connection, host, signal) = ready(Case::BadResult).await;
    let pending = connection.update(input(), revision(1), options_for("codec-change"));
    tokio::pin!(pending);
    tokio::select! {_=&mut pending=>panic!("pending codec"),_=signal.notified()=>{}}
    assert_eq!(codec_name(&connection), "pcmu");
    audio(&connection)
        .send_rtp(sdk::RtpPacket::new(42, raw_rtp(0)).unwrap(), options())
        .await
        .unwrap();
    let error = pending.await.unwrap_err();
    assert_eq!(error.kind, sdk::ErrorKind::Protocol);
    assert_eq!(
        error.operation.unwrap().operation_id.as_str(),
        "codec-change"
    );
    close(client, host).await;
}
#[tokio::test]
async fn native_codec_recorded_failure_releases_pending_proof_but_preserves_old_binding() {
    let (client, connection, host, _) = ready(Case::Failed).await;
    let operation = connection
        .update(input(), revision(1), options_for("codec-change"))
        .await
        .unwrap();
    assert_eq!(
        operation.status(options()).await.unwrap().state,
        sdk::OperationState::Unknown
    );
    let Err(error) = operation.wait(options()).await else {
        panic!("definitive failure must remain failed")
    };
    assert_eq!(error.certainty, sdk::Certainty::Recorded);
    assert_eq!(codec_name(&connection), "pcmu");
    connection
        .update(input(), revision(1), options_for("codec-retry"))
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap();
    assert_eq!(
        connection.native_negotiation().unwrap().unwrap().streams[0].stream_local_id,
        Some(18)
    );
    close(client, host).await;
}
