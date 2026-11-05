//! Actual private decoded readiness is separate from public programme quality.
use super::*;
use connect_client::{
    canonical::{
        observe_native_media_datagram, NativeActivationAudience, NativeDecodedVideoReadiness,
        NativeReplacementReadiness, NativeReplacementReady, ObservedNativeMediaDatagram,
    },
    sdk::VideoReceiveEpoch,
    NativeVideoPacket, VideoFrameAssembler,
};
const AGE: Duration = Duration::from_millis(200);
const REQUIRED_AUDIO_SAMPLES: u64 = 3 * 960;

struct AudioDecoder {
    codec: opus::Decoder,
    identity: Option<(u32, u16, u32)>,
    samples: u64,
    last: Option<tokio::time::Instant>,
}
impl AudioDecoder {
    fn new() -> Result<Self> {
        Ok(Self {
            codec: opus::Decoder::new(48000, opus::Channels::Mono)?,
            identity: None,
            samples: 0,
            last: None,
        })
    }
    fn frame(
        &mut self,
        ssrc: u32,
        sequence: u16,
        timestamp: u32,
        payload: &[u8],
        arrived: tokio::time::Instant,
    ) -> Result<bool> {
        if payload.is_empty() || opus::packet::get_nb_samples(payload, 48000)? != 960 {
            return Err("private readiness requires an actual selected 20ms Opus packet".into());
        }
        if let Some((old_ssrc, old_sequence, old_timestamp)) = self.identity {
            if old_ssrc == ssrc
                && (sequence == old_sequence || sequence.wrapping_sub(old_sequence) >= 32768)
            {
                return Ok(false);
            }
            if old_ssrc != ssrc {
                self.codec = opus::Decoder::new(48000, opus::Channels::Mono)?;
            }
            if old_ssrc != ssrc
                || old_sequence.wrapping_add(1) != sequence
                || old_timestamp.wrapping_add(960) != timestamp
            {
                self.samples = 0;
                self.last = None;
            }
        }
        let mut pcm = [0i16; 5760];
        let count = self.codec.decode(payload, &mut pcm, false)?;
        if count != 960 {
            return Err(
                "actual private Opus decode did not produce its original sample count".into(),
            );
        }
        self.identity = Some((ssrc, sequence, timestamp));
        self.samples += count as u64;
        self.last = Some(arrived);
        Ok(true)
    }
}
pub(super) struct Observer {
    preparation: NativeReplacementReadiness,
    audio: AudioDecoder,
    decoder: Option<decoder::Decoder>,
    assembler: VideoFrameAssembler,
    epoch: Option<VideoReceiveEpoch>,
    last_sequence: Option<u16>,
    waiting_key: bool,
    decoded_frames: u64,
    decoded_keys: u64,
    video_last: Option<tokio::time::Instant>,
    frozen: Option<NativeReplacementReady>,
    frozen_epoch: Option<VideoReceiveEpoch>,
    frozen_observations: Option<Value>,
    started: tokio::time::Instant,
    stale_packets: u64,
    incomplete_or_waiting_frames: u64,
    actual_decode_outputs: u64,
    last_decode: Option<Value>,
    joined: Option<decoder::Joined>,
    programme_start: Option<tokio::time::Instant>,
    video_receipts: Vec<Value>,
    activation: Option<Value>,
}
impl Observer {
    pub async fn open(preparation: NativeReplacementReadiness, source: &Source) -> Result<Self> {
        if source
            .video
            .as_ref()
            .map(|v| v.receive_video_stream_id.as_str())
            != preparation.video().map(|v| v.stream_id.as_str())
        {
            return Err("challenge selected another receiving video resource".into());
        }
        let audio = AudioDecoder::new()?;
        let decoder = match &source.video {
            Some(video) => Some(decoder::Decoder::open(&video.program).await?),
            None => None,
        };
        Ok(Self {
            preparation,
            audio,
            decoder,
            assembler: VideoFrameAssembler::default(),
            epoch: None,
            last_sequence: None,
            waiting_key: true,
            decoded_frames: 0,
            decoded_keys: 0,
            video_last: None,
            frozen: None,
            frozen_epoch: None,
            frozen_observations: None,
            started: tokio::time::Instant::now(),
            stale_packets: 0,
            incomplete_or_waiting_frames: 0,
            actual_decode_outputs: 0,
            last_decode: None,
            joined: None,
            programme_start: None,
            video_receipts: Vec::new(),
            activation: None,
        })
    }
    pub fn bind_programme_start(&mut self, start: tokio::time::Instant) -> Result<()> {
        if self.programme_start.replace(start).is_some() {
            return Err("decoder cannot rebase its original programme clock".into());
        }
        Ok(())
    }
    pub fn receive_video_handle(&self) -> Option<u16> {
        self.preparation.video().map(|v| v.stream_local_id)
    }
    pub fn started(&self) -> tokio::time::Instant {
        self.started
    }
    pub fn pending(&self) -> bool {
        self.decoder.as_ref().is_some_and(|d| d.pending())
    }
    pub async fn decoded(&mut self) -> Result<()> {
        let result = self
            .decoder
            .as_mut()
            .ok_or("missing original video decoder")?
            .receive()
            .await?;
        self.actual_decode_outputs += 1;
        let start = self
            .programme_start
            .ok_or("original decoder programme clock missing")?;
        if self.video_receipts.len() >= 1860 {
            return Err("original decoded receipt bound exceeded".into());
        }
        self.video_receipts.push(json!({"ordinal":result.ordinal,"timestamp_rtp":result.timestamp_rtp,"keyframe":result.keyframe,
            "video_epoch":result.epoch,"encoded_sha256":result.encoded_sha256,
            "received_programme_us":result.arrived.saturating_duration_since(start).as_micros(),
            "decoded_programme_us":result.finished.saturating_duration_since(start).as_micros()}));
        self.last_decode = Some(
            json!({"ordinal":result.ordinal,"timestamp_rtp":result.timestamp_rtp,
            "keyframe":result.keyframe,"luma_sum_16x16":result.luma_sum,
            "decode_after_receive_us":result.finished.saturating_duration_since(result.arrived).as_micros()}),
        );
        if self.epoch.as_ref() != Some(&result.epoch)
            || result.finished.saturating_duration_since(result.arrived) >= AGE
        {
            self.stale_packets += 1;
            return Ok(());
        }
        self.decoded_frames += 1;
        self.decoded_keys += u64::from(result.keyframe);
        self.video_last = Some(result.arrived);
        Ok(())
    }
    pub fn packet(
        &mut self,
        bytes: &[u8],
        epoch: Option<&VideoReceiveEpoch>,
        arrived: tokio::time::Instant,
        until: tokio::time::Instant,
    ) -> Result<Option<u16>> {
        if arrived < self.started || arrived.elapsed() >= AGE {
            self.stale_packets += 1;
            return Ok(None);
        }
        if let Some(epoch) = epoch {
            let Some(video) = self.preparation.video() else {
                return Err("unselected video reached readiness decoder".into());
            };
            if bytes.len() < 37 || u16::from_be_bytes([bytes[2], bytes[3]]) != video.stream_local_id
            {
                return Ok(None);
            }
            let packet = NativeVideoPacket::parse(bytes[24..].to_vec())?;
            if packet.payload_type != video.payload_type || packet.ssrc != epoch.ssrc {
                return Err("private video packet changed selected source".into());
            }
            if self.epoch.as_ref() != Some(epoch) {
                self.epoch = Some(epoch.clone());
                self.assembler = VideoFrameAssembler::default();
                self.last_sequence = None;
                self.waiting_key = true;
                self.decoded_frames = 0;
                self.decoded_keys = 0;
                self.video_last = None;
            }
            let before = self.assembler.diagnostics();
            let frame = self.assembler.push(&packet)?;
            let after = self.assembler.diagnostics();
            if before.discarded_frames != after.discarded_frames
                || before.expired_frames != after.expired_frames
            {
                self.waiting_key = true;
            }
            if let Some(frame) = frame {
                let range = self
                    .assembler
                    .completed_sequence_range()
                    .ok_or("complete frame omitted original sequence span")?;
                if self
                    .last_sequence
                    .is_some_and(|old| old.wrapping_add(1) != range.first)
                {
                    self.waiting_key = true;
                }
                self.last_sequence = Some(range.last);
                if self.waiting_key && !frame.keyframe {
                    self.incomplete_or_waiting_frames += 1;
                    return Ok(None);
                }
                if frame.keyframe {
                    self.waiting_key = false;
                }
                self.decoder
                    .as_mut()
                    .ok_or("actual video decoder unavailable")?
                    .submit(frame, epoch.clone(), arrived, until)?;
            }
            return Ok(None);
        }
        let ObservedNativeMediaDatagram::Rtp(packet) = observe_native_media_datagram(bytes)? else {
            return Ok(None);
        };
        if packet.stream_local_id != self.preparation.audio().stream_local_id {
            return Ok(None);
        }
        let packet = packet.rtp.into_packet();
        let h = packet.header;
        if h.payload_type != self.preparation.audio().payload_type {
            return Err("private Opus payload changed selected codec".into());
        }
        Ok(self
            .audio
            .frame(
                h.ssrc,
                h.sequence_number,
                h.timestamp,
                &packet.payload,
                arrived,
            )?
            .then_some(960))
    }

    pub fn proof(&mut self, current_video_epoch: bool) -> Result<Option<NativeReplacementReady>> {
        if self.preparation.video().is_some() && (!current_video_epoch || self.epoch.is_none()) {
            return Ok(None);
        }
        if let Some(proof) = &self.frozen {
            if self.epoch != self.frozen_epoch {
                return Err("original decoded readiness source epoch changed; frozen proof cannot be replaced".into());
            }
            return Ok(Some(proof.clone()));
        }
        if self.audio.samples < REQUIRED_AUDIO_SAMPLES
            || self.audio.last.is_none_or(|at| at.elapsed() >= AGE)
            || (self.preparation.video().is_some()
                && (self.decoded_frames == 0
                    || self.decoded_keys == 0
                    || self.video_last.is_none_or(|at| at.elapsed() >= AGE)))
        {
            return Ok(None);
        }
        let video = self
            .preparation
            .video()
            .map(|_| NativeDecodedVideoReadiness {
                decoded_frames: self.decoded_frames,
                decoded_keyframes: self.decoded_keys,
            });
        let proof = self.preparation.with_decoded(self.audio.samples, video)?;
        self.frozen_observations = Some(json!({"audio_decoded_samples":self.audio.samples,
            "audio_ssrc":self.audio.identity.map(|v|v.0),"audio_last_sequence":self.audio.identity.map(|v|v.1),"audio_last_timestamp":self.audio.identity.map(|v|v.2),
            "video_decoded_frames":self.decoded_frames,"video_decoded_keyframes":self.decoded_keys,"video_epoch":self.epoch}));
        self.frozen = Some(proof.clone());
        self.frozen_epoch = self.epoch.clone();
        Ok(Some(proof))
    }
    pub fn completed(&mut self, generation: u64, revision: u64, status: &Value) -> Result<()> {
        if self.frozen.is_none()
            || generation != self.preparation.target_generation()
            || revision <= self.preparation.stream_revision()
        {
            return Err(
                "completed root differs from original decoded preparation generation/revision"
                    .into(),
            );
        }
        let activation = self.preparation.activation_from_completed(status)?;
        if !matches!(activation.audience, NativeActivationAudience::Public { .. }) {
            return Err("this public programme has no public native activation cut".into());
        }
        let value = serde_json::to_value(activation)?;
        if self.activation.as_ref().is_some_and(|old| old != &value) {
            return Err("original native activation cut changed".into());
        }
        self.activation = Some(value);
        Ok(())
    }
    pub fn epoch(&self) -> Option<&VideoReceiveEpoch> {
        self.epoch.as_ref()
    }
    pub async fn stop_and_join(&mut self) -> Result<()> {
        if let Some(decoder) = &mut self.decoder {
            let joined = decoder.stop_and_join().await?;
            let passed = joined.joined && joined.clean && !joined.forced;
            self.joined = Some(joined);
            if !passed {
                return Err("actual decoder failed or required forced cleanup".into());
            }
        }
        Ok(())
    }
    pub fn summary(&self, start: Option<u128>) -> Value {
        json!({"profile":"connect/native-private-decoded-readiness/1","audio_decoder":"rust-opus-0.3.1-48k-mono-no-PLC-no-FEC",
            "audio_decoded_samples":self.audio.samples,"audio_samples_required":REQUIRED_AUDIO_SAMPLES,
            "video_decoder":self.decoder.as_ref().map(|_|"owned-libvpx-vp8"),"video_decoded_frames":self.decoded_frames,
            "video_decoded_keyframes":self.decoded_keys,"actual_video_decode_outputs":self.actual_decode_outputs,
            "stale_decode_observations":self.stale_packets,"waiting_for_keyframe":self.incomplete_or_waiting_frames,
            "native_activation":self.activation,"readiness_frozen":self.frozen.is_some(),"frozen_private_observations":self.frozen_observations,"decoder_reset_on_root_completion":false,
            "record_id":self.preparation.record_id(),"connection_id":self.preparation.connection_id(),"operation_id":self.preparation.operation_id(),
            "source_generation":self.preparation.source_generation(),"target_generation":self.preparation.target_generation(),
            "stream_revision":self.preparation.stream_revision(),"video_epoch":self.epoch,"decoder_process":self.joined,
            "last_video_decode":self.last_decode,"video_decode_receipts":self.video_receipts,"program_start_unix_us":start,"full_media_quality_verified":false})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_silent_opus_needs_three_decodes_without_plc_fec_or_duplicate_counts() {
        let mut encoder =
            opus::Encoder::new(48000, opus::Channels::Mono, opus::Application::Audio).unwrap();
        let mut packet = [0u8; 1275];
        let size = encoder.encode(&[0i16; 960], &mut packet).unwrap();
        let packet = &packet[..size];
        let mut decoder = AudioDecoder::new().unwrap();
        let start = tokio::time::Instant::now();
        for i in 0..3 {
            decoder
                .frame(7, i, (i as u32) * 960, packet, start)
                .unwrap();
        }
        assert_eq!(
            decoder.samples, REQUIRED_AUDIO_SAMPLES,
            "actual decoded silence remains eligible"
        );
        decoder.frame(7, 2, 1920, packet, start).unwrap();
        assert_eq!(
            decoder.samples, 2880,
            "same original RTP cannot count twice"
        );
        decoder.frame(7, 1, 960, packet, start).unwrap();
        assert_eq!(decoder.samples, 2880, "old RTP cannot count twice");
        assert!(
            decoder.frame(7, 3, 2880, &[], start).is_err(),
            "no PLC from absent input"
        );
        decoder.frame(7, 4, 3840, packet, start).unwrap();
        assert_eq!(
            decoder.samples, 960,
            "gap requires new actual contiguous observations"
        );
        decoder.frame(8, 5, 4800, packet, start).unwrap();
        assert_eq!(
            decoder.samples, 960,
            "another actual source starts its own decoder identity"
        );
    }
    #[test]
    fn different_duration_and_invalid_opus_are_not_fixed_profile_readiness() {
        let mut encoder =
            opus::Encoder::new(48000, opus::Channels::Mono, opus::Application::Audio).unwrap();
        let mut packet = [0u8; 1275];
        let size = encoder.encode(&[0i16; 480], &mut packet).unwrap();
        let mut decoder = AudioDecoder::new().unwrap();
        let now = tokio::time::Instant::now();
        assert!(decoder.frame(1, 0, 0, &packet[..size], now).is_err());
        assert!(decoder.frame(1, 0, 0, &[0xff], now).is_err());
        assert_eq!(decoder.samples, 0);
    }
}
