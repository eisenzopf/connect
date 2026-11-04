//! One original packet programme progresses while signed control is pending.
use super::*;
use connect_client::canonical::{NativeDatagrams, NativeVideoDatagrams};
use std::future::Future;

// The application owns its original programme; protocol history and feedback
// validation belong to the explicitly selected public client IO owner.
pub(super) enum Io {
    Audio(NativeDatagrams),
    Video(NativeVideoDatagrams),
}
struct Received {
    bytes: Vec<u8>,
    epoch: Option<connect_client::sdk::VideoReceiveEpoch>,
    arrived: tokio::time::Instant,
}
impl Io {
    fn send(&self, bytes: &[u8]) -> Result<()> {
        match self {
            Self::Audio(io) => io.send(bytes)?,
            Self::Video(io) => io.send(bytes)?,
        };
        Ok(())
    }
    async fn receive(&mut self) -> Result<Received> {
        match self {
            Self::Audio(io) => {
                let bytes = io.receive().await?;
                Ok(Received {
                    bytes,
                    epoch: None,
                    arrived: tokio::time::Instant::now(),
                })
            }
            Self::Video(io) => {
                let packet = io.receive().await?;
                Ok(Received {
                    bytes: packet.bytes,
                    epoch: packet.video_epoch,
                    arrived: packet.received_at,
                })
            }
        }
    }
    fn keyframe(&mut self, handle: u16, ssrc: u32) -> Result<bool> {
        Ok(match self {
            Self::Audio(_) => false,
            Self::Video(io) => io.take_keyframe_request(handle, ssrc)?,
        })
    }
}

#[derive(Default)]
pub(super) struct Observation {
    pub rows: Vec<Value>,
    pub sent: u32,
    pub skipped: u32,
    pub captured_bytes: usize,
    pub sent_video: u32,
    pub skipped_video: u32,
    pub video_open: bool,
    pub root_observed: bool,
    pub root_observed_programme_us: Option<u128>,
    pub program_origin: Option<u128>,
    pub source_start_audio_index: usize,
    pub source_start_video_index: usize,
    pub control_packet_events: u32,
    pub video_feedback_requests: u32,
}

pub(super) struct Pump<'a> {
    io: Io,
    source: &'a Source,
    audio: &'a selected::Binding,
    video: Option<&'a selected::Binding>,
    pub observation: &'a mut Observation,
    pub start: tokio::time::Instant,
    pub until: tokio::time::Instant,
    unix: u128,
    audio_index: usize,
    video_index: usize,
    datagram: u32,
    octets: u32,
    rtp_seq: u16,
    video_seq: u16,
    video_octets: u32,
    first_video_key: bool,
    last_video_clock: Option<usize>,
    encoder: Option<&'a mut encoder::Encoder>,
    encoded: Option<encoder::Frame>,
}
impl<'a> Pump<'a> {
    pub fn new(
        io: Io,
        source: &'a Source,
        audio: &'a selected::Binding,
        video: Option<&'a selected::Binding>,
        observation: &'a mut Observation,
        encoder: Option<&'a mut encoder::Encoder>,
    ) -> Result<Self> {
        let anchor = tokio::time::Instant::now();
        let program = timeline::Program::at(
            source.program_start_unix_us,
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_micros(),
            source.seconds,
        )?;
        let start = anchor
            .checked_sub(program.elapsed)
            .ok_or("original program monotonic origin out of bounds")?;
        observation.program_origin = Some(program.unix);
        observation.source_start_audio_index = program.audio_index;
        observation.source_start_video_index = program.video_index;
        Ok(Self {
            io,
            source,
            audio,
            video,
            observation,
            start,
            until: start + Duration::from_secs(u64::from(source.seconds)),
            unix: program.unix,
            audio_index: program.audio_index,
            video_index: program.video_index,
            datagram: 0,
            octets: 0,
            rtp_seq: 0,
            video_seq: 0,
            video_octets: 0,
            first_video_key: true,
            last_video_clock: None,
            encoder,
            encoded: None,
        })
    }

    /// The original control future remains pinned and is never restarted. Its
    /// cancellation/uncertainty semantics remain the client's responsibility.
    pub async fn during<F: Future>(&mut self, control: F) -> Result<F::Output> {
        self.during_until(control, self.until).await
    }

    /// An already-started local stop-marker read may join after packet time
    /// ends. It cannot extend media or start another control operation.
    pub async fn joining_stop_read<F: Future>(&mut self, read: F) -> Result<F::Output> {
        self.during_until(read, self.until + Duration::from_secs(5))
            .await
    }

    async fn during_until<F: Future>(
        &mut self,
        control: F,
        cutoff: tokio::time::Instant,
    ) -> Result<F::Output> {
        tokio::pin!(control);
        loop {
            if tokio::time::Instant::now() >= cutoff {
                return Err("original media programme ended during control".into());
            }
            let before = self.observation.rows.len() as u32
                + self.observation.sent
                + self.observation.sent_video;
            tokio::select! {
                biased;
                value = &mut control => return Ok(value),
                _ = tokio::time::sleep_until(cutoff) => return Err("original control cutoff expired".into()),
                result = self.step(self.until), if tokio::time::Instant::now() < self.until => {
                    result?;
                    let after = self.observation.rows.len() as u32
                        + self.observation.sent + self.observation.sent_video;
                    self.observation.control_packet_events += after - before;
                }
            }
            tokio::task::consume_budget().await;
        }
    }

    pub async fn step(&mut self, control_tick: tokio::time::Instant) -> Result<()> {
        if tokio::time::Instant::now() >= self.until {
            return Ok(());
        }
        let next_audio = if self.audio_index < self.source.audio_packets.len() {
            self.start + Duration::from_millis(self.audio_index as u64 * 20)
        } else {
            self.until
        };
        let next_video = self
            .source
            .video
            .as_ref()
            .filter(|v| self.video_index < v.program.frame_count as usize)
            .map(|_| {
                if self
                    .encoder
                    .as_ref()
                    .is_some_and(|encoder| encoder.pending())
                {
                    self.until
                } else {
                    self.start + Duration::from_micros(self.video_index as u64 * 1_000_000 / 30)
                }
            })
            .unwrap_or(self.until);
        let next = next_audio.min(next_video).min(control_tick).min(self.until);
        let pending = self
            .encoder
            .as_ref()
            .is_some_and(|encoder| encoder.pending());
        let packet = tokio::select! {
            biased;
            _ = tokio::time::sleep_until(next) => None,
            frame = async { self.encoder.as_mut().ok_or("missing live encoder")?.receive().await }, if pending => {
                // No await between taking the retained reply and storing it in
                // Pump. A control event can cancel the wait, never its ownership.
                self.encoded = Some(frame?);
                None
            },
            packet = self.io.receive() => Some(packet),
        };
        if let Some(packet) = packet {
            let packet = packet?;
            self.observation.captured_bytes += packet.bytes.len();
            let maximum_rows = if self.source.video.is_some() {
                16000
            } else {
                4000
            };
            if self.observation.rows.len() >= maximum_rows
                || self.observation.captured_bytes > 16 * 1024 * 1024
            {
                return Err("replacement receive observation bound exceeded".into());
            }
            let elapsed = packet
                .arrived
                .saturating_duration_since(self.start)
                .as_micros();
            let completed = self
                .observation
                .root_observed_programme_us
                .is_some_and(|at| elapsed >= at);
            self.observation.rows.push(
                json!({"elapsed_us":elapsed,"datagram":packet.bytes,"video_epoch":packet.epoch,
                "original_root_completed_observed":completed}),
            );
            return Ok(());
        }
        if tokio::time::Instant::now() >= self.until {
            return Ok(());
        }
        if self.audio_index < self.source.audio_packets.len()
            && tokio::time::Instant::now() >= next_audio
        {
            let elapsed = self.start.elapsed().as_millis();
            let timestamp = self.audio_index as u32 * 960;
            if elapsed.saturating_sub(self.audio_index as u128 * 20) >= 200 {
                self.observation.skipped += 1;
            } else {
                let bytes = &self.source.audio_packets[self.audio_index];
                self.io.send(&wire(
                    self.audio.handle,
                    self.datagram,
                    self.rtp_seq,
                    timestamp,
                    self.source.ssrc,
                    self.audio
                        .pt
                        .ok_or("original audio RTP payload type missing")?,
                    bytes,
                ))?;
                self.datagram += 1;
                self.rtp_seq = self.rtp_seq.wrapping_add(1);
                self.observation.sent += 1;
                self.octets += bytes.len() as u32;
                if self.observation.sent == 1 || self.audio_index % 25 == 0 {
                    self.io
                        .send(&pack_sender_clock_datagram(&SenderClockDatagram {
                            stream_local_id: self.audio.handle,
                            seq: self.datagram,
                            report: SenderClockReport {
                                ssrc: self.source.ssrc,
                                rtp: timestamp,
                                ntp: ntp(self.unix + self.audio_index as u128 * 20_000),
                                packets: self.observation.sent,
                                octets: self.octets,
                                cname: "g6-replacement-target".into(),
                            },
                        })?)?;
                    self.datagram += 1;
                }
            }
            self.audio_index += 1;
        }
        if let (Some(video), Some(binding)) = (&self.source.video, self.video) {
            let pending = self
                .encoder
                .as_ref()
                .is_some_and(|encoder| encoder.pending());
            let original_us = self.video_index as u128 * 1_000_000 / 30;
            let due = self.start + Duration::from_micros(original_us as u64);
            if self.video_index < video.program.frame_count as usize
                && tokio::time::Instant::now() >= due
                && !pending
            {
                if !self.observation.video_open
                    || self.start.elapsed().as_micros().saturating_sub(original_us) >= 200_000
                {
                    self.encoded = None;
                    self.observation.skipped_video += 1;
                    self.video_index += 1;
                    self.first_video_key = true;
                } else if let Some(frame) = self.encoded.take() {
                    if frame.index as usize != self.video_index {
                        return Err("encoder changed original programme position".into());
                    }
                    let timestamp = frame.index * 3000;
                    let mut complete = true;
                    for packet in video::packets(
                        binding,
                        video.ssrc,
                        timestamp,
                        &mut self.video_seq,
                        &mut self.datagram,
                        &frame.bytes,
                    ) {
                        if self.start.elapsed().as_micros().saturating_sub(original_us) >= 200_000 {
                            complete = false;
                            self.observation.skipped_video += 1;
                            break;
                        }
                        self.io.send(&packet)?;
                        self.observation.sent_video += 1;
                        self.video_octets += (packet.len() - 20) as u32;
                    }
                    self.first_video_key = !complete;
                    if complete
                        && self
                            .last_video_clock
                            .is_none_or(|last| self.video_index - last >= 15)
                    {
                        self.io
                            .send(&pack_sender_clock_datagram(&SenderClockDatagram {
                                stream_local_id: binding.handle,
                                seq: self.datagram,
                                report: SenderClockReport {
                                    ssrc: video.ssrc,
                                    rtp: timestamp,
                                    ntp: ntp(self.unix + original_us),
                                    packets: self.observation.sent_video,
                                    octets: self.video_octets,
                                    cname: "g6-replacement-target".into(),
                                },
                            })?)?;
                        self.datagram += 1;
                        self.last_video_clock = Some(self.video_index);
                    }
                    self.video_index += 1;
                } else {
                    let deadline = (tokio::time::Instant::now() + Duration::from_millis(100))
                        .min(due + Duration::from_millis(200));
                    let feedback = self.io.keyframe(binding.handle, video.ssrc)?;
                    self.observation.video_feedback_requests += u32::from(feedback);
                    self.encoder
                        .as_mut()
                        .ok_or("missing live video encoder")?
                        .request(
                            self.video_index as u32,
                            self.first_video_key || feedback,
                            deadline,
                        )?;
                }
            }
        }
        Ok(())
    }
}
