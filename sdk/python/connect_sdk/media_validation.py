"""Closed bounded numeric media observations, not delivery/decode guarantees."""
WEBRTC_INPUT = (
    'source_packets',
    'maximum_source_payload_bytes',
    'source_sequence_gap_events',
    'missing_sequence_slots',
    'source_reordered_packets',
    'padding_packets',
    'oversize_payload',
    'invalid_descriptor',
    'late_or_conflicting_mapping',
    'marshal_failed',
    'normalized_oversize',
    'reorder_expirations',
    'source_reset_discards',
)
QUEUE = ("capacity_packets", "sampled_high_water_packets", "observations",
         "enqueue_full_observations", "stale_forwarding_discards")
FORWARDING = ("samples", "p50_us", "p95_us", "max_us", "missing_ingress_observations", "handoff_timeouts")
QUIC = ("samples", "maximum_attempted_datagram_bytes", "video_budget_observations",
        "maximum_source_video_rtp_bytes", "video_repacketized_inputs", "video_packetizer_discards",
        "video_no_datagram_budget", "accepted_datagrams", "too_large", "unsupported_by_peer", "disabled", "connection_lost")
NULLABLE = ("min_max_datagram_size_bytes", "max_max_datagram_size_bytes", "min_path_mtu_bytes",
            "max_path_mtu_bytes", "min_video_rtp_budget_bytes", "max_video_rtp_budget_bytes")


def closed(value, required, optional=()):
    return isinstance(value, dict) and set(required) <= value.keys() <= set(required) | set(optional)


def natural(value):
    return type(value) is int and 0 <= value <= 9007199254740991


def label(value):
    return isinstance(value, str) and 1 <= len(value) <= 128 and all(c in "abcdefghijklmnopqrstuvwxyz0123456789-" for c in value)


def counters(value, keys):
    return all(natural(value[key]) for key in keys)


def queue(value):
    if not closed(value, ("stage", "kind", "unit", *QUEUE, "max_forwarded_frame_age_ms", "frame_age_boundary")):
        return False
    age = value["max_forwarded_frame_age_ms"]
    return (all(label(value[key]) for key in ("stage", "kind", "unit")) and counters(value, QUEUE)
            and value["sampled_high_water_packets"] <= value["capacity_packets"]
            and ((age is None and value["frame_age_boundary"] is None) or
                 (natural(age) and value["frame_age_boundary"] == "dequeue-before-handoff-or-write")))


def forwarding(value):
    return (closed(value, ("stage", *FORWARDING, "percentile_method")) and label(value["stage"])
            and counters(value, FORWARDING) and value["percentile_method"] == "log2-bucket-upper-bound"
            and value["p50_us"] <= value["p95_us"])


def bounded(value, check):
    return isinstance(value, list) and len(value) <= 128 and all(check(item) for item in value)


BUFFER = ("capacity_bytes", "observations", "minimum_available_bytes", "maximum_available_bytes",
          "sampled_high_water_bytes", "capacity_mismatch_observations", "send_exceeded_sampled_available_bytes")


def send_buffer(value):
    return (closed(value, ("scope", *BUFFER, "evictions_observed", "queued_age_observed"))
            and value["scope"] == "shared-audio-video-peer-buffer" and counters(value, BUFFER)
            and value["evictions_observed"] is False and value["queued_age_observed"] is False
            and value["minimum_available_bytes"] <= value["maximum_available_bytes"])



CONGESTION = ('congestion_samples', 'maximum_sampled_connection_lost_packets', 'maximum_sampled_connection_congestion_events', 'maximum_sampled_connection_sent_packets')
CONGESTION_NULLABLE = ('minimum_sampled_rtt_us', 'maximum_sampled_rtt_us', 'minimum_sampled_cwnd_bytes', 'maximum_sampled_cwnd_bytes')

def congestion(value):
    keys = (*CONGESTION, *CONGESTION_NULLABLE)
    if not any(key in value for key in keys): return True
    if not all(key in value for key in keys) or not counters(value, CONGESTION): return False
    if value['congestion_samples'] == 0:
        return all(value[key] is None for key in CONGESTION_NULLABLE)
    return (all(natural(value[key]) for key in CONGESTION_NULLABLE)
            and value['minimum_sampled_rtt_us'] <= value['maximum_sampled_rtt_us']
            and value['minimum_sampled_cwnd_bytes'] <= value['maximum_sampled_cwnd_bytes'])


def quic(value):
    return (
        closed(value, (*QUIC, *NULLABLE), ("send_buffer", *CONGESTION, *CONGESTION_NULLABLE))
        and counters(value, QUIC)
        and congestion(value)
        and ("send_buffer" not in value or send_buffer(value["send_buffer"]))
        and all(value[key] is None or natural(value[key]) for key in NULLABLE)
    )


def valid_media(value):
    required = ("scope", "queue_depth_observation", "forwarding_boundary", "queues", "forwarding")
    return (
        closed(value, required, ("quic", "webrtc_video_input"))
        and value["scope"] == "server-process"
        and value["queue_depth_observation"] == "sampled-at-enqueue-and-pump-poll"
        and value["forwarding_boundary"] == "adapter-frame-publication-to-core-destination-queue-handoff"
        and bounded(value["queues"], queue)
        and bounded(value["forwarding"], forwarding)
        and ("webrtc_video_input" not in value or (
            closed(value["webrtc_video_input"], WEBRTC_INPUT)
            and counters(value["webrtc_video_input"], WEBRTC_INPUT)
        ))
        and ("quic" not in value or quic(value["quic"]))
    )
