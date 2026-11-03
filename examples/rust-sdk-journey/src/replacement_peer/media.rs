//! One original packet programme progresses while signed control is pending.
use super::*;
use connect_client::canonical::NativeDatagrams;
use std::future::Future;

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
}

pub(super) struct Pump<'a> {
    io: NativeDatagrams,
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
}
impl<'a> Pump<'a> {
    pub fn new(
        io: NativeDatagrams,
        source: &'a Source,
        audio: &'a selected::Binding,
        video: Option<&'a selected::Binding>,
        observation: &'a mut Observation,
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
        })
    }

    /// The original control future remains pinned and is never restarted. Its
    /// cancellation/uncertainty semantics remain the client's responsibility.
    pub async fn during<F: Future>(&mut self, control: F) -> Result<F::Output> {
        tokio::pin!(control);
        loop {
            if tokio::time::Instant::now() >= self.until {
                return Err("original media programme ended during control".into());
            }
            let before = self.observation.rows.len() as u32
                + self.observation.sent
                + self.observation.sent_video;
            tokio::select! {
                biased;
                value = &mut control => return Ok(value),
                result = self.step(self.until) => {
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
            .filter(|v| self.video_index < v.frames.len())
            .map(|_| self.start + Duration::from_micros(self.video_index as u64 * 1_000_000 / 30))
            .unwrap_or(self.until);
        let next = next_audio.min(next_video).min(control_tick).min(self.until);
        if let Some(packet) = timeline::receive_before_tick(next, self.io.receive()).await {
            let packet = packet?;
            self.observation.captured_bytes += packet.len();
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
            self.observation.rows.push(
                json!({"elapsed_us":self.start.elapsed().as_micros(),"datagram":packet,
                "original_root_completed_observed":self.observation.root_observed}),
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
            if self.video_index < video.frames.len() && tokio::time::Instant::now() >= next_video {
                let frame = &video.frames[self.video_index];
                let original_us = self.video_index as u128 * 1_000_000 / 30;
                if !self.observation.video_open
                    || self.start.elapsed().as_micros().saturating_sub(original_us) >= 200_000
                    || (self.first_video_key && !video::key(frame))
                {
                    self.observation.skipped_video += 1;
                } else {
                    let timestamp = self.video_index as u32 * 3000;
                    let mut complete = true;
                    for packet in video::packets(
                        binding,
                        video.ssrc,
                        timestamp,
                        &mut self.video_seq,
                        &mut self.datagram,
                        frame,
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
                    if complete {
                        self.first_video_key = false;
                        if self
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
                    }
                }
                self.video_index += 1;
            }
        }
        Ok(())
    }
}
