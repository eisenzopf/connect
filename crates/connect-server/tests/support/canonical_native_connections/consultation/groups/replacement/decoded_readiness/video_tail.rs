//! Ordinary negotiated RTP observations, never a fabricated signed video epoch.
use super::*;
use connect_protocol::replacement_activation::{
    NativeActivationAudience, NativeActivationBoundary, SequenceMembership,
};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) struct VideoTail {
    selected: connect_client::canonical::NativeReadinessStream,
    generation: u64,
    ssrc: u32,
    timestamps: BTreeSet<u32>,
    assembler: VideoFrameAssembler,
    observations: Vec<(u32, u32, bool)>,
}
impl VideoTail {
    pub(super) fn new(
        readiness: &NativeReplacementReadiness,
        ssrc: u32,
        timestamps: BTreeSet<u32>,
        assembler: VideoFrameAssembler,
        private: Vec<(u32, u32)>,
    ) -> Self {
        let selected = readiness
            .video()
            .expect("original negotiated video")
            .clone();
        selected.native_egress.validate().unwrap();
        assert!(private.len() <= 2048 && !timestamps.is_empty() && timestamps.len() <= 2048);
        assert!(private.iter().all(|(_, t)| timestamps.contains(t)));
        Self {
            selected,
            generation: readiness.target_generation(),
            ssrc,
            timestamps,
            assembler,
            observations: private.into_iter().map(|(s, t)| (s, t, true)).collect(),
        }
    }
    pub(crate) fn packets(&self) -> usize {
        self.observations.len()
    }
    pub(crate) fn observe(&mut self, bytes: &[u8]) -> std::result::Result<(), &'static str> {
        let ObservedNativeMediaDatagram::Rtp(packet) =
            observe_native_media_datagram(bytes).map_err(|_| "invalid native video datagram")?
        else {
            return Err("non-RTP datagram is not video");
        };
        if packet.stream_local_id != self.selected.stream_local_id
            || self.observations.len() >= 2048
        {
            return Err("unexpected or unbounded video stream");
        }
        let p = NativeVideoPacket::parse(
            packet
                .rtp
                .packet()
                .serialize()
                .map_err(|_| "invalid video RTP image")?
                .to_vec(),
        )
        .map_err(|_| "invalid VP8 packet")?;
        if p.payload_type != self.selected.payload_type
            || p.ssrc != self.ssrc
            || !self.timestamps.contains(&p.timestamp_rtp)
        {
            return Err("video differs from original negotiated source");
        }
        if let Some(frame) = self
            .assembler
            .push(&p)
            .map_err(|_| "invalid video assembly")?
        {
            if !frame.keyframe
                || frame.encoded.as_slice()
                    != include_bytes!("../../../../../../fixtures/gate6-vp8-black-640x360.vp8")
            {
                return Err("video differs from actual original frame bytes");
            }
        }
        self.observations.push((packet.seq, p.timestamp_rtp, false));
        Ok(())
    }
    /// Caller first verifies this completed boundary using the original SDK
    /// readiness object. No buffered packet earns audio or public-video credit.
    pub(crate) fn finish(
        &self,
        boundary: &NativeActivationBoundary,
    ) -> std::result::Result<(usize, usize), &'static str> {
        boundary
            .validate()
            .map_err(|_| "invalid activation boundary")?;
        if boundary.target_generation != self.generation {
            return Err("foreign video generation");
        }
        let NativeActivationAudience::Public { streams } = &boundary.audience else {
            return Err("no public video membership");
        };
        let cut = streams
            .iter()
            .find(|s| s.stream_id == self.selected.stream_id)
            .ok_or("missing original video cut")?;
        cut.matches(
            &self.selected.stream_id,
            self.selected.stream_local_id,
            &self.selected.native_egress,
            self.generation,
        )
        .map_err(|_| "foreign video cut")?;
        if self.assembler.diagnostics().pending_frames != 0 {
            return Err("incomplete original video frame");
        }
        let mut frames = BTreeMap::new();
        let (mut private, mut public) = (0, 0);
        for (sequence, timestamp, preparing) in &self.observations {
            let membership = cut
                .membership(*sequence)
                .map_err(|_| "unclassifiable original video sequence")?;
            if *preparing && membership != SequenceMembership::Private {
                return Err("preparation video claimed public membership");
            }
            if frames
                .insert(timestamp, membership)
                .is_some_and(|prior| prior != membership)
            {
                return Err("one video frame spans activation cut");
            }
            match membership {
                SequenceMembership::Private => private += 1,
                SequenceMembership::Public => public += 1,
            }
        }
        Ok((private, public))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use connect_protocol::replacement_activation::{
        NativeEgressIdentity, NativeSequenceCut, PROFILE,
    };
    fn model() -> VideoTail {
        VideoTail {
            selected: connect_client::canonical::NativeReadinessStream {
                stream_id: "video".into(),
                stream_local_id: 3,
                payload_type: 96,
                native_egress: NativeEgressIdentity {
                    profile: "native-sequence-cursor/1".into(),
                    physical_incarnation: "00000000000000000000000000000003".into(),
                    stream_generation: 1,
                    connection_generation: 2,
                    initial_outer_sequence: 0,
                },
            },
            generation: 2,
            ssrc: 99,
            timestamps: BTreeSet::from([9000, 18000]),
            assembler: VideoFrameAssembler::default(),
            observations: vec![],
        }
    }
    fn packet(sequence: u32, timestamp: u32) -> Vec<u8> {
        let mut rtp = vec![0x80, 0x80 | 96];
        rtp.extend_from_slice(&(sequence as u16).to_be_bytes());
        rtp.extend_from_slice(&timestamp.to_be_bytes());
        rtp.extend_from_slice(&99u32.to_be_bytes());
        rtp.push(0x10);
        rtp.extend_from_slice(include_bytes!(
            "../../../../../../fixtures/gate6-vp8-black-640x360.vp8"
        ));
        pack(&MediaDatagram {
            flags: 0,
            stream_local_id: 3,
            seq: sequence,
            payload: rtp.into(),
        })
        .to_vec()
    }
    fn boundary() -> NativeActivationBoundary {
        NativeActivationBoundary {
            profile: PROFILE.into(),
            operation_record_id: "record".into(),
            operation_id: "move".into(),
            source_connection_id: "old".into(),
            target_connection_id: "new".into(),
            target_generation: 2,
            stream_revision: 3,
            audience: NativeActivationAudience::Public {
                streams: vec![NativeSequenceCut {
                    stream_id: "video".into(),
                    stream_local_id: 3,
                    stream_generation: 1,
                    physical_incarnation: "00000000000000000000000000000003".into(),
                    sequence_space: "native-outer-u32".into(),
                    public_sequence_floor: 2,
                    cursor_ordinal: 2,
                    submission_observed: false,
                }],
            },
        }
    }
    #[test]
    fn video_tail_model_preserves_private_membership_without_audio_credit() {
        let mut v = model();
        v.observe(&packet(1, 9000)).unwrap();
        v.observe(&packet(2, 18000)).unwrap();
        assert_eq!(v.finish(&boundary()).unwrap(), (1, 1));
        v.observations[0].2 = true;
        assert_eq!(v.finish(&boundary()).unwrap(), (1, 1));
        v.observations[1].2 = true;
        assert!(v.finish(&boundary()).is_err());
    }
    #[test]
    fn video_tail_model_refuses_foreign_source_payload_and_cut() {
        for mutation in ["handle", "payload", "ssrc", "timestamp", "bytes"] {
            let mut bytes = packet(1, 9000);
            let decoded = rvoip_uctp::substrate::unpack(&bytes).unwrap();
            let mut rtp = decoded.payload.to_vec();
            let mut handle = 3;
            match mutation {
                "handle" => handle = 4,
                "payload" => rtp[1] = 0x80,
                "ssrc" => rtp[11] ^= 1,
                "timestamp" => rtp[7] ^= 1,
                "bytes" => *rtp.last_mut().unwrap() ^= 1,
                _ => unreachable!(),
            }
            bytes = pack(&MediaDatagram {
                flags: 0,
                stream_local_id: handle,
                seq: 1,
                payload: rtp.into(),
            })
            .to_vec();
            assert!(model().observe(&bytes).is_err(), "{mutation}");
        }
        let mut v = model();
        v.observations = vec![(1, 9000, false), (2, 9000, false)];
        assert!(v.finish(&boundary()).is_err());
        let mut foreign = boundary();
        let NativeActivationAudience::Public { streams } = &mut foreign.audience else {
            unreachable!()
        };
        streams[0].physical_incarnation = "00000000000000000000000000000004".into();
        assert!(model().finish(&foreign).is_err());
    }
}
