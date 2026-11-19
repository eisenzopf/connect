//! Actual Gate selection constructor and observation parser. Native/Store read
//! provenance is tested by its partner; no authority/read/ready is fabricated.
use super::*;
use rvoip_core::{
    ids::StreamId,
    stream_lifecycle::{DesiredStream, MediaFormat, NegotiatedStream},
};
use rvoip_webrtc::canonical::ReservedVideoProfile;
fn selected() -> Vec<NegotiatedStream> {
    [
        ("audio", Medium::Audio, StreamDirection::Both),
        ("preview-a", Medium::Video, StreamDirection::Both),
        ("public-b", Medium::Video, StreamDirection::Send),
    ]
    .into_iter()
    .map(|(id, medium, direction)| NegotiatedStream {
        requested: DesiredStream {
            logical_id: StreamId::from_string(id),
            medium,
            direction,
        },
        format: MediaFormat::Encoded {
            name: (if medium == Medium::Audio {
                "pcmu"
            } else {
                "vp8"
            })
            .into(),
            clock_rate_hz: if medium == Medium::Audio { 8000 } else { 90000 },
            channels: if medium == Medium::Audio { 1 } else { 0 },
            fmtp: None,
        },
    })
    .collect()
}
fn profile() -> ReservedVideoProfile {
    ReservedVideoProfile::new(
        StreamId::from_string("audio"),
        StreamId::from_string("preview-a"),
        StreamId::from_string("public-b"),
    )
    .unwrap()
}
#[test]
fn reserved_private_gate_uses_actual_constructor_selects_audio_preview_a_and_rejects_public_b_evidence(
) {
    let streams = selected();
    let until = tokio::time::Instant::now() + Duration::from_secs(1);
    assert!(
        ReadyGate::from_selection(1, 2, &streams, until, None, false).is_err(),
        "legacy singleton private gate must not silently count both A and B"
    );
    let gate = ReadyGate::from_reserved_selection(1, 2, &streams, until, &profile()).unwrap();
    assert_eq!(gate.audio, "audio");
    assert_eq!(gate.video.as_deref(), Some("preview-a"));
    let observation = serde_json::json!({"nonce":gate.nonce,"source_generation":1,"target_generation":2,"stream_revision":2,"audio":{"stream_id":"audio","decoded_samples":160},"video":{"stream_id":"preview-a","decoded_frames":1,"decoded_keyframes":1,"decoder_startup":"fresh-peer-first-decoded-frame"}});
    assert!(gate.parse(&observation).is_ok());
    assert!(
        !gate.observed(),
        "parsing alone grants no decoded readiness"
    );
    let mut b = observation.clone();
    b["video"]["stream_id"] = serde_json::json!("public-b");
    assert!(gate.parse(&b).is_err());
    let mut missing = observation.clone();
    missing.as_object_mut().unwrap().remove("video");
    assert!(gate.parse(&missing).is_err());
    let mut wrong_audio = observation;
    wrong_audio["audio"]["stream_id"] = serde_json::json!("public-b");
    assert!(gate.parse(&wrong_audio).is_err());
    for mode in 0..3 {
        let mut bad = streams.clone();
        match mode {
            0 => bad[1].requested.direction = StreamDirection::Receive,
            1 => bad[2].requested.direction = StreamDirection::Both,
            _ => bad[0].requested.logical_id = StreamId::from_string("foreign-audio"),
        };
        assert!(ReadyGate::from_reserved_selection(1, 2, &bad, until, &profile()).is_err());
    }
    assert!(ReadyGate::from_reserved_selection(1, 3, &streams, until, &profile()).is_err());
    let audio_only = &streams[..1];
    assert!(
        ReadyGate::from_selection(1, 2, audio_only, until, None, false).is_ok(),
        "ordinary audio gate unchanged"
    );
}
