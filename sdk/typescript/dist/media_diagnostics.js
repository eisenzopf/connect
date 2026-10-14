import { object } from "./validation.js";
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
const sendBufferCounters = [
    "capacity_bytes",
    "observations",
    "minimum_available_bytes",
    "maximum_available_bytes",
    "sampled_high_water_bytes",
    "capacity_mismatch_observations",
    "send_exceeded_sampled_available_bytes",
];
function validSendBuffer(value) {
    return (closed(value, [
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
            Number(value.maximum_available_bytes));
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
const natural = (value) => Number.isSafeInteger(value) && Number(value) >= 0;
const label = (value) => typeof value === "string" && /^[a-z0-9-]{1,128}$/.test(value);
const closed = (value, required, optional = []) => object(value) &&
    required.every((key) => key in value) &&
    Object.keys(value).every((key) => required.includes(key) || optional.includes(key));
const counters = (value, keys) => keys.every((key) => natural(value[key]));
const bounded = (value, valid) => Array.isArray(value) && value.length <= 128 && value.every(valid);
function validQuic(value) {
    return (closed(value, [...quicCounters, ...quicNullable], ["send_buffer"]) &&
        counters(value, quicCounters) &&
        (!("send_buffer" in value) || validSendBuffer(value.send_buffer)) &&
        quicNullable.every((key) => value[key] === null || natural(value[key])));
}
/** Closed diagnostic shape prevents extra payloads, addresses or credentials from escaping. */
export function validMediaDiagnostics(value) {
    return (closed(value, [
        "scope",
        "queue_depth_observation",
        "forwarding_boundary",
        "queues",
        "forwarding",
    ], ["quic", "webrtc_video_input"]) &&
        value.scope === "server-process" &&
        value.queue_depth_observation === "sampled-at-enqueue-and-pump-poll" &&
        value.forwarding_boundary ===
            "adapter-frame-publication-to-core-destination-queue-handoff" &&
        bounded(value.queues, (item) => closed(item, [
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
                Number(item.capacity_packets)) &&
        bounded(value.forwarding, (item) => closed(item, [
            "stage",
            ...forwardingCounters,
            "percentile_method",
        ]) &&
            label(item.stage) &&
            counters(item, forwardingCounters) &&
            item.percentile_method === "log2-bucket-upper-bound" &&
            Number(item.p50_us) <= Number(item.p95_us)) &&
        (!("webrtc_video_input" in value) ||
            (closed(value.webrtc_video_input, webrtcInputCounters) &&
                counters(value.webrtc_video_input, webrtcInputCounters))) &&
        (!("quic" in value) || validQuic(value.quic)));
}
