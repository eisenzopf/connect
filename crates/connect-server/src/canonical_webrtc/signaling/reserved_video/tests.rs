//! Actual server validator entrypoints and native SDP codec validators. No
//! reservation/auth token is fabricated and no network/readiness credit given.
use super::*;
use rvoip_core::ids::StreamId;
fn profile() -> ReservedVideoProfile {
    ReservedVideoProfile::new(
        StreamId::from_string("audio"),
        StreamId::from_string("preview-a"),
        StreamId::from_string("public-b"),
    )
    .unwrap()
}
fn sdp(answer: bool) -> String {
    let direction = if answer { "sendonly" } else { "recvonly" };
    format!("v=0\r\no=- 1 1 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\na=group:BUNDLE au a b\r\nm=audio 9 UDP/TLS/RTP/SAVPF 0\r\na=mid:au\r\na=sendrecv\r\na=rtpmap:0 PCMU/8000\r\nm=video 9 UDP/TLS/RTP/SAVPF 120 124\r\na=mid:a\r\na=sendrecv\r\na=extmap:4 urn:ietf:params:rtp-hdrext:sdes:mid\r\na=rtpmap:120 VP8/90000\r\na=rtpmap:124 rtx/90000\r\na=fmtp:124 apt=120\r\nm=video 9 UDP/TLS/RTP/SAVPF 121 125\r\na=mid:b\r\na={direction}\r\na=extmap:4 urn:ietf:params:rtp-hdrext:sdes:mid\r\na=rtpmap:121 VP8/90000\r\na=rtpmap:125 rtx/90000\r\na=fmtp:125 apt=121\r\n")
}
fn offer() -> serde_json::Value {
    json!({"streams_offered":[{"id":"audio","kind":"audio","direction":"sendrecv","codec_preferences":["pcmu"]},{"id":"preview-a","kind":"video","direction":"sendrecv","codec_preferences":["vp8"]},{"id":"public-b","kind":"video","direction":"recvonly","codec_preferences":["vp8"]}],"substrate_setup":{"sdp":sdp(false)},"replacement_preparation":{"profile":"connect-replacement-media-v1","operation_record_id":"original-record"},"webrtc_video_receive_reservation":{"profile":"connect/webrtc-public-video-reservation/1","phase":"intent","reservation_id":"original-reservation","nonce":"original-intent-nonce","preview_stream_id":"preview-a","preview_role":"receive","public_stream_id":"public-b"}})
}
#[test]
fn reserved_server_answer_preserves_three_native_roles_distinct_mid_payloads_and_logical_ids() {
    let original = offer();
    validate_shape(&original, &profile()).unwrap();
    let answer = selected_shape("original-participant", &original, &sdp(true), &profile()).unwrap();
    let wire = serde_json::to_value(answer).unwrap();
    let rows = wire["streams_answered"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0]["id"], "audio");
    assert_eq!(rows[0]["direction"], "sendrecv");
    assert_eq!(rows[1]["id"], "preview-a");
    assert_eq!(rows[1]["direction"], "sendrecv");
    assert_eq!(rows[1]["codec"]["params"]["payload_type"], 120);
    assert_eq!(rows[2]["id"], "public-b");
    assert_eq!(rows[2]["direction"], "sendonly");
    assert_eq!(rows[2]["codec"]["params"]["payload_type"], 121);
    assert!(
        super::super::validate_offer(&original, true).is_err(),
        "an echo alone cannot select the explicit class on generic signaling"
    );
    assert!(
        super::super::selected_answer("original-participant", &original, &sdp(true), true).is_err()
    );
    for bad in [
        sdp(true).replace("a=mid:b", "a=mid:a"),
        sdp(true).replace("a=mid:b", "a=mid:foreign"),
        sdp(true).replace("a=sendonly", "a=sendrecv"),
        sdp(true).replace("a=fmtp:125 apt=121", "a=fmtp:125 apt=120"),
        sdp(true).replace("121 VP8/90000", "121 VP8/8000"),
        sdp(true) + "m=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\n",
    ] {
        assert!(selected_shape("original-participant", &original, &bad, &profile()).is_err());
    }
}
#[test]
fn reserved_server_rejects_foreign_or_private_slot_shapes_without_weakening_ordinary_profiles() {
    let original = offer();
    for (index, key, value) in [
        (1, "id", "foreign-a"),
        (1, "direction", "sendonly"),
        (2, "direction", "sendrecv"),
        (2, "kind", "screen"),
    ] {
        let mut bad = original.clone();
        bad["streams_offered"][index][key] = json!(value);
        assert!(validate_shape(&bad, &profile()).is_err());
    }
    let mut legacy = original.clone();
    legacy
        .as_object_mut()
        .unwrap()
        .remove("webrtc_video_receive_reservation");
    assert!(
        super::super::validate_offer(&legacy, true).is_err(),
        "three native mids are not a legacy singleton-video shape"
    );
    let mut wrong = original.clone();
    wrong["substrate_setup"]["sdp"] = json!(sdp(false).replace("a=recvonly", "a=sendrecv"));
    assert!(validate_shape(&wrong, &profile()).is_err());
    initial_bundle_only_server_vectors();
}

fn initial_bundle_only_server_vectors() {
    let mut original = offer();
    let initial = sdp(false).replace(
        "m=video 9 UDP/TLS/RTP/SAVPF 121 125\r\n",
        "m=video 0 UDP/TLS/RTP/SAVPF 121 125\r\na=bundle-only\r\n",
    );
    original["substrate_setup"]["sdp"] = json!(initial.clone());
    validate_shape(&original, &profile()).unwrap();
    let answer = selected_shape("original-participant", &original, &sdp(true), &profile()).unwrap();
    let selected = serde_json::to_value(answer).unwrap();
    assert_eq!(selected["streams_answered"][2]["id"], "public-b");
    assert_eq!(selected["streams_answered"][2]["direction"], "sendonly");
    assert_eq!(
        selected["streams_answered"][2]["codec"]["params"]["payload_type"],
        121
    );
    for bad in [
        initial.replace("a=bundle-only\r\n", ""),
        initial.replace("a=group:BUNDLE au a b", "a=group:BUNDLE au a"),
        initial.replace("a=group:BUNDLE au a b", "a=group:BUNDLE b au a"),
        initial.replace("m=audio 9 ", "m=audio 0 "),
        initial.replace("a=mid:b", "a=mid:a"),
        initial.replace("a=recvonly", "a=sendrecv"),
    ] {
        let mut refused = original.clone();
        refused["substrate_setup"]["sdp"] = json!(bad);
        assert!(validate_shape(&refused, &profile()).is_err());
    }
    for bad in [
        sdp(true).replace(
            "m=video 9 UDP/TLS/RTP/SAVPF 121 125",
            "m=video 0 UDP/TLS/RTP/SAVPF 121 125\r\na=bundle-only",
        ),
        sdp(true).replace("a=group:BUNDLE au a b", "a=group:BUNDLE au a"),
        sdp(true).replace("a=mid:b", "a=mid:foreign"),
        sdp(true).replace("a=fmtp:125 apt=121", "a=fmtp:125 apt=120"),
    ] {
        assert!(selected_shape("original-participant", &original, &bad, &profile()).is_err());
    }
}
