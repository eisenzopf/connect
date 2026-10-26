//! Real signed native frames exercise the SDK's two-leg owned negotiation.
use super::*;
mod codecs;
mod end;
mod reconciliations;
mod selections;
mod updates;
use crate::sdk::{
    self, Client, Direction, Medium, MutationOptions, NonEmpty, OfferedStream, RequestOptions,
};
#[derive(Clone, Copy)]
pub(in crate::canonical) enum Case {
    Mixed,
    Selection(selections::Case),
    Reconciliation(reconciliations::Case),
    Codec(codecs::Case),
    Controls(updates::Case),
    End,
    EndHandleReuse,
    EndCancelled,
    EndUnknown,
    EndWrongConnection,
    EndWrongRevision,
    EndRefused,
    Media,
    ChangedSetup,
    ForeignScope,
    WrongDirection,
    UnofferedCodec,
    Refused,
    Replay,
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
async fn write(
    writer: &mut quinn::SendStream,
    signer: &Sig9421Signer,
    request: &Envelope,
    kind: &str,
    payload: Value,
    reply: bool,
) -> Envelope {
    let mut frame = envelope(kind, payload);
    for field in ["cid", "sid", "connid", "operation_id"] {
        if let Some(value) = request.original().get(field) {
            frame[field] = value.clone();
        }
    }
    if reply {
        frame["in_reply_to"] = json!(request.id());
    }
    let frame = signer.sign(&frame).unwrap();
    let bytes = serde_json::to_vec(&frame).unwrap();
    let frame = Envelope::parse(&bytes).unwrap();
    writer
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .await
        .unwrap();
    writer.write_all(&bytes).await.unwrap();
    frame
}
async fn status(
    writer: &mut quinn::SendStream,
    signer: &Sig9421Signer,
    request: &Envelope,
    state: &str,
) {
    let mut value = json!({"operation_id":"native-original","state":state,"revision":1});
    if state == "completed" {
        value["result"] = json!({"streams":["audio","video","data"],"stream_revision":1});
    }
    write(writer, signer, request, "operation.updated", value, true).await;
    write(writer, signer, request, "ack", json!({}), true).await;
}
pub(super) async fn emit(
    connection: &quinn::Connection,
    case: Case,
    first: Envelope,
    writer: &mut quinn::SendStream,
    reader: &mut quinn::RecvStream,
    signer: &Sig9421Signer,
    signal: &Notify,
) {
    assert_eq!(first.kind(), "connection.offer");
    assert_eq!(first.original()["expected_revision"], 0);
    assert_eq!(first.scope(Scope::Operation), Some("native-original"));
    if matches!(case, Case::Refused) {
        write(
            writer,
            signer,
            &first,
            "error",
            json!({"code":403,"category":"authorization","reason":"Current actor refused"}),
            true,
        )
        .await;
        return;
    }
    if matches!(case, Case::Replay) {
        status(writer, signer, &first, "completed").await;
        let query = read(reader).await;
        assert_eq!(query.kind(), "operation.get");
        status(writer, signer, &query, "completed").await;
        return;
    }
    let audio_direction = match first.original()["payload"]["streams_offered"][0]["direction"]
        .as_str()
        .unwrap()
    {
        "sendrecv" => "sendrecv",
        "sendonly" => "recvonly",
        "recvonly" => "sendonly",
        _ => panic!("inactive"),
    };
    let mut streams = json!([
        {"strm_id":"audio","kind":"audio","direction":audio_direction,"stream_local_id":7,"codec":{"name":"pcmu","params":{"sample_rate":8000,"channels":1,"payload_type":0}}},
        {"strm_id":"video","kind":"video","direction":"sendonly","stream_local_id":8,"codec":{"name":"vp8","params":{"sample_rate":90000,"channels":0,"payload_type":96}}},
        {"strm_id":"data","kind":"data","direction":"sendrecv","stream_local_id":9,"codec":{"name":"json","params":{"reliability":"reliable_ordered"}}}
    ]);
    if matches!(case, Case::WrongDirection) {
        streams[0]["direction"] = json!("sendrecv");
    }
    if matches!(case, Case::UnofferedCodec) {
        streams[0]["codec"]["name"] = json!("pcma");
    }
    let setup =
        json!({"kind":"native-prepared/1","revision":1,"offer_id":first.id(),"streams":streams});
    let answered=streams.as_array().unwrap().iter().map(|s|json!({"id":s["strm_id"],"kind":s["kind"],"direction":s["direction"],"codec":s["codec"]})).collect::<Vec<_>>();
    let capabilities = first.original()["payload"]["capabilities"].clone();
    let answer = json!({"by_participant":"part_one","substrate":"quic","capabilities":capabilities,"streams_answered":answered,"substrate_setup":setup});
    write(writer, signer, &first, "connection.answer", answer, true).await;
    if matches!(case, Case::WrongDirection | Case::UnofferedCodec) {
        return;
    }
    // The initial answer has arrived, but the server has not requested confirmation.
    let query = read(reader).await;
    assert_eq!(query.kind(), "operation.get");
    status(writer, signer, &query, "preparing").await;
    let offered=streams.as_array().unwrap().iter().map(|s|json!({"id":s["strm_id"],"kind":s["kind"],"direction":s["direction"],"codec_preferences":[s["codec"]["name"]]})).collect::<Vec<_>>();
    let mut payload = json!({"by_participant":"part_one","substrate":"quic","capabilities":capabilities,"streams_offered":offered,"substrate_setup":setup});
    if matches!(case, Case::ChangedSetup) {
        payload["substrate_setup"]["streams"][0]["stream_local_id"] = json!(99);
    }
    let mut root = first.original().clone();
    if matches!(case, Case::ForeignScope) {
        root["cid"] = json!("foreign-room");
    }
    let callback = write(
        writer,
        signer,
        &Envelope::from_value(root).unwrap(),
        "connection.offer",
        payload,
        false,
    )
    .await;
    if matches!(case, Case::ChangedSetup | Case::ForeignScope) {
        // A status query may already be in flight; no native confirmation may be signed.
        loop {
            let mut prefix = [0; 4];
            if reader.read_exact(&mut prefix).await.is_err() {
                return;
            }
            let mut raw = vec![0; u32::from_be_bytes(prefix) as usize];
            reader.read_exact(&mut raw).await.unwrap();
            assert_ne!(Envelope::parse(&raw).unwrap().kind(), "connection.answer");
        }
    }
    let confirmation = loop {
        let frame = read(reader).await;
        if frame.kind() == "operation.get" {
            status(writer, signer, &frame, "preparing").await;
            continue;
        }
        break frame;
    };
    assert_eq!(confirmation.kind(), "connection.answer");
    assert_eq!(confirmation.scope(Scope::Reply), Some(callback.id()));
    for field in ["cid", "sid", "connid", "operation_id"] {
        assert_eq!(confirmation.original()[field], first.original()[field]);
    }
    assert!(confirmation.original().get("expected_revision").is_none());
    assert_eq!(confirmation.original()["payload"]["substrate_setup"], setup);
    assert_eq!(
        confirmation.original()["payload"]["streams_answered"][0]["direction"],
        first.original()["payload"]["streams_offered"][0]["direction"]
    );
    assert_eq!(
        confirmation.original()["payload"]["streams_answered"][1]["direction"],
        "recvonly"
    );
    // There is deliberately no final ACK for the original native offer.
    let query = read(reader).await;
    assert_eq!(query.kind(), "operation.get");
    status(writer, signer, &query, "completed").await;
    if let Case::Codec(case) = case {
        codecs::emit(
            connection,
            case,
            reader,
            writer,
            signer,
            signal,
            &capabilities,
        )
        .await;
    } else if let Case::Reconciliation(case) = case {
        reconciliations::emit(
            connection,
            case,
            reader,
            writer,
            signer,
            signal,
            &capabilities,
        )
        .await;
    } else if let Case::Selection(case) = case {
        selections::emit(
            connection,
            case,
            reader,
            writer,
            signer,
            signal,
            &capabilities,
        )
        .await;
    } else if let Case::Controls(case) = case {
        updates::emit(connection, case, writer, reader, signer, signal).await;
    } else if matches!(case, Case::Media) {
        media_wire(connection, writer, reader, signer).await;
    } else if matches!(
        case,
        Case::End
            | Case::EndHandleReuse
            | Case::EndCancelled
            | Case::EndUnknown
            | Case::EndWrongConnection
            | Case::EndWrongRevision
            | Case::EndRefused
    ) {
        end::emit(case, writer, reader, signer, signal).await;
    }
}
fn options() -> RequestOptions {
    RequestOptions::new(Duration::from_secs(3)).unwrap()
}
fn mutation() -> MutationOptions {
    MutationOptions {
        operation_id: "native-original".parse().unwrap(),
        request: options(),
        extensions: None,
    }
}
fn offered() -> NonEmpty<OfferedStream> {
    let item = |id: &str, kind, direction, codec: &str| OfferedStream {
        id: id.parse().unwrap(),
        kind,
        direction,
        codecs: NonEmpty {
            first: codec.into(),
            rest: vec![],
        },
    };
    NonEmpty {
        first: item("audio", Medium::Audio, Direction::Sendonly, "g.711-mu"),
        rest: vec![
            item("video", Medium::Video, Direction::Recvonly, "vp8"),
            item("data", Medium::Data, Direction::Sendrecv, "json"),
        ],
    }
}
async fn client(case: Case) -> (Client, sdk::Session, tokio::task::JoinHandle<()>) {
    let (peer, host, _) = controlled(Fault::SdkNative(case)).await;
    let client = Client::from_authenticated_peer(peer, "tenant".parse().unwrap()).unwrap();
    let session = client
        .conversation_by_id("conv_one".parse().unwrap())
        .session_by_id("session_one".parse().unwrap());
    (client, session, host)
}
async fn close(client: Client, host: tokio::task::JoinHandle<()>) {
    client.close(options()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), host)
        .await
        .unwrap()
        .unwrap();
}
#[tokio::test]
async fn native_mixed_offer_keeps_pending_status_and_confirms_exact_multistream_parameters_without_final_ack(
) {
    let (client, session, host) = client(Case::Mixed).await;
    let operation = session
        .offer_connection("connection_one".parse().unwrap(), offered(), mutation())
        .await
        .unwrap();
    assert_eq!(
        operation.status(options()).await.unwrap().state,
        sdk::OperationState::Preparing
    );
    let completed = operation.wait(options()).await.unwrap();
    assert_eq!(completed.value.id().as_str(), "connection_one");
    assert_eq!(completed.value.receipt_stream_revision().unwrap().get(), 1);
    let negotiation = completed.value.native_negotiation().unwrap().unwrap();
    assert_eq!(negotiation.streams.len(), 3);
    assert_eq!(negotiation.streams[0].direction, Direction::Sendonly);
    assert_eq!(negotiation.streams[1].direction, Direction::Recvonly);
    assert_eq!(negotiation.streams[0].stream_local_id, Some(7));
    assert_eq!(negotiation.streams[2].stream_local_id, None);
    close(client, host).await;
}
#[tokio::test]
async fn native_offer_refuses_signed_direction_and_unoffered_codec_changes_with_original_key() {
    for case in [Case::WrongDirection, Case::UnofferedCodec] {
        let (client, session, host) = client(case).await;
        let error = session
            .offer_connection("connection_one".parse().unwrap(), offered(), mutation())
            .await
            .unwrap_err();
        assert_eq!(error.kind, sdk::ErrorKind::Protocol);
        assert_eq!(error.certainty, sdk::Certainty::Uncertain);
        assert_eq!(
            error.operation.unwrap().operation_id.as_str(),
            "native-original"
        );
        close(client, host).await;
    }
}
#[tokio::test]
async fn native_confirmation_refuses_changed_setup_and_foreign_scope_before_signing() {
    for case in [Case::ChangedSetup, Case::ForeignScope] {
        let (client, session, host) = client(case).await;
        let operation = session
            .offer_connection("connection_one".parse().unwrap(), offered(), mutation())
            .await
            .unwrap();
        assert_eq!(
            operation.status(options()).await.unwrap().state,
            sdk::OperationState::Preparing
        );
        let Err(error) = operation.wait(options()).await else {
            panic!("changed native confirmation must not complete")
        };
        assert_eq!(
            error.operation.unwrap().operation_id.as_str(),
            "native-original"
        );
        assert!(session
            .connection_by_id("connection_one".parse().unwrap())
            .native_negotiation()
            .is_err());
        close(client, host).await;
    }
}
#[tokio::test]
async fn native_offer_preserves_explicit_refusal_and_completed_replay_without_borrowing_media_handles(
) {
    let (c, session, host) = client(Case::Refused).await;
    let error = session
        .offer_connection("connection_one".parse().unwrap(), offered(), mutation())
        .await
        .unwrap_err();
    assert_eq!(error.kind, sdk::ErrorKind::Authentication);
    assert_eq!(error.certainty, sdk::Certainty::Refused);
    assert_eq!(error.problem.unwrap().code, 403);
    close(c, host).await;
    let (c, session, host) = client(Case::Replay).await;
    let operation = session
        .offer_connection("connection_one".parse().unwrap(), offered(), mutation())
        .await
        .unwrap();
    let completed = operation.wait(options()).await.unwrap();
    assert!(completed.value.native_negotiation().unwrap().is_none());
    close(c, host).await;
}

fn raw_rtp(pt: u8) -> Vec<u8> {
    let mut raw = vec![0x91, 0x80 | pt, 0x12, 0x34, 1, 2, 3, 4, 0, 0, 0, 9];
    raw.extend_from_slice(&7u32.to_be_bytes());
    raw.extend_from_slice(&[0xbe, 0xde, 0, 1, 0x10, 0xab, 0, 0]);
    raw.extend_from_slice(&[0x10, 0xaa, 0xbb]);
    raw
}
fn datagram(handle: u16, sequence: u32, raw: &[u8]) -> Vec<u8> {
    let mut wire = vec![1, 0];
    wire.extend_from_slice(&handle.to_be_bytes());
    wire.extend_from_slice(&sequence.to_be_bytes());
    wire.extend_from_slice(raw);
    wire
}
async fn media_wire(
    connection: &quinn::Connection,
    writer: &mut quinn::SendStream,
    reader: &mut quinn::RecvStream,
    signer: &Sig9421Signer,
) {
    let bytes = connection.read_datagram().await.unwrap();
    assert_eq!(bytes.as_ref(), datagram(7, 40, &raw_rtp(0)));
    let bytes = connection.read_datagram().await.unwrap();
    assert!(
        matches!(observe_native_media_datagram(&bytes).unwrap(),ObservedNativeMediaDatagram::Clock(c) if c.stream_local_id==7 && c.seq==41 && c.report.cname=="sdk-source")
    );
    connection
        .send_datagram(datagram(8, 42, &raw_rtp(96)).into())
        .unwrap();
    let report = SenderClockReport {
        ssrc: 9,
        ntp: 0x80000000ffffffff,
        rtp: 0x01020304,
        packets: 1,
        octets: 3,
        cname: "remote-source".into(),
    };
    connection
        .send_datagram(
            pack_sender_clock_datagram(&SenderClockDatagram {
                stream_local_id: 8,
                seq: 43,
                report,
            })
            .unwrap(),
        )
        .unwrap();
    connection
        .send_datagram(datagram(99, 44, &raw_rtp(96)).into())
        .unwrap();
    connection
        .send_datagram(datagram(8, 45, &raw_rtp(0)).into())
        .unwrap();
    let query = read(reader).await;
    assert_eq!(query.kind(), "operation.get");
    status(writer, signer, &query, "completed").await;
}
#[tokio::test]
async fn owned_sdk_media_preserves_wire_packets_and_clock_identity_with_cancellation_and_live_control(
) {
    let (client, session, host) = client(Case::Media).await;
    let operation = session
        .offer_connection("connection_one".parse().unwrap(), offered(), mutation())
        .await
        .unwrap();
    assert_eq!(
        operation.status(options()).await.unwrap().state,
        sdk::OperationState::Preparing
    );
    let connection = operation.wait(options()).await.unwrap().value;
    let audio = connection.stream_by_id("audio".parse().unwrap());
    let video = connection.stream_by_id("video".parse().unwrap());
    let error = video
        .send_rtp(sdk::RtpPacket::new(39, raw_rtp(96)).unwrap(), options())
        .await
        .unwrap_err();
    assert_eq!(error.certainty, sdk::Certainty::NotSubmitted);
    assert_eq!(
        audio
            .send_rtp(sdk::RtpPacket::new(40, raw_rtp(0)).unwrap(), options())
            .await
            .unwrap()
            .datagram_sequence,
        40
    );
    let report = SenderClockReport {
        ssrc: 9,
        ntp: 0x80000000ffffffff,
        rtp: 0x01020304,
        packets: 1,
        octets: 3,
        cname: "sdk-source".into(),
    };
    audio.send_clock(41, report, options()).await.unwrap();
    let cancelled = options();
    cancelled.signal.cancel();
    assert!(
        matches!(video.receive_packet(cancelled).await,Err(e) if e.kind==sdk::ErrorKind::Cancelled)
    );
    let mut got_rtp = false;
    let mut got_clock = false;
    for _ in 0..2 {
        match video.receive_packet(options()).await.unwrap() {
            sdk::MediaPacket::Rtp(packet) => {
                assert_eq!(packet.datagram_sequence, 42);
                assert_eq!(packet.bytes(), raw_rtp(96));
                assert!(packet.header().marker);
                assert_eq!(packet.payload(), [0x10, 0xaa, 0xbb]);
                got_rtp = true;
            }
            sdk::MediaPacket::Clock {
                datagram_sequence,
                report,
            } => {
                assert_eq!(datagram_sequence, 43);
                assert_eq!(report.cname, "remote-source");
                assert_eq!(report.ntp, 0x80000000ffffffff);
                got_clock = true;
            }
            _ => panic!("unexpected local gap"),
        }
    }
    assert!(got_rtp && got_clock);
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let stats = client.media_stats().unwrap();
            if stats.received_datagrams >= 4 {
                assert_eq!(stats.unknown_handles, 1);
                assert_eq!(stats.malformed_datagrams, 1);
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        operation.status(options()).await.unwrap().state,
        sdk::OperationState::Completed
    );
    close(client, host).await;
    assert!(
        matches!(audio.send_rtp(sdk::RtpPacket::new(46,raw_rtp(0)).unwrap(),options()).await,Err(e) if e.kind==sdk::ErrorKind::Cancelled && e.certainty==sdk::Certainty::NotSubmitted)
    );
}
