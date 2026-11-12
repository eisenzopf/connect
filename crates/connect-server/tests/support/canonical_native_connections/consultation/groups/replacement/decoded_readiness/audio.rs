//! Actual G.711 companding or libopus output, never empty-input PLC/FEC.
use serde_json::Value;
#[derive(Clone, Copy)]
enum Kind {
    Mu,
    A,
    Opus,
}
/// Copied RTP coordinates only: no payload, authority or media owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RtpHeader {
    pub(super) ssrc: u32,
    pub(super) sequence: u16,
    pub(super) timestamp: u32,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ResetReasons {
    pub(super) ssrc_changed: bool,
    pub(super) sequence_discontinuity: bool,
    pub(super) timestamp_discontinuity: bool,
}
impl ResetReasons {
    pub(super) fn any(self) -> bool {
        self.ssrc_changed || self.sequence_discontinuity || self.timestamp_discontinuity
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ContinuityObservation {
    // Diagnostic accepted-decode serial, never an RTP or operation identifier.
    pub(super) decode_serial: u64,
    pub(super) previous: Option<RtpHeader>,
    pub(super) current: RtpHeader,
    pub(super) reset: ResetReasons,
}
pub(super) struct Decoder {
    kind: Kind,
    rate: u32,
    opus: Option<opus::Decoder>,
    previous: Option<(u32, u16, u32)>,
    samples: u64,
    decode_serial: u64,
    continuity: Option<ContinuityObservation>,
}
impl Decoder {
    pub(super) fn selected(codec: &Value) -> Result<Self, String> {
        let (kind, rate, pt) = match codec["name"].as_str() {
            Some("pcmu" | "g.711-mu") => (Kind::Mu, 8000, 0),
            Some("pcma" | "g.711-a") => (Kind::A, 8000, 8),
            Some("opus") => (Kind::Opus, 48000, 111),
            _ => return Err("unsupported actual fixture audio decoder".into()),
        };
        if codec["params"]["sample_rate"] != rate
            || codec["params"]["channels"] != 1
            || codec["params"]["payload_type"] != pt
        {
            return Err("audio parameters differ from negotiated decoder".into());
        }
        Ok(Self {
            kind,
            rate,
            opus: if matches!(kind, Kind::Opus) {
                Some(opus::Decoder::new(rate, opus::Channels::Mono).map_err(|e| e.to_string())?)
            } else {
                None
            },
            previous: None,
            samples: 0,
            decode_serial: 0,
            continuity: None,
        })
    }
    pub(super) fn rate(&self) -> u32 {
        self.rate
    }
    pub(super) fn samples(&self) -> u64 {
        self.samples
    }
    pub(super) fn continuity(&self) -> Option<ContinuityObservation> {
        self.continuity
    }
    pub(super) fn usable(&self) -> bool {
        self.samples >= u64::from(self.rate) * 60 / 1000
    }
    pub(super) fn encoded_silence(&self) -> Result<Vec<u8>, String> {
        Ok(match self.kind {
            Kind::Mu => vec![0xff; 160],
            Kind::A => vec![0xd5; 160],
            Kind::Opus => {
                let mut encoder =
                    opus::Encoder::new(48000, opus::Channels::Mono, opus::Application::Audio)
                        .map_err(|e| e.to_string())?;
                let mut bytes = vec![0u8; 1275];
                let count = encoder
                    .encode(&[0i16; 960], &mut bytes)
                    .map_err(|e| e.to_string())?;
                bytes.truncate(count);
                bytes
            }
        })
    }
    // True reports actual new PCM output. Duplicate/late packets produce none.
    pub(super) fn decode(
        &mut self,
        ssrc: u32,
        seq: u16,
        timestamp: u32,
        packet: &[u8],
    ) -> Result<bool, String> {
        if packet.is_empty() {
            return Err("empty audio cannot invoke PLC or count as a decode".into());
        }
        let frame_samples = self.rate / 50;
        let mut reset = ResetReasons::default();
        if let Some((old_source, old_seq, old_timestamp)) = self.previous {
            if old_source == ssrc && seq.wrapping_sub(old_seq) >= 32768 {
                return Ok(false);
            }
            if old_source == ssrc && seq == old_seq {
                return Ok(false);
            }
            reset = ResetReasons {
                ssrc_changed: old_source != ssrc,
                sequence_discontinuity: seq != old_seq.wrapping_add(1),
                timestamp_discontinuity: timestamp != old_timestamp.wrapping_add(frame_samples),
            };
            if reset.any() {
                self.samples = 0;
                if let Some(decoder) = self.opus.as_mut() {
                    decoder.reset_state().map_err(|e| e.to_string())?;
                }
            }
        }
        let count = match self.kind {
            Kind::Mu | Kind::A => {
                if packet.len() != frame_samples as usize {
                    return Err("G711 packet is not actual 20ms PCM".into());
                }
                // Materialize actual PCM. Silence is valid decoded output; byte
                // count alone is never the observation supplied to readiness.
                let decoded = packet
                    .iter()
                    .map(|byte| match self.kind {
                        Kind::Mu => mu(*byte),
                        Kind::A => alaw(*byte),
                        Kind::Opus => unreachable!(),
                    })
                    .collect::<Vec<i16>>();
                std::hint::black_box(&decoded);
                decoded.len()
            }
            Kind::Opus => {
                let mut pcm = [0i16; 5760];
                let decoded = self
                    .opus
                    .as_mut()
                    .unwrap()
                    .decode(packet, &mut pcm, false)
                    .map_err(|e| e.to_string())?;
                if decoded != frame_samples as usize {
                    return Err("Opus decoder did not output negotiated 20ms".into());
                }
                std::hint::black_box(&pcm[..decoded]);
                decoded
            }
        };
        // Only genuine successful PCM output creates an observation. Errors,
        // ignored duplicates and late packets retain the last accepted one.
        self.decode_serial = self.decode_serial.wrapping_add(1);
        self.continuity = Some(ContinuityObservation {
            decode_serial: self.decode_serial,
            previous: self.previous.map(|(ssrc, sequence, timestamp)| RtpHeader {
                ssrc,
                sequence,
                timestamp,
            }),
            current: RtpHeader {
                ssrc,
                sequence: seq,
                timestamp,
            },
            reset,
        });
        self.previous = Some((ssrc, seq, timestamp));
        self.samples += count as u64;
        Ok(true)
    }
}
pub(super) fn mu(byte: u8) -> i16 {
    let u = !byte;
    let magnitude = ((i32::from(u & 15) << 3) + 132) << ((u >> 4) & 7);
    if u & 128 != 0 {
        (132 - magnitude) as i16
    } else {
        (magnitude - 132) as i16
    }
}
pub(super) fn alaw(byte: u8) -> i16 {
    let a = byte ^ 0x55;
    let segment = (a >> 4) & 7;
    let value = match segment {
        0 => (i32::from(a & 15) << 4) + 8,
        1 => (i32::from(a & 15) << 4) + 264,
        _ => ((i32::from(a & 15) << 4) + 264) << (segment - 1),
    };
    if a & 128 != 0 {
        value as i16
    } else {
        -value as i16
    }
}
