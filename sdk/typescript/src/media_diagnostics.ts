import { object } from "./validation.js";

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
const webrtcInputCounters = [
    "source_packets",
    "maximum_source_payload_bytes",
    "source_sequence_gap_events",
    "missing_sequence_slots",
    "source_reordered_packets",
    "padding_packets",
    "oversize_payload",
    "invalid_descriptor",
    "late_or_conflicting_mapping",
    "marshal_failed",
    "normalized_oversize",
    "reorder_expirations",
    "source_reset_discards",
];

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
    /** Sampled physical connection extrema; cumulative counters are maxima, not sums. */
    congestion_samples?: number;
    maximum_sampled_connection_lost_packets?: number;
    maximum_sampled_connection_congestion_events?: number;
    maximum_sampled_connection_sent_packets?: number;
    minimum_sampled_rtt_us?: number | null;
    maximum_sampled_rtt_us?: number | null;
    minimum_sampled_cwnd_bytes?: number | null;
    maximum_sampled_cwnd_bytes?: number | null;

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
const sendBufferCounters = [
    "capacity_bytes",
    "observations",
    "minimum_available_bytes",
    "maximum_available_bytes",
    "sampled_high_water_bytes",
    "capacity_mismatch_observations",
    "send_exceeded_sampled_available_bytes",
];
function validSendBuffer(value: unknown): boolean {
    return (
        closed(value, [
            "scope",
            ...sendBufferCounters,
            "evictions_observed",
            "queued_age_observed",
        ]) &&
        value.scope === "shared-audio-video-peer-buffer" &&
        counters(value, sendBufferCounters) &&
        value.evictions_observed === false &&
        value.queued_age_observed === false &&
        Number(value.minimum_available_bytes) <=
            Number(value.maximum_available_bytes)
    );
}

const queueCounters = [
    "capacity_packets",
    "sampled_high_water_packets",
    "observations",
    "enqueue_full_observations",
    "stale_forwarding_discards",
];
const forwardingCounters = [
    "samples",
    "p50_us",
    "p95_us",
    "max_us",
    "missing_ingress_observations",
    "handoff_timeouts",
];
const quicCounters = [
    "samples",
    "maximum_attempted_datagram_bytes",
    "video_budget_observations",
    "maximum_source_video_rtp_bytes",
    "video_repacketized_inputs",
    "video_packetizer_discards",
    "video_no_datagram_budget",
    "accepted_datagrams",
    "too_large",
    "unsupported_by_peer",
    "disabled",
    "connection_lost",
];
const quicNullable = [
    "min_max_datagram_size_bytes",
    "max_max_datagram_size_bytes",
    "min_path_mtu_bytes",
    "max_path_mtu_bytes",
    "min_video_rtp_budget_bytes",
    "max_video_rtp_budget_bytes",
];
const natural = (value: unknown) =>
    Number.isSafeInteger(value) && Number(value) >= 0;
const label = (value: unknown) =>
    typeof value === "string" && /^[a-z0-9-]{1,128}$/.test(value);
const closed = (
    value: unknown,
    required: string[],
    optional: string[] = [],
): value is Record<string, unknown> =>
    object(value) &&
    required.every((key) => key in value) &&
    Object.keys(value).every(
        (key) => required.includes(key) || optional.includes(key),
    );
const counters = (value: Record<string, unknown>, keys: string[]) =>
    keys.every((key) => natural(value[key]));
const bounded = (value: unknown, valid: (item: unknown) => boolean) =>
    Array.isArray(value) && value.length <= 128 && value.every(valid);

const congestionCounters = [
    "congestion_samples",
    "maximum_sampled_connection_lost_packets",
    "maximum_sampled_connection_congestion_events",
    "maximum_sampled_connection_sent_packets",
];
const congestionNullable = [
    "minimum_sampled_rtt_us",
    "maximum_sampled_rtt_us",
    "minimum_sampled_cwnd_bytes",
    "maximum_sampled_cwnd_bytes",
];
function validCongestion(value: Record<string, unknown>): boolean {
    const keys = [...congestionCounters, ...congestionNullable];
    if (!keys.some((key) => key in value)) return true;
    if (
        !keys.every((key) => key in value) ||
        !counters(value, congestionCounters)
    )
        return false;
    if (value.congestion_samples === 0)
        return congestionNullable.every((key) => value[key] === null);
    return (
        congestionNullable.every((key) => natural(value[key])) &&
        Number(value.minimum_sampled_rtt_us) <=
            Number(value.maximum_sampled_rtt_us) &&
        Number(value.minimum_sampled_cwnd_bytes) <=
            Number(value.maximum_sampled_cwnd_bytes)
    );
}
function validQuic(value: unknown): boolean {
    return (
        closed(
            value,
            [...quicCounters, ...quicNullable],
            ["send_buffer", ...congestionCounters, ...congestionNullable],
        ) &&
        validCongestion(value) &&
        counters(value, quicCounters) &&
        (!("send_buffer" in value) || validSendBuffer(value.send_buffer)) &&
        quicNullable.every((key) => value[key] === null || natural(value[key]))
    );
}

/** Closed diagnostic shape prevents extra payloads, addresses or credentials from escaping. */
export function validMediaDiagnostics(
    value: unknown,
): value is MediaDiagnostics {
    return (
        closed(
            value,
            [
                "scope",
                "queue_depth_observation",
                "forwarding_boundary",
                "queues",
                "forwarding",
            ],
            ["quic", "webrtc_video_input"],
        ) &&
        value.scope === "server-process" &&
        value.queue_depth_observation === "sampled-at-enqueue-and-pump-poll" &&
        value.forwarding_boundary ===
            "adapter-frame-publication-to-core-destination-queue-handoff" &&
        bounded(
            value.queues,
            (item) =>
                closed(item, [
                    "stage",
                    "kind",
                    "unit",
                    ...queueCounters,
                    "max_forwarded_frame_age_ms",
                    "frame_age_boundary",
                ]) &&
                [item.stage, item.kind, item.unit].every(label) &&
                counters(item, queueCounters) &&
                (item.max_forwarded_frame_age_ms === null
                    ? item.frame_age_boundary === null
                    : natural(item.max_forwarded_frame_age_ms) &&
                      item.frame_age_boundary ===
                          "dequeue-before-handoff-or-write") &&
                Number(item.sampled_high_water_packets) <=
                    Number(item.capacity_packets),
        ) &&
        bounded(
            value.forwarding,
            (item) =>
                closed(item, [
                    "stage",
                    ...forwardingCounters,
                    "percentile_method",
                ]) &&
                label(item.stage) &&
                counters(item, forwardingCounters) &&
                item.percentile_method === "log2-bucket-upper-bound" &&
                Number(item.p50_us) <= Number(item.p95_us),
        ) &&
        (!("webrtc_video_input" in value) ||
            (closed(value.webrtc_video_input, webrtcInputCounters) &&
                counters(value.webrtc_video_input, webrtcInputCounters))) &&
        (!("quic" in value) || validQuic(value.quic))
    );
}
