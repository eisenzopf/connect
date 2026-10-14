/** Process-local observations, never delivery receipts or remote decode proof. */
export interface MediaDiagnostics {
    scope: "server-process";
    queue_depth_observation: "sampled-at-enqueue-and-pump-poll";
    forwarding_boundary: "adapter-frame-publication-to-core-destination-queue-handoff";
    queues: MediaQueueObservation[];
    forwarding: MediaForwardObservation[];
    quic?: QuicTransmissionObservation;
    webrtc_video_input?: WebRtcVideoInputObservation;
}
/** Pre-normalization observations. A sequence skip is not proven network loss. */
export interface WebRtcVideoInputObservation {
    source_packets: number;
    maximum_source_payload_bytes: number;
    source_sequence_gap_events: number;
    missing_sequence_slots: number;
    source_reordered_packets: number;
    padding_packets: number;
    oversize_payload: number;
    invalid_descriptor: number;
    late_or_conflicting_mapping: number;
    marshal_failed: number;
    normalized_oversize: number;
    reorder_expirations: number;
    source_reset_discards: number;
}
export interface MediaQueueObservation {
    stage: string;
    kind: string;
    unit: string;
    capacity_packets: number;
    sampled_high_water_packets: number;
    observations: number;
    enqueue_full_observations: number;
    stale_forwarding_discards: number;
    max_forwarded_frame_age_ms: number | null;
    frame_age_boundary: "dequeue-before-handoff-or-write" | null;
}
export interface MediaForwardObservation {
    stage: string;
    samples: number;
    p50_us: number;
    p95_us: number;
    max_us: number;
    percentile_method: "log2-bucket-upper-bound";
    missing_ingress_observations: number;
    handoff_timeouts: number;
}
export interface QuicTransmissionObservation {
    send_buffer?: QuicSendBufferObservation;
    samples: number;
    min_max_datagram_size_bytes: number | null;
    max_max_datagram_size_bytes: number | null;
    min_path_mtu_bytes: number | null;
    max_path_mtu_bytes: number | null;
    maximum_attempted_datagram_bytes: number;
    video_budget_observations: number;
    min_video_rtp_budget_bytes: number | null;
    max_video_rtp_budget_bytes: number | null;
    maximum_source_video_rtp_bytes: number;
    video_repacketized_inputs: number;
    video_packetizer_discards: number;
    video_no_datagram_budget: number;
    accepted_datagrams: number;
    too_large: number;
    unsupported_by_peer: number;
    disabled: number;
    connection_lost: number;
}
/** Shared encrypted DATAGRAM byte queue; older enqueued data may be evicted by Quinn. */
export interface QuicSendBufferObservation {
    scope: "shared-audio-video-peer-buffer";
    capacity_bytes: number;
    observations: number;
    minimum_available_bytes: number;
    maximum_available_bytes: number;
    sampled_high_water_bytes: number;
    capacity_mismatch_observations: number;
    send_exceeded_sampled_available_bytes: number;
    evictions_observed: false;
    queued_age_observed: false;
}
/** Closed diagnostic shape prevents extra payloads, addresses or credentials from escaping. */
export declare function validMediaDiagnostics(value: unknown): value is MediaDiagnostics;
