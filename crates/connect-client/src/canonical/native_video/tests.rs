use super::*;
pub(super) fn scope() -> CommandScope {
    CommandScope {
        conversation_id: Some("room".into()),
        session_id: Some("session".into()),
        connection_id: Some("connection".into()),
        operation_id: Some("operation".into()),
        expected_revision: Some(0),
    }
}
pub(super) fn original() -> Value {
    json!({"substrate":"quic","capabilities":{"video_codecs":[{"name":"vp8","params":{"sample_rate":90000,"channels":0}}],"video_feedback":"rtcp-pli-epoch/1"},"streams_offered":[{"id":"camera","kind":"video","direction":"sendrecv","codec_preferences":["vp8"]}]})
}
pub(super) fn pair(request: &str) -> (Value, Value) {
    let resource = json!({"strm_id":"camera","kind":"video","direction":"sendrecv","stream_local_id":7,"codec":{"name":"vp8","params":{"sample_rate":90000,"channels":0,"payload_type":96}},"video_feedback":{"profile":"rtcp-pli-epoch/1","original_operation":"operation","original_offer":request,"stream_generation":1,"connection_generation":1}});
    let p = json!({"by_participant":"participant","substrate":"quic","capabilities":original()["capabilities"],"substrate_setup":{"kind":"native-prepared/1","revision":1,"offer_id":request,"streams":[resource.clone()]}});
    let mut answer = p.clone();
    answer["streams_answered"] =
        json!([{"id":"camera","kind":"video","direction":"sendrecv","codec":resource["codec"]}]);
    let mut callback = p;
    callback["streams_offered"] =
        json!([{"id":"camera","kind":"video","direction":"sendrecv","codec_preferences":["vp8"]}]);
    (answer, callback)
}
pub(super) fn scoped(kind: &str, payload: Value, reply: Option<&str>) -> Envelope {
    let mut v = envelope(kind, payload);
    v["cid"] = json!("room");
    v["sid"] = json!("session");
    v["connid"] = json!("connection");
    v["operation_id"] = json!("operation");
    if matches!(
        kind,
        "stream.subscribe" | "stream.unsubscribe" | "connection.update"
    ) {
        v["expected_revision"] = json!(1);
    }
    if let Some(reply) = reply {
        v["in_reply_to"] = json!(reply);
    }
    Envelope::from_value(v).unwrap()
}
pub(super) fn epoch(request: &str, revision: u64, publisher: bool) -> Envelope {
    let mut v =
        crate::sdk::video_feedback::epochs::tests::notice_value(revision.into(), revision, 0, 0);
    let fresh = envelope("ack", json!({}));
    v["ts"] = fresh["ts"].clone();
    v["id"] = fresh["id"].clone();
    v["payload"]["original_offer"] = json!(request);
    v["payload"]["direction"] = json!(if publisher { "publisher" } else { "receiver" });
    Envelope::from_value(v).unwrap()
}
pub(super) fn stage(shared: &Shared, request: &str) -> (Envelope, payloads::ConnectionAnswer) {
    shared.begin(request.into(), scope(), original()).unwrap();
    let (answer, callback) = pair(request);
    shared
        .verified(&scoped("connection.answer", answer, Some(request)))
        .unwrap();
    let callback = scoped("connection.offer", callback, None);
    shared.verified(&callback).unwrap();
    let guard = shared.state.lock().unwrap();
    let s = guard.as_ref().unwrap();
    let confirmation = protocol(s.negotiating.confirmation(&callback)).unwrap();
    (callback, confirmation)
}
#[test]
fn replacement_video_seal_requires_actual_callback_original_answer_and_completed_write() {
    let shared = Shared::default();
    let (callback, answer) = stage(&shared, "offer");
    shared.verified(&epoch("offer", 1, false)).unwrap();
    assert!(shared
        .state
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .active
        .is_empty());
    let mut forged = callback.original().clone();
    forged["payload"]["substrate_setup"]["streams"][0]["stream_local_id"] = json!(9);
    assert!(shared
        .check_confirmation(&Envelope::from_value(forged).unwrap(), &answer, &scope())
        .is_err());
    let mut changed = serde_json::to_value(&answer).unwrap();
    changed["substrate_setup"]["streams"][0]["stream_local_id"] = json!(9);
    assert!(shared
        .check_confirmation(
            &callback,
            &serde_json::from_value(changed).unwrap(),
            &scope()
        )
        .is_err());
    let foreign = Shared::default();
    assert!(!foreign
        .check_confirmation(&callback, &answer, &scope())
        .unwrap());
    assert!(foreign.confirmed().is_err());
    assert!(shared
        .check_confirmation(&callback, &answer, &scope())
        .unwrap());
    shared.confirmed().unwrap();
    let guard = shared.state.lock().unwrap();
    let s = guard.as_ref().unwrap();
    assert_eq!(s.active.len(), 1);
    assert_eq!(s.selected.as_ref().unwrap().video_feedback.len(), 1);
    drop(guard);
    assert!(shared.confirmed().is_err());
    assert!(shared.begin("other".into(), scope(), original()).is_err());
}
#[test]
fn replacement_video_signed_history_cannot_replay_after_unsubscribe_gate_or_retirement() {
    let shared = Shared::default();
    let (_, _) = stage(&shared, "offer");
    shared.confirmed().unwrap();
    for publisher in [false, true] {
        shared.verified(&epoch("offer", 1, publisher)).unwrap();
    }
    let unselect = scoped("stream.unsubscribe", json!({"strm_ids":["camera"]}), None);
    shared.outgoing(unselect.original()).unwrap();
    shared.verified(&epoch("offer", 1, false)).unwrap();
    assert_eq!(
        shared.state.lock().unwrap().as_ref().unwrap().active.len(),
        1
    );
    shared.verified(&epoch("other", 2, false)).unwrap();
    assert_eq!(
        shared.state.lock().unwrap().as_ref().unwrap().active.len(),
        1
    );
    shared.verified(&epoch("offer", 2, false)).unwrap();
    assert_eq!(
        shared.state.lock().unwrap().as_ref().unwrap().active.len(),
        2
    );
    shared
        .outgoing(
            scoped(
                "connection.update",
                json!({"action":"hold","details":{"direction":"inactive"}}),
                None,
            )
            .original(),
        )
        .unwrap();
    shared.verified(&epoch("offer", 3, true)).unwrap();
    assert!(shared
        .state
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .active
        .is_empty());
    shared
        .verified(&scoped(
            "connection.end",
            json!({"reason_code":200,"reason":"done"}),
            None,
        ))
        .unwrap();
    shared.verified(&epoch("offer", 4, false)).unwrap();
    let guard = shared.state.lock().unwrap();
    let s = guard.as_ref().unwrap();
    assert!(s.ended && s.active.is_empty());
}
// Actual signed QUIC verifies that the owned background reader installs notices
// before foreground control dequeues them. No fake Envelope can enter this path.
pub(crate) async fn emit(
    request: Envelope,
    writer: &mut quinn::SendStream,
    reader: &mut quinn::RecvStream,
    signer: &Sig9421Signer,
    invalid: bool,
) {
    let send = async |value: Value, writer: &mut quinn::SendStream| {
        let mut signed = signer.sign(&value).unwrap();
        if invalid && value["type"] == "stream.epoch" {
            signed["payload"]["ssrc"] = json!(999);
        }
        let raw = serde_json::to_vec(&signed).unwrap();
        writer
            .write_all(&(raw.len() as u32).to_be_bytes())
            .await
            .unwrap();
        writer.write_all(&raw).await.unwrap();
    };
    let (answer, callback) = pair(request.id());
    send(
        scoped("connection.answer", answer, Some(request.id()))
            .original()
            .clone(),
        writer,
    )
    .await;
    let callback = scoped("connection.offer", callback, None);
    send(callback.original().clone(), writer).await;
    let mut prefix = [0; 4];
    if reader.read_exact(&mut prefix).await.is_err() {
        return;
    }
    let mut raw = vec![0; u32::from_be_bytes(prefix) as usize];
    reader.read_exact(&mut raw).await.unwrap();
    let reply = Envelope::parse(&raw).unwrap();
    assert_eq!(reply.kind(), "connection.answer");
    assert_eq!(reply.scope(Scope::Reply), Some(callback.id()));
    send(epoch(request.id(), 1, true).original().clone(), writer).await;
}
#[tokio::test]
async fn replacement_video_owner_uses_existing_signature_reader_and_refuses_tampered_notices() {
    use crate::canonical::tests::{controlled, Fault};
    for invalid in [false, true] {
        let (mut client, server, _) = controlled(Fault::NativeVideoOwner(invalid)).await;
        client.start_signaling_reader().unwrap();
        let request = command_value("connection.offer", original(), scope()).unwrap();
        let id = request["id"].as_str().unwrap().to_owned();
        client
            .native_video
            .begin(id.clone(), scope(), original())
            .unwrap();
        client.write(request, true).await.unwrap();
        let answer = client.receive().await.unwrap();
        assert_eq!(answer.kind(), "connection.answer");
        let callback = client.receive().await.unwrap();
        assert_eq!(callback.kind(), "connection.offer");
        let expected = {
            let g = client.native_video.state.lock().unwrap();
            protocol(g.as_ref().unwrap().negotiating.confirmation(&callback)).unwrap()
        };
        client
            .answer_connection_offer(&callback, expected, &scope())
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let active = client
                    .native_video
                    .state
                    .lock()
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .active
                    .len();
                if (invalid && client.closed.load(Ordering::Acquire)) || (!invalid && active == 1) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let guard = client.native_video.state.lock().unwrap();
        assert_eq!(guard.as_ref().unwrap().active.len(), usize::from(!invalid));
        drop(guard);
        if !invalid {
            assert!(
                tokio::time::timeout(Duration::from_millis(5), client.receive())
                    .await
                    .is_err()
            );
            client.check_current().unwrap();
        }
        client.shutdown().await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
}

#[test]
fn replacement_video_seals_signed_generation_two_and_later_without_weakening_initial_negotiation() {
    for generation in [2, 3, 9_007_199_254_740_991_u64] {
        let shared = Shared::default();
        shared.begin("offer".into(), scope(), original()).unwrap();
        let (mut answer, mut callback) = pair("offer");
        for payload in [&mut answer, &mut callback] {
            payload["substrate_setup"]["streams"][0]["video_feedback"]["connection_generation"] =
                json!(generation);
        }
        let answer = scoped("connection.answer", answer, Some("offer"));
        let mut ordinary = Negotiating::new(original(), scope());
        assert!(ordinary.answer(&answer, "offer").is_err());
        shared.verified(&answer).unwrap();
        let mut different = callback.clone();
        different["substrate_setup"]["streams"][0]["video_feedback"]["connection_generation"] =
            json!(generation - 1);
        assert!(shared
            .verified(&scoped("connection.offer", different, None))
            .is_err());
        let callback = scoped("connection.offer", callback, None);
        shared.verified(&callback).unwrap();
        shared.confirmed().unwrap();
        // An old generation-one notice cannot seed the replacement history.
        shared.verified(&epoch("offer", 1, false)).unwrap();
        assert!(shared
            .state
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .active
            .is_empty());
        let mut notice = epoch("offer", 2, false).original().clone();
        notice["payload"]["connection_generation"] = json!(generation);
        shared
            .verified(&Envelope::from_value(notice).unwrap())
            .unwrap();
        assert_eq!(
            shared.state.lock().unwrap().as_ref().unwrap().active.len(),
            1
        );
    }
    for invalid in [json!(0), json!(9_007_199_254_740_992_u64), json!("2")] {
        let shared = Shared::default();
        shared.begin("offer".into(), scope(), original()).unwrap();
        let (mut answer, _) = pair("offer");
        answer["substrate_setup"]["streams"][0]["video_feedback"]["connection_generation"] =
            invalid;
        assert!(shared
            .verified(&scoped("connection.answer", answer, Some("offer")))
            .is_err());
    }
}

mod controls;

mod codecs;
