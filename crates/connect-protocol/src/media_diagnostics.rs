//! Bounded, authenticated numeric media observations from one server process.
//! These are diagnostics, not delivery receipts or decoded-media proof.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaDiagnostics {
    pub scope: String,
    pub queue_depth_observation: String,
    pub forwarding_boundary: String,
    pub queues: Vec<MediaQueueObservation>,
    pub forwarding: Vec<MediaForwardObservation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quic: Option<QuicTransmissionObservation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webrtc_video_input: Option<WebRtcVideoInputObservation>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaQueueObservation {
    pub stage: String,
    pub kind: String,
    /// Raw WebRTC queues carry track events; other stages carry frame packets.
    pub unit: String,
    pub capacity_packets: usize,
    pub sampled_high_water_packets: usize,
    pub observations: u64,
    /// This queue was full at an enqueue, not an end-to-end loss estimate.
    pub enqueue_full_observations: u64,
    pub stale_forwarding_discards: u64,
    /// Present only at stages enforcing a forwarding-age bound.
    pub max_forwarded_frame_age_ms: Option<u64>,
    /// Age is sampled at dequeue before the corresponding handoff/write.
    /// It excludes waiting inside the write and remote network/decoder time.
    pub frame_age_boundary: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaForwardObservation {
    pub stage: String,
    pub samples: u64,
    pub p50_us: u64,
    pub p95_us: u64,
    pub max_us: u64,
    pub percentile_method: String,
    /// A missing receipt is explicit and never synthesized as a zero sample.
    pub missing_ingress_observations: u64,
    pub handoff_timeouts: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuicTransmissionObservation {
    /// Samples at media sends; connection counters below are maxima across
    /// sampled physical connections, never sums or measured network-loss rates.
    #[serde(default)]
    pub congestion_samples: u64,
    #[serde(default)]
    pub minimum_sampled_rtt_us: Option<u64>,
    #[serde(default)]
    pub maximum_sampled_rtt_us: Option<u64>,
    #[serde(default)]
    pub minimum_sampled_cwnd_bytes: Option<u64>,
    #[serde(default)]
    pub maximum_sampled_cwnd_bytes: Option<u64>,
    #[serde(default)]
    pub maximum_sampled_connection_lost_packets: u64,
    #[serde(default)]
    pub maximum_sampled_connection_congestion_events: u64,
    #[serde(default)]
    pub maximum_sampled_connection_sent_packets: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send_buffer: Option<QuicSendBufferObservation>,
    pub samples: u64,
    pub min_max_datagram_size_bytes: Option<usize>,
    pub max_max_datagram_size_bytes: Option<usize>,
    pub min_path_mtu_bytes: Option<u16>,
    pub max_path_mtu_bytes: Option<u16>,
    pub maximum_attempted_datagram_bytes: usize,
    pub video_budget_observations: u64,
    pub min_video_rtp_budget_bytes: Option<usize>,
    pub max_video_rtp_budget_bytes: Option<usize>,
    pub maximum_source_video_rtp_bytes: usize,
    pub video_repacketized_inputs: u64,
    pub video_packetizer_discards: u64,
    pub video_no_datagram_budget: u64,
    /// Quinn accepted a datagram; this does not prove remote receipt.
    pub accepted_datagrams: u64,
    pub too_large: u64,
    pub unsupported_by_peer: u64,
    pub disabled: u64,
    pub connection_lost: u64,
}

/// Shared physical-peer DATAGRAM byte occupancy, independent of packet queues.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuicSendBufferObservation {
    pub scope: String,
    pub capacity_bytes: usize,
    pub observations: u64,
    pub minimum_available_bytes: usize,
    pub maximum_available_bytes: usize,
    pub sampled_high_water_bytes: usize,
    pub capacity_mismatch_observations: u64,
    pub send_exceeded_sampled_available_bytes: u64,
    pub evictions_observed: bool,
    pub queued_age_observed: bool,
}

/// Numeric observations before WebRTC VP8 normalization, without RTP bytes or IDs.
/// Sequence skips are local observations; they do not assert network packet loss.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebRtcVideoInputObservation {
    pub source_packets: u64,
    pub maximum_source_payload_bytes: usize,
    pub source_sequence_gap_events: u64,
    pub missing_sequence_slots: u64,
    pub source_reordered_packets: u64,
    pub padding_packets: u64,
    pub oversize_payload: u64,
    pub invalid_descriptor: u64,
    pub late_or_conflicting_mapping: u64,
    pub marshal_failed: u64,
    pub normalized_oversize: u64,
    pub reorder_expirations: u64,
    pub source_reset_discards: u64,
}
