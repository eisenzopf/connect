//! Opt-in numeric observations for an independent decoder qualification.
use anyhow::{bail, Result};
use std::{
    path::Path,
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{fs::File, io::AsyncWriteExt};

pub async fn private_output(path: &Path) -> Result<File> {
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    Ok(options.open(path).await?)
}

/// The public SDK snapshot contains fixed stage labels and numeric observations.
/// Channel polling is sampled; it is not an exact every-enqueue peak.
pub fn sdk_media_json(session: &connect_client::NativeVideoSession) -> String {
    let snapshot = session.media_diagnostics();
    let queues = snapshot.queues.iter().map(|(stage, value)| {
        let age = value.max_forwarded_frame_age_ms.map_or("null".into(), |v| v.to_string());
        format!("{{\"stage\":\"{}\",\"kind\":\"{}\",\"unit\":\"{}\",\"capacity_packets\":{},\"sampled_high_water_packets\":{},\"observations\":{},\"enqueue_full_observations\":{},\"stale_forwarding_discards\":{},\"max_forwarded_frame_age_ms\":{age}}}",
            stage.name(),stage.kind(),stage.unit(),value.capacity_packets,value.sampled_high_water_packets,
            value.observations,value.enqueue_full_observations,value.stale_forwarding_discards)
    }).collect::<Vec<_>>().join(",");
    let quic = snapshot.quic.map_or("null".into(), |value| {
        let buffer = value.send_buffer.map_or("null".into(), |buffer| format!(
            "{{\"capacity_bytes\":{},\"observations\":{},\"minimum_available_bytes\":{},\"maximum_available_bytes\":{},\"sampled_high_water_bytes\":{},\"capacity_mismatch_observations\":{},\"send_exceeded_sampled_available_bytes\":{},\"evictions_observed\":false,\"queued_age_observed\":false}}",
            buffer.capacity_bytes,buffer.observations,buffer.minimum_available_bytes,buffer.maximum_available_bytes,
            buffer.sampled_high_water_bytes,buffer.capacity_mismatch_observations,buffer.send_exceeded_sampled_available_bytes));
        let min_budget=value.min_video_rtp_budget_bytes.map_or("null".into(), |v| v.to_string());
        let max_budget=value.max_video_rtp_budget_bytes.map_or("null".into(), |v| v.to_string());
        let congestion = format!("{{\"scope\":\"sampled physical-connection extrema; connection counters are maxima, not aggregate loss or eviction counts\",\"samples\":{},\"minimum_rtt_us\":{},\"maximum_rtt_us\":{},\"minimum_cwnd_bytes\":{},\"maximum_cwnd_bytes\":{},\"maximum_connection_lost_packets\":{},\"maximum_connection_congestion_events\":{},\"maximum_connection_sent_packets\":{}}}",
            value.congestion_samples, value.minimum_sampled_rtt_us.map_or("null".into(), |v|v.to_string()), value.maximum_sampled_rtt_us.map_or("null".into(), |v|v.to_string()), value.minimum_sampled_cwnd_bytes.map_or("null".into(), |v|v.to_string()), value.maximum_sampled_cwnd_bytes.map_or("null".into(), |v|v.to_string()), value.maximum_sampled_connection_lost_packets, value.maximum_sampled_connection_congestion_events, value.maximum_sampled_connection_sent_packets);
        let min_mtu=value.min_path_mtu_bytes.map_or("null".into(), |v| v.to_string());
        let max_mtu=value.max_path_mtu_bytes.map_or("null".into(), |v| v.to_string());
        let min_datagram=value.min_max_datagram_size_bytes.map_or("null".into(), |v| v.to_string());
        let max_datagram=value.max_max_datagram_size_bytes.map_or("null".into(), |v| v.to_string());
        format!("{{\"congestion\":{congestion},\"send_buffer\":{buffer},\"samples\":{},\"min_video_rtp_budget_bytes\":{min_budget},\"max_video_rtp_budget_bytes\":{max_budget},\"min_path_mtu_bytes\":{min_mtu},\"max_path_mtu_bytes\":{max_mtu},\"min_max_datagram_size_bytes\":{min_datagram},\"max_max_datagram_size_bytes\":{max_datagram},\"maximum_attempted_datagram_bytes\":{},\"video_budget_observations\":{},\"maximum_source_video_rtp_bytes\":{},\"video_repacketized_inputs\":{},\"video_packetizer_discards\":{},\"video_no_datagram_budget\":{},\"accepted_datagrams\":{},\"too_large\":{},\"unsupported_by_peer\":{},\"disabled\":{},\"connection_lost\":{}}}",
            value.samples,value.maximum_attempted_datagram_bytes,value.video_budget_observations,value.maximum_source_video_rtp_bytes,
            value.video_repacketized_inputs,value.video_packetizer_discards,value.video_no_datagram_budget,value.accepted_datagrams,
            value.too_large,value.unsupported_by_peer,value.disabled,value.connection_lost)
    });
    format!("{{\"scope\":\"native-session-handle\",\"queue_depth_observation\":\"sampled-at-sdk-send-receive-and-pump-poll\",\"ingress_age_boundary\":\"client-adapter-local-receipt-to-sdk-receive\",\"egress_age_boundary\":\"dequeue-before-handoff-or-write\",\"exact_ingress_enqueue_peak_observed\":false,\"queues\":[{queues}],\"quic\":{quic}}}")
}

struct Sample {
    arrival: f64,
    emitted: f64,
    timestamp: u32,
    presentation: u64,
    bytes: usize,
    keyframe: bool,
}
struct MuteTransition {
    kind: &'static str,
    muted: bool,
    seconds: f64,
}
pub struct Measurements {
    output: Option<File>,
    start: Instant,
    unix_start: f64,
    video: Vec<Sample>,
    audio: Vec<Sample>,
    packets: usize,
    maximum_rtp_bytes: usize,
    maximum_encoded_bytes: usize,
    camera_muted: bool,
    microphone_muted: bool,
    mute_transitions: Vec<MuteTransition>,
}
impl Measurements {
    pub async fn new(path: Option<&Path>) -> Result<Self> {
        Ok(Self {
            output: match path {
                Some(p) => Some(private_output(p).await?),
                None => None,
            },
            start: Instant::now(),
            unix_start: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs_f64(),
            video: Vec::new(),
            audio: Vec::new(),
            packets: 0,
            maximum_rtp_bytes: 0,
            maximum_encoded_bytes: 0,
            camera_muted: false,
            microphone_muted: false,
            mute_transitions: Vec::new(),
        })
    }
    pub fn begin(&mut self) {
        self.start = Instant::now();
        self.unix_start = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
    }
    pub fn elapsed(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }
    pub fn packet(&mut self, bytes: usize) {
        self.packets += 1;
        self.maximum_rtp_bytes = self.maximum_rtp_bytes.max(bytes);
    }
    pub fn mute_transition(&mut self, kind: &'static str, muted: bool) {
        let current = if kind == "video" {
            &mut self.camera_muted
        } else {
            &mut self.microphone_muted
        };
        if *current != muted {
            *current = muted;
            // One explicit schedule per kind produces at most four transitions.
            if self.mute_transitions.len() < 4 {
                self.mute_transitions.push(MuteTransition {
                    kind,
                    muted,
                    seconds: self.elapsed(),
                });
            }
        }
    }
    pub fn video_at(
        &mut self,
        timestamp: u32,
        presentation: u64,
        keyframe: bool,
        bytes: usize,
        received: Instant,
    ) -> Result<()> {
        self.maximum_encoded_bytes = self.maximum_encoded_bytes.max(bytes);
        if self.output.is_some() {
            if self.video.len() >= 10_000 {
                bail!("Video observation journal exceeds the bounded profile");
            }
            self.video.push(Sample {
                arrival: received.duration_since(self.start).as_secs_f64(),
                emitted: self.elapsed(),
                timestamp,
                presentation,
                bytes,
                keyframe,
            });
        }
        Ok(())
    }
    pub fn audio_at(&mut self, timestamp: u32, bytes: usize, received: Instant) -> Result<()> {
        if self.output.is_some() {
            if self.audio.len() >= 20_000 {
                bail!("Audio observation journal exceeds the bounded profile");
            }
            self.audio.push(Sample {
                arrival: received.duration_since(self.start).as_secs_f64(),
                emitted: self.elapsed(),
                timestamp,
                presentation: 0,
                bytes,
                keyframe: false,
            });
        }
        Ok(())
    }
    pub async fn finish(
        &mut self,
        teardown: f64,
        success: bool,
        end_reason: &str,
        assembly: connect_client::VideoAssemblyDiagnostics,
        playout_json: &str,
        sdk_json: &str,
    ) -> Result<()> {
        let elapsed = self.elapsed();
        let Some(output) = &mut self.output else {
            return Ok(());
        };
        let samples = |values: &[Sample]| {
            values
                .iter()
                .map(|s| {
                    format!(
            "{{\"arrival_seconds\":{},\"emit_seconds\":{},\"timestamp_rtp\":{},\"timestamp_ivf\":{},\"bytes\":{},\"keyframe\":{}}}",
            s.arrival, s.emitted, s.timestamp, s.presentation, s.bytes, s.keyframe)
                })
                .collect::<Vec<_>>()
                .join(",")
        };
        let assembly_json = format!("{{\"pending_frames\":{},\"queued_fragments\":{},\"peak_pending_frames\":{},\"peak_queued_fragments\":{},\"discarded_frames\":{},\"expired_frames\":{},\"completed_frames\":{}}}",
            assembly.pending_frames, assembly.queued_fragments, assembly.peak_pending_frames,
            assembly.peak_queued_fragments, assembly.discarded_frames, assembly.expired_frames, assembly.completed_frames);
        let maximum_hold = self
            .video
            .iter()
            .chain(&self.audio)
            .map(|sample| (sample.emitted - sample.arrival) * 1000.0)
            .fold(0.0_f64, f64::max);
        let transitions = self
            .mute_transitions
            .iter()
            .map(|event| {
                format!(
                    "{{\"kind\":\"{}\",\"muted\":{},\"seconds\":{}}}",
                    event.kind, event.muted, event.seconds
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let content = format!("{{\"schema\":\"connect-native-media-observation/1\",\"success\":{success},\"end_reason\":\"{end_reason}\",\"unix_start_seconds\":{},\"elapsed_seconds\":{elapsed},\"teardown_seconds\":{teardown},\"maximum_codec_emission_hold_ms\":{maximum_hold},\"terminal_live_buffers_discarded\":true,\"video_packets\":{},\"maximum_rtp_bytes\":{},\"maximum_encoded_frame_bytes\":{},\"assembly\":{assembly_json},\"playout\":{playout_json},\"sdk_media\":{sdk_json},\"mute_transitions\":[{transitions}],\"video\":[{}],\"audio\":[{}]}}\n",
            self.unix_start, self.packets, self.maximum_rtp_bytes, self.maximum_encoded_bytes,
            samples(&self.video), samples(&self.audio));
        output.write_all(content.as_bytes()).await?;
        output.flush().await?;
        Ok(())
    }
}
