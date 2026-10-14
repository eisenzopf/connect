//! Map upstream bounded numeric observations into public Health diagnostics.
use connect_protocol::{
    MediaDiagnostics, MediaForwardObservation, MediaQueueObservation, QuicSendBufferObservation,
    QuicTransmissionObservation, WebRtcVideoInputObservation,
};
use rvoip_core::media_diagnostics::MediaDiagnosticsSnapshot;

pub(crate) fn snapshot(value: MediaDiagnosticsSnapshot) -> MediaDiagnostics {
    MediaDiagnostics {
        scope: "server-process".into(),
        queue_depth_observation: "sampled-at-enqueue-and-pump-poll".into(),
        forwarding_boundary: "adapter-frame-publication-to-core-destination-queue-handoff".into(),
        queues: value
            .queues
            .into_iter()
            .map(|(stage, value)| MediaQueueObservation {
                stage: stage.name().into(),
                kind: stage.kind().into(),
                unit: stage.unit().into(),
                capacity_packets: value.capacity_packets,
                sampled_high_water_packets: value.sampled_high_water_packets,
                observations: value.observations,
                enqueue_full_observations: value.enqueue_full_observations,
                stale_forwarding_discards: value.stale_forwarding_discards,
                max_forwarded_frame_age_ms: value.max_forwarded_frame_age_ms,
                frame_age_boundary: value
                    .max_forwarded_frame_age_ms
                    .map(|_| "dequeue-before-handoff-or-write".into()),
            })
            .collect(),
        forwarding: value
            .forwarding
            .into_iter()
            .map(|(stage, value)| MediaForwardObservation {
                stage: stage.name().into(),
                samples: value.samples,
                p50_us: value.p50_us,
                p95_us: value.p95_us,
                max_us: value.max_us,
                percentile_method: "log2-bucket-upper-bound".into(),
                missing_ingress_observations: value.missing_ingress_observations,
                handoff_timeouts: value.handoff_timeouts,
            })
            .collect(),
        webrtc_video_input: value
            .webrtc_video_input
            .map(|value| WebRtcVideoInputObservation {
                source_packets: value.source_packets,
                maximum_source_payload_bytes: value.maximum_source_payload_bytes,
                source_sequence_gap_events: value.source_sequence_gap_events,
                missing_sequence_slots: value.missing_sequence_slots,
                source_reordered_packets: value.source_reordered_packets,
                padding_packets: value.padding_packets,
                oversize_payload: value.oversize_payload,
                invalid_descriptor: value.invalid_descriptor,
                late_or_conflicting_mapping: value.late_or_conflicting_mapping,
                marshal_failed: value.marshal_failed,
                normalized_oversize: value.normalized_oversize,
                reorder_expirations: value.reorder_expirations,
                source_reset_discards: value.source_reset_discards,
            }),
        quic: value.quic.map(|value| QuicTransmissionObservation {
            congestion_samples: value.congestion_samples,
            minimum_sampled_rtt_us: value.minimum_sampled_rtt_us,
            maximum_sampled_rtt_us: value.maximum_sampled_rtt_us,
            minimum_sampled_cwnd_bytes: value.minimum_sampled_cwnd_bytes,
            maximum_sampled_cwnd_bytes: value.maximum_sampled_cwnd_bytes,
            maximum_sampled_connection_lost_packets: value.maximum_sampled_connection_lost_packets,
            maximum_sampled_connection_congestion_events: value
                .maximum_sampled_connection_congestion_events,
            maximum_sampled_connection_sent_packets: value.maximum_sampled_connection_sent_packets,

            send_buffer: value
                .send_buffer
                .filter(|buffer| buffer.observations > 0)
                .map(|buffer| QuicSendBufferObservation {
                    scope: "shared-audio-video-peer-buffer".into(),
                    capacity_bytes: buffer.capacity_bytes,
                    observations: buffer.observations,
                    minimum_available_bytes: buffer.minimum_available_bytes,
                    maximum_available_bytes: buffer.maximum_available_bytes,
                    sampled_high_water_bytes: buffer.sampled_high_water_bytes,
                    capacity_mismatch_observations: buffer.capacity_mismatch_observations,
                    send_exceeded_sampled_available_bytes: buffer
                        .send_exceeded_sampled_available_bytes,
                    evictions_observed: false,
                    queued_age_observed: false,
                }),
            samples: value.samples,
            min_max_datagram_size_bytes: value.min_max_datagram_size_bytes,
            max_max_datagram_size_bytes: value.max_max_datagram_size_bytes,
            min_path_mtu_bytes: value.min_path_mtu_bytes,
            max_path_mtu_bytes: value.max_path_mtu_bytes,
            maximum_attempted_datagram_bytes: value.maximum_attempted_datagram_bytes,
            video_budget_observations: value.video_budget_observations,
            min_video_rtp_budget_bytes: value.min_video_rtp_budget_bytes,
            max_video_rtp_budget_bytes: value.max_video_rtp_budget_bytes,
            maximum_source_video_rtp_bytes: value.maximum_source_video_rtp_bytes,
            video_repacketized_inputs: value.video_repacketized_inputs,
            video_packetizer_discards: value.video_packetizer_discards,
            video_no_datagram_budget: value.video_no_datagram_budget,
            accepted_datagrams: value.accepted_datagrams,
            too_large: value.too_large,
            unsupported_by_peer: value.unsupported_by_peer,
            disabled: value.disabled,
            connection_lost: value.connection_lost,
        }),
    }
}
