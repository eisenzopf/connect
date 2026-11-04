use super::*;

fn ready(generation: u64) -> Shared {
    let s = Shared::default();
    s.begin("offer".into(), scope(), original()).unwrap();
    let (mut answer, mut callback) = pair("offer");
    for p in [&mut answer, &mut callback] {
        p["substrate_setup"]["streams"][0]["video_feedback"]["connection_generation"] =
            json!(generation);
    }
    s.verified(&scoped("connection.answer", answer, Some("offer")))
        .unwrap();
    s.verified(&scoped("connection.offer", callback, None))
        .unwrap();
    s.confirmed().unwrap();
    for publisher in [false, true] {
        s.verified(&history(1, publisher, generation, false))
            .unwrap();
    }
    s
}
fn history(revision: u64, publisher: bool, generation: u64, changed: bool) -> Envelope {
    let mut v = epoch("offer", revision, publisher).original().clone();
    v["payload"]["connection_generation"] = json!(generation);
    if changed {
        v["payload"]["original_operation"] = json!("codec");
        v["payload"]["original_offer"] = json!("codec-request");
        v["payload"]["stream_generation"] = json!(2);
        v["payload"]["stream_local_id"] = json!(9);
    }
    Envelope::from_value(v).unwrap()
}
fn command() -> Value {
    let mut v = scoped(
        "connection.update",
        json!({"action":"codec-renegotiate","streams":["camera"],"codec_preferences":["vp8"]}),
        None,
    )
    .original()
    .clone();
    v["id"] = json!("codec-request");
    v["operation_id"] = json!("codec");
    v["expected_revision"] = json!(4);
    v
}
fn root() -> CommandScope {
    CommandScope {
        operation_id: Some("codec".into()),
        expected_revision: Some(4),
        ..scope()
    }
}
fn callback(generation: u64, changed: bool) -> Envelope {
    let (_, mut p) = pair("offer");
    p["substrate_setup"]["offer_id"] = json!("codec-request");
    p["substrate_setup"]["revision"] = json!(5);
    let stream = &mut p["substrate_setup"]["streams"][0];
    stream["video_feedback"]["connection_generation"] = json!(generation);
    if changed {
        stream["stream_local_id"] = json!(9);
        stream["video_feedback"]["original_operation"] = json!("codec");
        stream["video_feedback"]["original_offer"] = json!("codec-request");
        stream["video_feedback"]["stream_generation"] = json!(2);
    }
    let mut v = scoped("connection.offer", p, None).original().clone();
    v["operation_id"] = json!("codec");
    Envelope::from_value(v).unwrap()
}
fn answer(offer: &Envelope) -> payloads::ConnectionAnswer {
    let p = &offer.original()["payload"];
    let b = &p["substrate_setup"]["streams"][0];
    serde_json::from_value(json!({"by_participant":p["by_participant"],"substrate":"quic","capabilities":original()["capabilities"],"streams_answered":[{"id":"camera","kind":"video","direction":"sendrecv","codec":b["codec"]}],"substrate_setup":p["substrate_setup"]})).unwrap()
}
fn completed(generation: u64, changed: bool) -> Value {
    let callback = callback(generation, changed);
    let wire = &callback.original()["payload"]["substrate_setup"]["streams"][0];
    let binding = json!({"stream_id":"camera","medium":"video","direction":"sendrecv","codec":wire["codec"],"stream_local_id":wire["stream_local_id"],"stream_generation":if changed{2}else{1},"connection_generation":generation});
    let mut v=scoped("operation.updated",json!({"operation_id":"codec","command_type":"connection.update","expected_revision":4,"state":"completed","revision":2,"result":{"connection_id":"connection","stream_revision":5,"resources_changed":true,"codec_bindings":[binding]}}),Some("codec-request")).original().clone();
    v["operation_id"] = json!("codec");
    v
}
fn deliver(s: &Shared, v: Value) -> Result<bool> {
    s.verified(&Envelope::from_value(v).unwrap())
}
fn active(s: &Shared) -> usize {
    s.state.lock().unwrap().as_ref().unwrap().active.len()
}

#[test]
fn replacement_video_codec_adoption_pairs_write_result_and_new_allocation_for_each_generation() {
    for generation in [1, 2, 3] {
        let s = ready(generation);
        s.outgoing(&command()).unwrap();
        let offer = callback(generation, true);
        s.verified(&offer).unwrap();
        s.verified(&history(2, true, generation, true)).unwrap();
        deliver(&s, completed(generation, true)).unwrap();
        assert_eq!(active(&s), 0, "receipt alone cannot adopt");
        let answer = answer(&offer);
        assert!(s.check_confirmation(&offer, &answer, &root()).unwrap());
        s.confirmed().unwrap();
        let g = s.state.lock().unwrap();
        let state = g.as_ref().unwrap();
        assert!(!state.suspended);
        assert_eq!(
            state.selected.as_ref().unwrap().streams[0].stream_local_id,
            Some(9)
        );
        assert_eq!(
            state
                .selected
                .as_ref()
                .unwrap()
                .video_feedback
                .values()
                .next()
                .unwrap()
                .connection_generation,
            generation
        );
        assert_eq!(state.active.len(), 1);
        assert!(state.active.contains_key(&(9, true)));
        drop(g);
        s.verified(&history(3, false, generation, false)).unwrap();
        assert_eq!(active(&s), 1, "retired allocation is refused");
        s.outgoing(&command()).unwrap();
        deliver(&s, completed(generation, true)).unwrap();
        assert_eq!(active(&s), 1, "logical replay does not suspend again");
    }
}

#[test]
fn replacement_video_codec_retained_resources_require_paired_receipt_and_preserve_unsubscribe() {
    let s = ready(2);
    s.outgoing(&command()).unwrap();
    let offer = callback(2, false);
    s.verified(&offer).unwrap();
    assert!(s
        .check_confirmation(&offer, &answer(&offer), &root())
        .unwrap());
    s.confirmed().unwrap();
    assert_eq!(active(&s), 0, "write alone cannot adopt");
    s.outgoing(scoped("stream.unsubscribe", json!({"strm_ids":["camera"]}), None).original())
        .unwrap();
    deliver(&s, completed(2, false)).unwrap();
    assert_eq!(active(&s), 1);
    let g = s.state.lock().unwrap();
    let state = g.as_ref().unwrap();
    assert!(state.active.contains_key(&(7, true)));
    assert_eq!(
        state.selected.as_ref().unwrap().streams[0].stream_local_id,
        Some(7)
    );
    assert_eq!(
        state
            .selected
            .as_ref()
            .unwrap()
            .video_feedback
            .values()
            .next()
            .unwrap()
            .original_offer,
        "offer"
    );
}

#[test]
fn replacement_video_codec_rejects_altered_offer_confirmation_receipt_and_handle_reuse() {
    for field in [
        "connection_generation",
        "stream_generation",
        "original_operation",
    ] {
        let s = ready(2);
        s.outgoing(&command()).unwrap();
        let mut v = callback(2, true).original().clone();
        v["payload"]["substrate_setup"]["streams"][0]["video_feedback"][field] =
            if field == "original_operation" {
                json!("foreign")
            } else {
                json!(99)
            };
        assert!(deliver(&s, v).is_err(), "{field}");
        assert_eq!(active(&s), 0);
    }
    let s = ready(2);
    s.outgoing(&command()).unwrap();
    let offer = callback(2, true);
    s.verified(&offer).unwrap();
    let answer = answer(&offer);
    let mut altered = offer.original().clone();
    altered["id"] = json!("other-callback");
    assert!(s
        .check_confirmation(&Envelope::from_value(altered).unwrap(), &answer, &root())
        .is_err());
    let mut v = completed(2, true);
    v["payload"]["result"]["codec_bindings"][0]["stream_local_id"] = json!(7);
    assert!(deliver(&s, v).is_err());
    let mut v = completed(2, true);
    v["payload"]["result"]["stream_revision"] = json!(6);
    assert!(deliver(&s, v).is_err());
    assert_eq!(active(&s), 0);
    assert!(s.check_confirmation(&offer, &answer, &root()).unwrap());
    s.confirmed().unwrap();
    deliver(&s, completed(2, true)).unwrap();
    let mut next = command();
    next["id"] = json!("next-request");
    next["operation_id"] = json!("next");
    next["expected_revision"] = json!(5);
    s.outgoing(&next).unwrap();
    let mut v = callback(2, true).original().clone();
    v["operation_id"] = json!("next");
    v["payload"]["substrate_setup"]["revision"] = json!(6);
    v["payload"]["substrate_setup"]["offer_id"] = json!("next-request");
    let b = &mut v["payload"]["substrate_setup"]["streams"][0];
    b["stream_local_id"] = json!(7);
    b["video_feedback"]["original_operation"] = json!("next");
    b["video_feedback"]["original_offer"] = json!("next-request");
    b["video_feedback"]["stream_generation"] = json!(3);
    assert!(
        deliver(&s, v).is_err(),
        "retired hop handle cannot be reused"
    );
}

#[test]
fn replacement_video_codec_uncertainty_failure_and_retirement_never_install_candidate_resources() {
    let s = ready(2);
    s.outgoing(&command()).unwrap();
    s.verified(&callback(2, true)).unwrap();
    s.verified(&history(2, true, 2, true)).unwrap();
    let mut unknown = completed(2, true);
    unknown["payload"]["state"] = json!("unknown");
    unknown["payload"].as_object_mut().unwrap().remove("result");
    deliver(&s, unknown).unwrap();
    assert_eq!(active(&s), 0);
    let mut failed = completed(2, true);
    failed["payload"]["state"] = json!("failed");
    failed["payload"].as_object_mut().unwrap().remove("result");
    deliver(&s, failed).unwrap();
    let g = s.state.lock().unwrap();
    let state = g.as_ref().unwrap();
    assert!(!state.suspended);
    assert_eq!(
        state.selected.as_ref().unwrap().streams[0].stream_local_id,
        Some(7)
    );
    assert!(state.active.keys().all(|k| k.0 == 7));
    drop(g);
    deliver(&s, completed(2, true)).unwrap();
    assert_eq!(
        s.state
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .selected
            .as_ref()
            .unwrap()
            .streams[0]
            .stream_local_id,
        Some(7)
    );
    let s = ready(2);
    s.outgoing(&command()).unwrap();
    s.verified(&callback(2, true)).unwrap();
    s.confirmed().unwrap();
    s.outgoing(
        scoped(
            "connection.end",
            json!({"reason_code":200,"reason":"done"}),
            None,
        )
        .original(),
    )
    .unwrap();
    deliver(&s, completed(2, true)).unwrap();
    assert_eq!(active(&s), 0);
}
