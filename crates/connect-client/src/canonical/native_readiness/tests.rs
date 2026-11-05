use super::*;

const SOURCE_CONNECTION: &str = "source";
fn scope() -> CommandScope {
    CommandScope {
        conversation_id: Some("room".into()),
        session_id: Some("session".into()),
        connection_id: Some("connection".into()),
        operation_id: Some("operation".into()),
        expected_revision: Some(0),
    }
}
fn scoped(kind: &str, payload: Value, reply: Option<&str>) -> Envelope {
    let mut value = envelope(kind, payload);
    value["cid"] = json!("room");
    value["sid"] = json!("session");
    value["connid"] = json!("connection");
    value["operation_id"] = json!("operation");
    if let Some(reply) = reply {
        value["in_reply_to"] = json!(reply);
    }
    Envelope::from_value(value).unwrap()
}
fn original() -> Value {
    json!({"substrate":"quic","capabilities":{"audio_codecs":[{"name":"opus","params":{"sample_rate":48000,"channels":1}}],
        "video_codecs":[{"name":"vp8","params":{"sample_rate":90000,"channels":0}}],"video_feedback":"rtcp-pli-epoch/1"},
        "streams_offered":[{"id":"audio","kind":"audio","direction":"sendrecv","codec_preferences":["opus"]},
            {"id":"camera","kind":"video","direction":"sendrecv","codec_preferences":["vp8"]}],
        "replacement_preparation":{"profile":"connect-replacement-media-v1","operation_record_id":"record"}})
}
fn native_egress(handle: u16) -> Value {
    // Explicit negotiated parser-model identity, not live media evidence.
    json!({"profile":"native-sequence-cursor/1","physical_incarnation":format!("{handle:032x}"),
        "stream_generation":1,"connection_generation":2,"initial_outer_sequence":0})
}
fn pair(request: &str) -> (Value, Value) {
    let audio = json!({"strm_id":"audio","kind":"audio","direction":"sendrecv","stream_local_id":6,"native_egress":native_egress(6),
        "codec":{"name":"opus","params":{"sample_rate":48000,"channels":1,"payload_type":111}}});
    let video = json!({"strm_id":"camera","kind":"video","direction":"sendrecv","stream_local_id":7,"native_egress":native_egress(7),
        "codec":{"name":"vp8","params":{"sample_rate":90000,"channels":0,"payload_type":96}},
        "video_feedback":{"profile":"rtcp-pli-epoch/1","original_operation":"operation","original_offer":request,"stream_generation":1,"connection_generation":2}});
    let base = json!({"by_participant":"participant","substrate":"quic","capabilities":original()["capabilities"],
        "substrate_setup":{"kind":"native-prepared/1","revision":1,"offer_id":request,"streams":[audio.clone(),video.clone()]},
        "replacement_preparation":{"profile":"connect-replacement-media-v1","operation_record_id":"record",
            "readiness":{"nonce":"original-nonce","source_generation":1,"target_generation":2,"stream_revision":2}}});
    let mut answer = base.clone();
    answer["streams_answered"] = json!([
        {"id":"audio","kind":"audio","direction":"sendrecv","codec":audio["codec"]},
        {"id":"camera","kind":"video","direction":"sendrecv","codec":video["codec"]}]);
    let mut callback = base;
    callback["streams_offered"] = original()["streams_offered"].clone();
    (answer, callback)
}
fn setup() -> (Shared, Envelope, payloads::ConnectionAnswer) {
    let shared = Shared::default();
    shared
        .begin(
            "offer",
            &scope(),
            &original(),
            vec!["camera".into()],
            SOURCE_CONNECTION,
        )
        .unwrap();
    let (answer, callback) = pair("offer");
    shared
        .verified(&scoped("connection.answer", answer, Some("offer")))
        .unwrap();
    let callback = scoped("connection.offer", callback, None);
    shared.verified(&callback).unwrap();
    let expected = serde_json::from_value(
        shared
            .state
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .expected_answer
            .clone()
            .unwrap(),
    )
    .unwrap();
    (shared, callback, expected)
}
#[test]
fn challenge_requires_exact_verified_callback_and_its_completed_answer_write() {
    let (shared, callback, answer) = setup();
    assert!(shared.preparation().is_err());
    assert_eq!(
        serde_json::to_value(shared.answer(&callback).unwrap()).unwrap(),
        serde_json::to_value(&answer).unwrap()
    );
    let mut changed = callback.original().clone();
    changed["payload"]["replacement_preparation"]["readiness"]["nonce"] = json!("foreign");
    let changed = Envelope::from_value(changed).unwrap();
    assert!(shared.answer(&changed).is_err());
    assert!(shared.check_answer(&changed, &answer, &scope()).is_err());
    assert!(shared.check_answer(&callback, &answer, &scope()).unwrap());
    shared.confirmed().unwrap();
    let proof = shared.preparation().unwrap();
    assert_eq!(proof.audio().stream_local_id, 6);
    assert_eq!(proof.video().unwrap().stream_local_id, 7);
    assert_eq!(
        (
            proof.source_generation(),
            proof.target_generation(),
            proof.stream_revision()
        ),
        (1, 2, 2)
    );
}
#[test]
fn changed_nonce_or_generation_between_signed_legs_cannot_become_readiness() {
    for field in [
        "nonce",
        "source_generation",
        "target_generation",
        "stream_revision",
    ] {
        let shared = Shared::default();
        shared
            .begin(
                "offer",
                &scope(),
                &original(),
                vec!["camera".into()],
                SOURCE_CONNECTION,
            )
            .unwrap();
        let (answer, mut callback) = pair("offer");
        shared
            .verified(&scoped("connection.answer", answer, Some("offer")))
            .unwrap();
        callback["replacement_preparation"]["readiness"][field] = if field == "nonce" {
            json!("changed")
        } else {
            json!(99)
        };
        assert!(
            shared
                .verified(&scoped("connection.offer", callback, None))
                .is_err(),
            "{field}"
        );
    }
}
#[test]
fn exact_frozen_ready_replays_after_uncertain_capture_but_changed_observation_refuses() {
    let (shared, callback, answer) = setup();
    assert!(shared.check_answer(&callback, &answer, &scope()).unwrap());
    shared.confirmed().unwrap();
    let prepared = shared.preparation().unwrap();
    let video = NativeDecodedVideoReadiness {
        decoded_frames: 1,
        decoded_keyframes: 1,
    };
    let ready = prepared.with_decoded(2880, Some(video)).unwrap();
    shared.admit(&ready).unwrap();
    // A lost post-capture reply does not change this original client intent.
    shared.admit(&ready.clone()).unwrap();
    assert!(shared
        .admit(&prepared.with_decoded(3840, Some(video)).unwrap())
        .is_err());
    let mut changed = ready.clone();
    changed.payload["replacement_preparation"]["readiness"]["nonce"] = json!("another");
    assert!(shared.admit(&changed).is_err());
    let mut normalized = ready.payload.clone();
    normalized["replacement_preparation"]["profile"] = json!("connect-replacement-media-v1");
    normalized["replacement_preparation"]
        .as_object_mut()
        .unwrap()
        .remove("readiness");
    assert_eq!(
        normalized,
        original(),
        "ready cannot change SDP, streams, codecs or the original record"
    );
}
#[test]
fn missing_real_decode_or_current_physical_end_cannot_sign_ready() {
    let (shared, callback, answer) = setup();
    shared.check_answer(&callback, &answer, &scope()).unwrap();
    shared.confirmed().unwrap();
    let prepared = shared.preparation().unwrap();
    let v = NativeDecodedVideoReadiness {
        decoded_frames: 1,
        decoded_keyframes: 1,
    };
    assert!(prepared.with_decoded(0, Some(v)).is_err());
    assert!(prepared.with_decoded(2880, None).is_err());
    assert!(prepared
        .with_decoded(
            2880,
            Some(NativeDecodedVideoReadiness {
                decoded_frames: 1,
                decoded_keyframes: 0
            })
        )
        .is_err());
    let ready = prepared.with_decoded(2880, Some(v)).unwrap();
    shared
        .outgoing(
            &json!({"type":"connection.end","cid":"room","sid":"session","connid":"connection"}),
        )
        .unwrap();
    assert!(shared.admit(&ready).is_err());
    assert!(shared.preparation().is_err());
}
#[test]
fn another_original_media_request_cannot_reuse_opaque_readiness() {
    let (shared, callback, answer) = setup();
    shared.check_answer(&callback, &answer, &scope()).unwrap();
    shared.confirmed().unwrap();
    let ready = shared
        .preparation()
        .unwrap()
        .with_decoded(
            2880,
            Some(NativeDecodedVideoReadiness {
                decoded_frames: 1,
                decoded_keyframes: 1,
            }),
        )
        .unwrap();
    let other = Shared::default();
    other
        .begin(
            "another-offer",
            &scope(),
            &original(),
            vec!["camera".into()],
            SOURCE_CONNECTION,
        )
        .unwrap();
    let (answer, callback) = pair("another-offer");
    other
        .verified(&scoped("connection.answer", answer, Some("another-offer")))
        .unwrap();
    other
        .verified(&scoped("connection.offer", callback, None))
        .unwrap();
    other.confirmed().unwrap();
    assert!(other.admit(&ready).is_err());
}

#[test]
fn absent_challenge_is_not_a_downgraded_native_replacement_profile() {
    let shared = Shared::default();
    shared
        .begin(
            "offer",
            &scope(),
            &original(),
            vec!["camera".into()],
            SOURCE_CONNECTION,
        )
        .unwrap();
    let (mut answer, _) = pair("offer");
    answer
        .as_object_mut()
        .unwrap()
        .remove("replacement_preparation");
    assert!(shared
        .verified(&scoped("connection.answer", answer, Some("offer")))
        .is_err());
    assert!(shared.preparation().is_err());
}

#[test]
fn omitted_video_transfer_keeps_selected_video_muted_and_requires_only_audio() {
    let shared = Shared::default();
    shared
        .begin("offer", &scope(), &original(), vec![], SOURCE_CONNECTION)
        .unwrap();
    let (answer, callback) = pair("offer");
    shared
        .verified(&scoped("connection.answer", answer, Some("offer")))
        .unwrap();
    let callback = scoped("connection.offer", callback, None);
    shared.verified(&callback).unwrap();
    let answer = shared.answer(&callback).unwrap();
    shared.check_answer(&callback, &answer, &scope()).unwrap();
    shared.confirmed().unwrap();
    let prepared = shared.preparation().unwrap();
    assert!(prepared.video().is_none());
    let ready = prepared.with_decoded(2880, None).unwrap();
    assert!(ready.payload["replacement_preparation"]["readiness"]["video"].is_null());
    shared.admit(&ready).unwrap();
}
#[test]
fn explicit_video_transfer_must_resolve_the_original_selected_receiver() {
    let shared = Shared::default();
    shared
        .begin(
            "offer",
            &scope(),
            &original(),
            vec!["another-camera".into()],
            SOURCE_CONNECTION,
        )
        .unwrap();
    let (answer, callback) = pair("offer");
    shared
        .verified(&scoped("connection.answer", answer, Some("offer")))
        .unwrap();
    shared
        .verified(&scoped("connection.offer", callback, None))
        .unwrap();
    shared.confirmed().unwrap();
    assert!(shared.preparation().is_err());
}
