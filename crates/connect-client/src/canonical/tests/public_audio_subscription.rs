//! Controlled signed QUIC responses for the actual public audio SDK method.
use super::*;

async fn read(reader: &mut quinn::RecvStream) -> Option<Envelope> {
    let mut prefix = [0; 4];
    reader.read_exact(&mut prefix).await.ok()?;
    let size = u32::from_be_bytes(prefix) as usize;
    assert!(size > 0 && size <= MAX_ENVELOPE_BYTES);
    let mut bytes = vec![0; size];
    reader.read_exact(&mut bytes).await.ok()?;
    Some(Envelope::parse(&bytes).unwrap())
}
async fn write(
    writer: &mut quinn::SendStream,
    signer: &Sig9421Signer,
    request: &Envelope,
    kind: &str,
    payload: Value,
    reply: bool,
) -> Option<Envelope> {
    let mut value = envelope(kind, payload);
    for field in ["cid", "sid", "connid", "operation_id"] {
        if let Some(value_at) = request.original().get(field) {
            value[field] = value_at.clone();
        }
    }
    if reply {
        value["in_reply_to"] = json!(request.id());
    }
    let value = signer.sign(&value).unwrap();
    let bytes = serde_json::to_vec(&value).unwrap();
    let frame = Envelope::parse(&bytes).unwrap();
    writer
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .await
        .ok()?;
    writer.write_all(&bytes).await.ok()?;
    Some(frame)
}
fn update(completed: bool) -> Value {
    let mut value = json!({"operation_id":"original-subscription","record_id":"original-record",
        "command_type":"stream.subscribe","expected_revision":4,
        "state":if completed {"completed"} else {"preparing"},
        "revision":if completed {2} else {1}});
    if completed {
        value["result"] = json!({"connection_id":"original-connection","stream_revision":5});
    }
    value
}
fn query_is_original(query: &Envelope, original: &Envelope) {
    assert_eq!(
        query.kind(),
        "operation.get",
        "subscription must never be resubmitted"
    );
    assert_eq!(
        query.original()["payload"]["operation_id"],
        "original-subscription"
    );
    assert!(
        query.scope(Scope::Operation).is_none(),
        "Get has no new mutation ID"
    );
    assert!(query.expected_revision().is_none());
    for scope in [Scope::Conversation, Scope::Session, Scope::Connection] {
        assert_eq!(query.scope(scope), original.scope(scope));
    }
}
pub(super) async fn emit(
    stall: bool,
    connection: &quinn::Connection,
    original: Envelope,
    writer: &mut quinn::SendStream,
    reader: &mut quinn::RecvStream,
    signer: &Sig9421Signer,
    release_ack: &Notify,
) {
    assert_eq!(original.kind(), "stream.subscribe");
    assert_eq!(
        original.scope(Scope::Operation),
        Some("original-subscription")
    );
    assert_eq!(original.expected_revision(), Some(4));
    assert_eq!(original.scope(Scope::Conversation), Some("conv_one"));
    assert_eq!(original.scope(Scope::Session), Some("session_one"));
    assert_eq!(
        original.scope(Scope::Connection),
        Some("original-connection")
    );
    assert_eq!(
        original.original()["payload"]["subscriptions"],
        json!([{"from_participant":"part_source","kinds":["audio"]}])
    );
    write(
        writer,
        signer,
        &original,
        "operation.updated",
        update(false),
        true,
    )
    .await
    .unwrap();
    write(writer, signer, &original, "ack", json!({}), true)
        .await
        .unwrap();
    let query = read(reader)
        .await
        .expect("SDK must observe the accepted original");
    query_is_original(&query, &original);
    if stall {
        let mut query = Some(query);
        let mut observations = 0;
        while let Some(current) = query {
            query_is_original(&current, &original);
            observations += 1;
            if write(
                writer,
                signer,
                &current,
                "operation.updated",
                update(false),
                true,
            )
            .await
            .is_none()
                || write(writer, signer, &current, "ack", json!({}), true)
                    .await
                    .is_none()
            {
                break;
            }
            query = read(reader).await;
        }
        assert!(
            observations >= 2,
            "unchanged status must not renew the original deadline"
        );
        return;
    }
    let setup = json!({"kind":"native-prepared/1","revision":5,"offer_id":original.id(),"streams":[{
        "strm_id":"audio","kind":"audio","direction":"sendrecv","stream_local_id":9,
        "codec":{"name":"pcmu","params":{"payload_type":0,"sample_rate":8000,"channels":1}}}]});
    let offer = write(writer,signer,&original,"connection.offer",json!({
        "by_participant":"part_one","substrate":"quic",
        "capabilities":{"audio_codecs":[{"name":"pcmu","params":{"sample_rate":8000,"channels":1}}],"max_streams_per_connection":8,"identity_assurance_offered":"identified"},
        "streams_offered":[{"id":"audio","kind":"audio","direction":"sendrecv","codec_preferences":["pcmu"]}],
        "substrate_setup":setup}),false).await.unwrap();
    let answer = read(reader).await.unwrap();
    assert_eq!(answer.kind(), "connection.answer");
    assert_eq!(answer.scope(Scope::Reply), Some(offer.id()));
    assert_eq!(
        answer.scope(Scope::Operation),
        original.scope(Scope::Operation)
    );
    for scope in [Scope::Conversation, Scope::Session, Scope::Connection] {
        assert_eq!(answer.scope(scope), original.scope(scope));
    }
    assert_eq!(answer.original()["payload"]["substrate_setup"], setup);
    write(
        writer,
        signer,
        &query,
        "operation.updated",
        update(true),
        true,
    )
    .await
    .unwrap();
    connection
        .send_datagram(b"completed-without-ack".as_slice().into())
        .unwrap();
    tokio::select! {
        _ = release_ack.notified() => (),
        extra = read(reader) => panic!("command sent before the original final ACK: {:?}", extra.map(|frame| frame.kind().to_owned())),
        _ = connection.closed() => return,
    }
    write(writer, signer, &query, "ack", json!({}), true)
        .await
        .unwrap();
    let packet = connection.read_datagram().await.unwrap();
    let ObservedNativeMediaDatagram::Rtp(packet) = observe_native_media_datagram(&packet).unwrap()
    else {
        panic!("expected actual audio on the newly published handle")
    };
    assert_eq!(packet.stream_local_id, 9);
    connection
        .send_datagram(b"new-handle-verified".as_slice().into())
        .unwrap();
}
