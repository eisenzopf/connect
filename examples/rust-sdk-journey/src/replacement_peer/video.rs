//! Original 30fps VP8 source; closing a gate never retimes a captured frame.
use super::*;
use base64::{engine::general_purpose::STANDARD, Engine};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Video {
    pub stream_id: String,
    pub receive_audio_stream_id: String,
    pub receive_video_stream_id: String,
    pub ssrc: u32,
    #[serde(deserialize_with = "frames")]
    pub frames: Vec<Vec<u8>>,
}
fn frames<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Vec<Vec<u8>>, D::Error> {
    let encoded = Vec::<String>::deserialize(deserializer)?;
    if encoded.len() > 62 * 30 {
        return Err(serde::de::Error::custom(
            "VP8 frame count exceeds original program",
        ));
    }
    let mut total = 0usize;
    encoded
        .into_iter()
        .map(|value| {
            if value.len() > (256 * 1024 + 2) / 3 * 4 {
                return Err(serde::de::Error::custom("VP8 encoded frame exceeds bound"));
            }
            let frame = STANDARD.decode(value).map_err(serde::de::Error::custom)?;
            total += frame.len();
            if frame.len() > 256 * 1024 || total > 16 * 1024 * 1024 {
                return Err(serde::de::Error::custom("VP8 decoded source exceeds bound"));
            }
            Ok(frame)
        })
        .collect()
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
        ];
        if self.ssrc == 0
            || self.ssrc == audio_ssrc
            || ids.iter().any(|v| !id(v))
            || ids.iter().collect::<std::collections::BTreeSet<_>>().len() != 3
            || self.frames.len() != usize::from(seconds) * 30
            || self.frames.iter().map(Vec::len).sum::<usize>() > 16 * 1024 * 1024
        {
            return Err("replacement VP8 source exceeds the declared profile".into());
        }
        let mut previous_key = None;
        for (index, frame) in self.frames.iter().enumerate() {
            if frame.len() < 3 || frame.len() > 256 * 1024 {
                return Err("invalid encoded VP8 frame bound".into());
            }
            if key(frame) {
                if frame.len() < 10
                    || frame[3..6] != [0x9d, 0x01, 0x2a]
                    || u16::from_le_bytes([frame[6], frame[7]]) & 0x3fff != 640
                    || u16::from_le_bytes([frame[8], frame[9]]) & 0x3fff != 360
                {
                    return Err("replacement VP8 source must be 640x360".into());
                }
                previous_key = Some(index);
            }
            if previous_key.is_none_or(|last| index - last >= 30) {
                return Err(
                    "replacement VP8 requires an original keyframe at least every second".into(),
                );
            }
        }
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
    let chunks = encoded.chunks(1167).collect::<Vec<_>>();
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
                binding.pt,
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
    fn compact_original_frames_decode_exactly_and_refuse_noncanonical_or_unbounded_input() {
        let source = json!({"stream_id":"camera","receive_audio_stream_id":"audio-in",
            "receive_video_stream_id":"video-in","ssrc":2,"frames":[STANDARD.encode([0, 1, 255])]});
        assert_eq!(
            serde_json::from_value::<Video>(source.clone())
                .unwrap()
                .frames,
            vec![vec![0, 1, 255]]
        );
        for invalid in [
            json!(["!invalid!"]),
            json!([[0, 1, 255]]),
            json!(["AA== "]),
            json!(vec!["AA=="; 1861]),
            json!([STANDARD.encode(vec![0; 256 * 1024 + 1])]),
        ] {
            let mut changed = source.clone();
            changed["frames"] = invalid;
            assert!(serde_json::from_value::<Video>(changed).is_err());
        }
    }
    #[test]
    fn original_vp8_fragments_round_trip_public_receiver_with_wrap_and_exact_timestamps() {
        let binding = selected::Binding {
            id: "camera".into(),
            handle: 7,
            pt: 96,
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
    fn dimensions_identity_keyframe_cadence_and_bytes_refuse_before_opening_a_peer() {
        let mut frame = vec![4u8; 20];
        frame[3..10].copy_from_slice(&[0x9d, 0x01, 0x2a, 0x80, 0x02, 0x68, 0x01]);
        let build = || Video {
            stream_id: "camera".into(),
            receive_audio_stream_id: "remote-audio".into(),
            receive_video_stream_id: "remote-video".into(),
            ssrc: 2,
            frames: vec![frame.clone(); 30],
        };
        assert!(build().validate(1, 1).is_ok());
        for change in 0..6 {
            let mut source = build();
            match change {
                0 => source.ssrc = 1,
                1 => source.receive_video_stream_id = source.stream_id.clone(),
                2 => {
                    source.frames.pop();
                }
                3 => source.frames[0][6] = 0,
                4 => {
                    for frame in &mut source.frames {
                        frame[0] |= 1;
                    }
                }
                _ => source.frames[0] = vec![0; 256 * 1024 + 1],
            }
            assert!(source.validate(1, 1).is_err(), "{change}");
        }
    }
}
