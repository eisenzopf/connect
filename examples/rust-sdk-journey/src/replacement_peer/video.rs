//! Original 30fps VP8 source; closing a gate never retimes a captured frame.
use super::*;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Video {
    pub stream_id: String,
    pub receive_audio_stream_id: String,
    pub receive_video_stream_id: String,
    #[serde(default)]
    pub receive_data_stream_id: Option<String>,
    pub ssrc: u32,
    pub program: encoder::Program,
}
pub(super) fn id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._~-".contains(&b))
}
pub(super) fn key(frame: &[u8]) -> bool {
    frame.first().is_some_and(|b| b & 1 == 0)
}
impl Video {
    pub fn validate(&self, seconds: u16, audio_ssrc: u32) -> Result<()> {
        let ids = [
            &self.stream_id,
            &self.receive_audio_stream_id,
            &self.receive_video_stream_id,
        ]
        .into_iter()
        .chain(self.receive_data_stream_id.iter())
        .collect::<Vec<_>>();
        if self.ssrc == 0
            || self.ssrc == audio_ssrc
            || ids.iter().any(|v| !id(v))
            || ids.iter().collect::<std::collections::BTreeSet<_>>().len() != ids.len()
            || self.program.frame_count != u32::from(seconds) * 30
        {
            return Err("replacement VP8 source exceeds the declared profile".into());
        }
        self.program.validate()?;
        Ok(())
    }
}
pub(super) fn packets(
    binding: &selected::Binding,
    ssrc: u32,
    timestamp: u32,
    sequence: &mut u16,
    datagram: &mut u32,
    encoded: &[u8],
) -> Vec<Vec<u8>> {
    // Reserve the selected 16-byte receiver epoch within the original MTU.
    let chunks = encoded.chunks(1151).collect::<Vec<_>>();
    chunks
        .iter()
        .enumerate()
        .map(|(index, chunk)| {
            let mut payload = vec![if index == 0 { 0x10 } else { 0 }];
            payload.extend_from_slice(chunk);
            let mut bytes = wire(
                binding.handle,
                *datagram,
                *sequence,
                timestamp,
                ssrc,
                binding
                    .pt
                    .expect("validated VP8 binding has an RTP payload type"),
                &payload,
            );
            if index + 1 == chunks.len() {
                bytes[9] |= 0x80;
            }
            *sequence = sequence.wrapping_add(1);
            *datagram = datagram.wrapping_add(1);
            bytes
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_vp8_fragments_round_trip_public_receiver_with_wrap_and_exact_timestamps() {
        let binding = selected::Binding {
            id: "camera".into(),
            handle: 7,
            pt: Some(96),
            raw: json!({}),
        };
        let mut frame = vec![4u8; 5000];
        frame[3..10].copy_from_slice(&[0x9d, 0x01, 0x2a, 0x80, 0x02, 0x68, 0x01]);
        let mut sequence = u16::MAX - 1;
        let mut datagram = u32::MAX - 1;
        let packets = packets(
            &binding,
            53,
            u32::MAX - 900,
            &mut sequence,
            &mut datagram,
            &frame,
        );
        assert_eq!(packets.len(), 5);
        assert_eq!(sequence, 3);
        assert_eq!(datagram, 3);
        let mut receiver = connect_client::VideoFrameAssembler::default();
        let mut decoded = None;
        for bytes in packets {
            assert!(bytes.len() - 8 <= 1180);
            assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), 7);
            let packet = connect_client::NativeVideoPacket::parse(bytes[8..].to_vec()).unwrap();
            assert_eq!(packet.timestamp_rtp, u32::MAX - 900);
            if let Some(frame) = receiver.push(&packet).unwrap() {
                assert!(decoded.replace(frame).is_none());
            }
        }
        assert_eq!(decoded.unwrap().encoded, frame);
    }
    #[test]
    fn live_program_dimensions_count_and_identity_contract_refuses_before_peer_io() {
        let build = || Video {
            stream_id: "camera".into(),
            receive_audio_stream_id: "remote-audio".into(),
            receive_video_stream_id: "remote-video".into(),
            receive_data_stream_id: None,
            ssrc: 2,
            program: encoder::test_program(30),
        };
        assert!(build().validate(1, 1).is_ok());
        for change in 0..7 {
            let mut source = build();
            match change {
                0 => source.ssrc = 1,
                1 => source.receive_video_stream_id = source.stream_id.clone(),
                2 => source.program.frame_count = 29,
                3 => source.program.raw_path = "relative.i420".into(),
                4 => source.program.raw_sha256 = "0".into(),
                5 => source.program.executable = source.program.raw_path.clone(),
                _ => source.program.executable = "/tmp/.local/../foreign".into(),
            }
            assert!(source.validate(1, 1).is_err(), "{change}");
        }
    }
}
