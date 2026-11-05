use super::*;
fn selected(name: &str, rate: u32, payload_type: u8) -> Value {
    json!({"name":name,"params":{"sample_rate":rate,"channels":1,"payload_type":payload_type}})
}
#[test]
fn actual_g711_companding_vectors_and_received_samples_are_decoded() {
    assert_eq!(
        [
            audio::mu(0xff),
            audio::mu(0x7f),
            audio::mu(0x80),
            audio::mu(0x00)
        ],
        [0, 0, 32124, -32124]
    );
    assert_eq!(
        [
            audio::alaw(0xd5),
            audio::alaw(0x55),
            audio::alaw(0xaa),
            audio::alaw(0x2a)
        ],
        [8, -8, 32256, -32256]
    );
    for (name, pt) in [("pcmu", 0), ("pcma", 8)] {
        let mut decoder = audio::Decoder::selected(&selected(name, 8000, pt)).unwrap();
        let bytes = decoder.encoded_silence().unwrap();
        for index in 0..3 {
            decoder
                .decode(7, index, u32::from(index) * 160, &bytes)
                .unwrap();
        }
        assert_eq!(decoder.samples(), 480);
        assert!(decoder.usable(), "actual decoded silence is usable PCM");
    }
}
#[test]
fn original_audio_duplicates_loss_and_source_change_cannot_reuse_readiness() {
    let mut decoder = audio::Decoder::selected(&selected("pcmu", 8000, 0)).unwrap();
    let bytes = [0x80; 160];
    decoder.decode(7, 10, 1600, &bytes).unwrap();
    decoder.decode(7, 10, 1600, &bytes).unwrap();
    decoder.decode(7, 9, 1440, &bytes).unwrap();
    assert_eq!(decoder.samples(), 160);
    decoder.decode(7, 12, 1920, &bytes).unwrap();
    assert_eq!(
        decoder.samples(),
        160,
        "a loss starts fresh actual consecutive samples"
    );
    decoder.decode(7, 13, 2080, &bytes).unwrap();
    decoder.decode(8, 14, 2240, &bytes).unwrap();
    assert_eq!(
        decoder.samples(),
        160,
        "another real SSRC cannot inherit decoded readiness"
    );
    assert!(!decoder.usable());
    assert!(decoder.decode(8, 15, 2400, &[]).is_err());
    assert!(decoder.decode(8, 15, 2400, &[0xff; 159]).is_err());
    assert_eq!(decoder.samples(), 160);
}
#[test]
fn actual_opus_output_is_required_and_absent_input_cannot_conceal() {
    let mut decoder = audio::Decoder::selected(&selected("opus", 48000, 111)).unwrap();
    let bytes = decoder.encoded_silence().unwrap();
    for index in 0..3 {
        decoder
            .decode(9, index, u32::from(index) * 960, &bytes)
            .unwrap();
    }
    assert_eq!(decoder.samples(), 2880);
    assert!(decoder.usable());
    assert!(decoder.decode(9, 3, 2880, &[]).is_err());
    assert!(decoder.decode(9, 3, 2880, &[0xff]).is_err());
    assert_eq!(decoder.samples(), 2880);
    assert!(audio::Decoder::selected(&selected("opus", 8000, 111)).is_err());
    assert!(audio::Decoder::selected(&selected("pcmu", 8000, 111)).is_err());
}
#[tokio::test]
async fn actual_received_keyframe_decodes_and_corrupt_output_is_not_readiness() {
    let frame = connect_client::NativeVideoFrame {
        timestamp_rtp: 99000,
        keyframe: true,
        encoded: include_bytes!("../../../../../../fixtures/gate6-vp8-black-640x360.vp8").to_vec(),
    };
    let output = video::decode(&frame, Instant::now() + Duration::from_secs(5))
        .await
        .unwrap();
    assert!(!output.forced);
    assert!(output.pid > 0);
    assert_eq!(output.pixels, 384);
    let corrupt = connect_client::NativeVideoFrame {
        encoded: vec![0; 16],
        ..frame
    };
    let error = video::decode(&corrupt, Instant::now() + Duration::from_secs(5))
        .await
        .err()
        .unwrap();
    assert!(
        error.contains("actual received VP8 decode refused"),
        "{error}"
    );
}
#[tokio::test]
async fn expired_original_decoder_cutoff_is_not_renewed_or_launched() {
    let frame = connect_client::NativeVideoFrame {
        timestamp_rtp: 0,
        keyframe: true,
        encoded: vec![0; 16],
    };
    let error = video::decode(&frame, Instant::now() - Duration::from_millis(1))
        .await
        .err()
        .unwrap();
    assert_eq!(error, "original replacement expired before decoder launch");
}

#[test]
fn only_signed_transient_refusal_keeps_original_physical_peer_replayable() {
    for code in ["authority-busy", "storage-unavailable"] {
        assert_eq!(
            recovery(&ClientError::Application(connect_protocol::Error::new(
                code,
                "actual signed refusal"
            ))),
            Some(true)
        );
    }
    assert_eq!(recovery(&ClientError::Timeout), Some(false));
    assert_eq!(recovery(&ClientError::Transport), Some(false));
    for error in [
        ClientError::Authentication,
        ClientError::Protocol,
        ClientError::Cancelled,
        ClientError::Application(connect_protocol::Error::new(
            "conflict",
            "changed original observation",
        )),
    ] {
        assert_eq!(recovery(&error), None);
    }
}
