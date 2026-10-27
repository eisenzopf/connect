use rvoip_webrtc::media::core_video_profile::negotiated_profile;
fn sdp(video: &str) -> String {
    format!(
        "v=0\r\no=- 1 1 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\nc=IN IP4 127.0.0.1\r\nm=audio 9 UDP/TLS/RTP/SAVPF 0\r\na=mid:a\r\n{video}"
    )
}
fn video() -> &'static str {
    "m=video 9 UDP/TLS/RTP/SAVPF 120 121\r\na=mid:v\r\na=rtpmap:120 VP8/90000\r\na=rtpmap:121 rtx/90000\r\na=fmtp:121 apt=120\r\n"
}
#[test]
fn actual_dynamic_mapping_and_declined_rtx() {
    let offer = sdp(video());
    let n = negotiated_profile(&offer, &offer).unwrap();
    assert_eq!(
        (n.payload_type, n.rtx_payload_type, n.mid.as_deref()),
        (120, Some(121), Some("v"))
    );
    let answer = sdp(&video().replace("SAVPF 120 121", "SAVPF 120"));
    assert_eq!(
        negotiated_profile(&offer, &answer)
            .unwrap()
            .rtx_payload_type,
        None
    );
}
#[test]
fn answer_cannot_reassign_offered_pt_codec_clock_or_mid() {
    let offer = sdp(video());
    for answer in [
        offer.replace("120", "122"),
        offer.replace("VP8/90000", "H264/90000"),
        offer.replace("VP8/90000", "VP8/8000"),
        offer.replace("a=mid:v", "a=mid:other"),
    ] {
        assert!(negotiated_profile(&offer, &answer).is_err());
    }
}
#[test]
fn ambiguous_rejected_and_multiple_primary_video_refuse() {
    let one = sdp(video());
    for answer in [
        format!("{one}{}", video()),
        one.replace("m=video 9", "m=video 0"),
        one.replace("SAVPF 120 121", "SAVPF 120 120 121"),
        one.replace("rtx/90000", "VP8/90000"),
    ] {
        assert!(negotiated_profile(&one, &answer).is_err());
    }
}
#[test]
fn rtx_must_keep_exact_offered_apt() {
    let one = sdp(video());
    assert!(negotiated_profile(&one, &one.replace("apt=120", "apt=119")).is_err());
    assert!(negotiated_profile(&one.replace("apt=120", "apt=119"), &one).is_err());
    assert!(negotiated_profile(&one, &one.replace("a=fmtp:121 apt=120\r\n", "")).is_err());
}
#[test]
fn unsupported_packet_parameters_rid_and_simulcast_refuse() {
    let one = sdp(video());
    for extra in [
        "a=fmtp:120 max-fs=12288\r\n",
        "a=rid:f send\r\n",
        "a=simulcast:send f\r\n",
        "a=ssrc-group:SIM 10 20\r\n",
        "a=fmtp:121 apt=119\r\n",
        "a=rtpmap:120 VP8/90000\r\n",
    ] {
        let bad = format!("{one}{extra}");
        assert!(negotiated_profile(&bad, &bad).is_err(),"accepted {extra:?}");
    }
}
#[test]
fn mid_bounds_and_section_identity_refuse() {
    let one = sdp(video());
    for bad in [
        one.replace("a=mid:v", "a=mid:abcdefghijklmnopq"),
        one.replace("a=mid:v", "a=mid:v\r\na=mid:x"),
        one.replace("UDP/TLS/RTP/SAVPF 120", "RTP/AVP 120"),
    ] {
        assert!(negotiated_profile(&one, &bad).is_err());
    }
}
