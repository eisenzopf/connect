"""Prospective Firefox-only original source-frame witness; closed metadata only.

Never regrades an old profile, fits an offset, decodes SRTP, or creates authority.
The caller still owns the original browser/capture/server joins and every gate.
"""
import copy
import hashlib
import ipaddress
import json
import math
import os
import stat
import struct
import time
from pathlib import Path

PROFILE = "connect/native-rtc-original-source-frame/1"
MAP_PROFILE = "connect/rtc-original-video-mapping/1"
MAX_PAIRS, MAX_HEADERS = 16, 16384


class WitnessError(ValueError):
    pass


def need(ok, reason):
    if not ok:
        raise WitnessError(reason)


def uint(value, bits):
    return type(value) is int and 0 <= value < 1 << bits


def delta(a, b, bits=32):
    value = (a - b) % (1 << bits)
    need(value != 1 << (bits - 1), "ambiguous-modular-coordinate")
    return value if value < 1 << (bits - 1) else value - (1 << bits)


def regular(path, maximum):
    fd = os.open(path, os.O_RDONLY | os.O_NONBLOCK | os.O_NOFOLLOW)
    try:
        before = os.fstat(fd)
        need(stat.S_ISREG(before.st_mode) and 0 < before.st_size <= maximum, "regular-bounded-original-required")
        with os.fdopen(fd, "rb") as source:
            fd = None
            yield source, before
            fields=("st_dev","st_ino","st_mode","st_size","st_mtime_ns","st_ctime_ns","st_nlink","st_uid")
            need(all(getattr(os.fstat(source.fileno()),key)==getattr(before,key) for key in fields), "original-input-changed")
    finally:
        if fd is not None:
            os.close(fd)


# A contextmanager keeps the SAME original descriptor open through validation.
from contextlib import contextmanager
regular = contextmanager(regular)


def read_json(path, maximum=16 * 1024**2):
    with regular(path, maximum) as (source, _):
        raw = source.read(maximum + 1)
        need(len(raw) <= maximum, "json-cap")
        return json.loads(raw), hashlib.sha256(raw).hexdigest()


def mapping_summaries(path, original_server_pid):
    result = []
    digest = hashlib.sha256()
    with regular(path, 256 * 1024**2) as (source, _):
        while line := source.readline(256 * 1024 + 1):
            need(len(line) <= 256 * 1024, "mapping-line-cap")
            digest.update(line)
            if b"RTC original video mapping witness" not in line:
                continue
            marker = b'witness={"profile":"connect/rtc-original-video-mapping/1"'
            at = line.find(marker)
            need(at >= 0, "mapping-export-format")
            value, _ = json.JSONDecoder().raw_decode(line[at + len(b"witness="):].decode("utf8"))
            need(len(result) < 128 and value.get("profile") == MAP_PROFILE, "mapping-summary-cap")
            need(value.get("process") == original_server_pid and value.get("original_timeline_retired") is True,
                 "mapping-original-owner-mismatch")
            epochs = value.get("epochs")
            need(isinstance(epochs, list) and 0 <= len(epochs) <= 32, "mapping-epoch-cap")
            need([e.get("ordinal") for e in epochs] == list(range(len(epochs))), "mapping-epoch-order")
            result.append(value)
    need(result and len({v["timeline"] for v in result}) == len(result), "mapping-missing-or-reexported")
    return result, digest.hexdigest()


def stable_stats(cycle, owner, direction, first, last, *, stream=None, track=None, ssrc=None):
    samples = cycle["decoded_media"]["rtc_media_pipeline"]["samples"]
    before = [s for s in samples if s["completed_seconds"] <= first]
    after = [s for s in samples if s["requested_seconds"] >= last]
    need(before and after, "owner-statistics-do-not-bracket-pair")
    lo, hi = before[-1], after[0]
    need(first - lo["completed_seconds"] <= 2 and hi["requested_seconds"] - last <= 2,
         "owner-statistics-too-distant")
    selected = samples[samples.index(lo):samples.index(hi) + 1]
    values = []
    for sample in selected:
        rows = [r for group in sample["peers"] for r in group if r.get("connection_id") == owner
                and r.get("kind") == "video" and r.get("type") == direction
                and r.get("codec", {}).get("mime_type", "").lower() == "video/vp8"]
        need(len(rows) == 1, "owner-statistics-ambiguous")
        row = rows[0]
        original = row.get("original_track") or {}
        need(original.get("connection_id") == owner and isinstance(original.get("track_id"), str)
             and isinstance(original.get("stream_id"), str), "original-peer-track-unproved")
        need((stream is None or original["stream_id"] == stream)
             and (track is None or original["track_id"] == track)
             and (ssrc is None or row.get("ssrc") == ssrc), "original-track-or-ssrc-changed")
        path, codec = row.get("selected_path") or {}, row["codec"]
        need(path.get("nominated") is True and path.get("succeeded") is True, "selected-path-unproved")
        for end in (path.get("local"), path.get("remote")):
            need(isinstance(end, dict) and end.get("protocol") == "udp"
                 and uint(end.get("port"), 16) and end["port"] > 0, "selected-udp-tuple-unavailable")
            ipaddress.ip_address(end["address"])
        need(codec.get("clock_rate") == 90000 and uint(codec.get("payload_type"), 7)
             and uint(row.get("ssrc"), 32), "original-vp8-clock-profile")
        values.append({"track": original, "path": path, "ssrc": row["ssrc"], "codec": codec})
    need(all(v == values[0] for v in values), "source-or-receiver-state-changed")
    return values[0]


def epoch_for_pair(summaries, receiver, first_ts, last_ts):
    found = []
    for summary in summaries:
        epochs = summary["epochs"]
        for i, epoch in enumerate(epochs):
            if epoch.get("target_ssrc") != receiver["ssrc"] or epoch.get("target_logical") != receiver["track"]["stream_id"]:
                continue
            need(summary.get("complete") is True and summary.get("omitted")==summary.get("invalid")==0,"selected-mapping-incomplete")
            need(all(type(epoch.get(k)) is int and 0 < epoch[k] < 2**64 for k in
                     ("source_generation","target_generation","target_connection_generation")),"mapping-generation-invalid")
            need(all(uint(epoch.get(k), bits) for k, bits in (("input_sequence", 16), ("output_sequence", 16),
                 ("input_timestamp", 32), ("output_timestamp", 32), ("sequence_offset", 16), ("timestamp_offset", 32))),
                 "mapping-coordinate-invalid")
            if delta(first_ts, epoch["output_timestamp"]) < 0:
                continue
            next_epoch = epochs[i + 1] if i + 1 < len(epochs) else None
            if next_epoch is not None and delta(last_ts, next_epoch["output_timestamp"]) >= 0:
                continue
            # No transition near the exception window, including target rebind.
            if delta(first_ts, epoch["output_timestamp"]) < 90000:
                continue
            if next_epoch is not None and delta(next_epoch["output_timestamp"], last_ts) <= 90000:
                continue
            need(epoch["target_pt"] == receiver["codec"]["payload_type"], "target-profile-mismatch")
            found.append((summary["timeline"], epoch))
    need(len(found) == 1, "mapping-missing-or-ambiguous")
    return found[0]


def packet_flow(stats, outbound):
    a, b = stats["path"]["local"], stats["path"]["remote"]
    if not outbound:
        a, b = b, a
    return (str(ipaddress.ip_address(a["address"])), a["port"],
            str(ipaddress.ip_address(b["address"])), b["port"], stats["ssrc"], stats["codec"]["payload_type"])


def capture_headers(path, capture, windows, *, cycles=1):
    need(capture.get("observed") is True and capture.get("original_joined") is True
         and capture.get("exit_code") == 0 and not capture.get("forced")
         and capture.get("kernel_dropped_packets") == 0, "original-capture-not-complete")
    kept, count = [], 0
    deadline = time.monotonic() + 30
    with regular(path, 256 * 1024**2 * cycles) as (source, st):
        need(st.st_size == capture.get("pcap_bytes"), "capture-size-changed")
        digest = hashlib.file_digest(source, "sha256").hexdigest()
        need(digest == capture.get("pcap_sha256"), "capture-digest-changed")
        source.seek(0)
        head = source.read(24)
        formats = {b"\xd4\xc3\xb2\xa1": ("<", 1000000), b"\xa1\xb2\xc3\xd4": (">", 1000000),
                   b"\x4d\x3c\xb2\xa1": ("<", 1000000000), b"\xa1\xb2\x3c\x4d": (">", 1000000000)}
        need(len(head) == 24 and head[:4] in formats, "classic-pcap-required")
        order, resolution = formats[head[:4]]
        major, minor, _, _, snaplen, link = struct.unpack(order + "HHiIII", head[4:])
        need((major, minor) == (2, 4) and link in (0, 1) and snaplen <= 262144, "capture-format")
        while source.tell() < st.st_size:
            raw = source.read(16)
            need(len(raw) == 16 and count < 200000 * cycles, "capture-record-cap")
            sec, frac, length, original = struct.unpack(order + "IIII", raw)
            count += 1
            if count % 4096 == 0:
                need(time.monotonic() < deadline, "closed-metadata-time-bound")
            need(frac < resolution and 14 <= length == original <= snaplen, "capture-truncated")
            start, end = source.tell(), source.tell() + length
            need(end <= st.st_size, "capture-body-truncated")
            prefix = source.read(14 if link == 1 else 4)
            ip_at = source.tell()
            version = source.read(1)[0] >> 4
            source.seek(ip_at)
            if link == 1 and prefix[12:14] not in (b"\x08\x00", b"\x86\xdd"):
                source.seek(end); continue
            if version == 4:
                ip = source.read(20); ihl = (ip[0] & 15) * 4
                if ip[9] != 17 or int.from_bytes(ip[6:8], "big") & 0x3fff:
                    source.seek(end); continue
                need(20 <= ihl <= 60, "ip-header")
                address_a, address_b = str(ipaddress.ip_address(ip[12:16])), str(ipaddress.ip_address(ip[16:20]))
                source.seek(ip_at + ihl)
            elif version == 6:
                ip = source.read(40)
                if ip[6] != 17:
                    source.seek(end); continue
                address_a, address_b = str(ipaddress.ip_address(ip[8:24])), str(ipaddress.ip_address(ip[24:40]))
            else:
                source.seek(end); continue
            udp = source.read(8)
            need(len(udp) == 8, "udp-header")
            port_a, port_b, udp_len, _ = struct.unpack("!HHHH", udp)
            if udp_len < 20:
                source.seek(end); continue
            rtp_at = source.tell(); header = source.read(12)
            need(len(header) == 12, "rtp-header")
            flags, pt_marker, seq, ts, ssrc = struct.unpack("!BBHII", header)
            if flags >> 6 != 2 or 192 <= pt_marker <= 223:
                source.seek(end); continue
            flow = (address_a, port_a, address_b, port_b, ssrc, pt_marker & 127)
            wanted = [(low, high) for f, low, high in windows if f == flow]
            if not wanted or not any(-90000 <= delta(ts, low) and delta(ts, high) <= 90000 for low, high in wanted):
                source.seek(end); continue
            header_len = 12 + (flags & 15) * 4
            source.seek(rtp_at + header_len)
            if flags & 16:
                extension = source.read(4)
                need(len(extension) == 4, "rtp-extension-header")
                header_len += 4 + int.from_bytes(extension[2:4], "big") * 4
            need(not flags & 32 and header_len < udp_len - 8 and rtp_at + udp_len - 8 <= end,
                 "selected-rtp-unsupported-or-truncated")
            need(len(kept) < MAX_HEADERS, "witness-global-header-cap")
            kept.append({"flow": flow, "seq": seq, "ts": ts, "marker": bool(pt_marker & 128),
                         "body_bytes": udp_len - 8 - header_len,
                         "unix_ns": sec * 1000000000 + frac * (1000000000 // resolution), "record": count})
            source.seek(end)
    need(count == capture.get("original_packets_captured"), "original-capture-count")
    return kept


def prove_headers(rows, source_flow, target_flow, epoch, input_ts, output_ts):
    source = [r for r in rows if tuple(r["flow"]) == source_flow]
    target = [r for r in rows if tuple(r["flow"]) == target_flow]
    need(source and len(source) == len(target), "primary-window-not-bijective")
    index = {}
    for row in target:
        key = (row["seq"], row["ts"], row["marker"])
        need(key not in index, "target-duplicate-packet")
        index[key] = row
    matched = []
    source_keys = set()
    for a in source:
        key = (a["seq"], a["ts"], a["marker"])
        need(key not in source_keys, "source-duplicate-packet")
        source_keys.add(key)
        key = ((a["seq"] + epoch["sequence_offset"]) % 65536,
               (a["ts"] + epoch["timestamp_offset"]) % 2**32, a["marker"])
        b = index.get(key)
        need(b is not None and a["body_bytes"] == b["body_bytes"], "header-mapping-or-packetization-differs")
        need(0 < b["unix_ns"] - a["unix_ns"] < 200000000, "primary-transit-unproved")
        matched.append((a, b))
    need(len({b["record"] for _, b in matched}) == len(target), "nonunique-primary-association")
    need(any(delta(r["ts"], input_ts[0]) < 0 for r in source)
         and any(delta(r["ts"], input_ts[1]) > 0 for r in source), "original-neighbors-missing")
    for ts, target_ts in zip(input_ts, output_ts):
        a = [r for r in source if r["ts"] == ts]
        b = [r for r in target if r["ts"] == target_ts]
        need(a and len(a) == len(b) and sum(r["marker"] for r in a) == 1, "original-frame-group-incomplete")
        ordered = sorted(a, key=lambda r: delta(r["seq"], a[0]["seq"], 16))
        need(ordered[-1]["marker"] and all(delta(y["seq"], x["seq"], 16) == 1 for x, y in zip(ordered, ordered[1:])),
             "original-frame-fragment-gap")
    need(input_ts[0] != input_ts[1] and delta(input_ts[1], input_ts[0]) > 0, "repeated-or-backward-original-source-frame")
    return {"matched_packets": len(matched), "original_timestamps": input_ts, "target_timestamps": output_ts}


def prepare_pair(cycle, route, pair, summaries):
    a, b = pair["first"], pair["current"]
    pa, pb = a["presentation"], b["presentation"]
    need(a["owner"] == b["owner"] and pa["element_id"] == pb["element_id"]
         and pa["stream_id"] == pb["stream_id"] and pa["track_id"] == pb["track_id"]
         and pb["presentedFrames"] > pa["presentedFrames"], "cross-owner-or-presentation-replay")
    need(1 <= a["callback_seconds"] <= b["callback_seconds"] < 61, "outside-public-quality-window")
    receiver = stable_stats(cycle, a["owner"], "inbound-rtp", a["callback_seconds"], b["callback_seconds"],
                            stream=pa["stream_id"], track=pa["track_id"])
    timeline, epoch = epoch_for_pair(summaries, receiver, pa["rtpTimestamp"], pb["rtpTimestamp"])
    sources = cycle["programme"]["sources"]["sources"]
    selected = [value for value in sources if value.get("index")==route["source_route_index"]]
    need(len(selected)==1,"original-source-route-ambiguous")
    source_record = selected[0]
    need(any(v["connection_id"] == epoch["source_connection"] for v in source_record["attachments"]),
         "mapping-source-not-original-authenticated-route")
    source = stable_stats(cycle, epoch["source_connection"], "outbound-rtp", a["callback_seconds"], b["callback_seconds"],
                          stream=epoch["source_logical"], ssrc=epoch["source_ssrc"])
    source_cipher,receiver_cipher=source["path"].get("srtp_cipher"),receiver["path"].get("srtp_cipher")
    if source_cipher is not None and receiver_cipher is not None:
        need(source_cipher==receiver_cipher,"different-srtp-wire-profiles")
    output_ts = [pa["rtpTimestamp"], pb["rtpTimestamp"]]
    input_ts = [(v - epoch["timestamp_offset"]) % 2**32 for v in output_ts]
    return {"timeline": timeline, "epoch": epoch, "source_flow": packet_flow(source, True),
            "target_flow": packet_flow(receiver, False), "input_ts": input_ts, "output_ts": output_ts}


def finalize(report, browser_folder, server_log, pcap_path):
    """Return a NEW final observation; never mutate original raw cycle artifacts."""
    need(report.get("transport") == "firefox-wss-webrtc" and report.get("stage") == "quality", "firefox-quality-profile-only")
    need(report.get("browser_exit_code") == 0 and report.get("browser_owner_joined") is True, "original-browser-collection-not-joined0")
    raw = report["browser_observation"]
    need(raw.get("source_revision")==report.get("source_revision") and raw.get("transport")==report.get("transport"),
         "original-browser-source-or-transport-differs")
    need(raw.get("source_frame_witness_profile") == PROFILE and raw.get("collection_passed") is True,
         "not-a-prospective-complete-collection")
    servers = report.get("servers", [])
    need(servers and all(s.get("original_joined") is True and s.get("original_exit_code") == 0
         and s.get("original_process_group_joined") is True for s in servers), "original-server-joins-unproved")
    actual = [s for s in servers if s["name"] == "materialized-server"]
    need(len(actual) == 1, "original-server-owner-ambiguous")
    summaries, log_sha = mapping_summaries(server_log, actual[0]["original_pid"])
    cycles, pairs, windows = [], [], []
    for archive in raw["resource_cycles"]:
        folder = Path(browser_folder) / "cycles" / ("cycle-" + str(archive["index"]))
        cycle, digest = read_json(folder / "cycle-observation.json")
        need(digest == archive["observation_sha256"] and cycle.get("collection_passed") is True,
             "original-cycle-hash-or-collection")
        need(cycle.get("cycle", {}).get("workload_id") == archive["workload_id"], "original-cycle-owner")
        final_cycle = copy.deepcopy(cycle)
        for route in final_cycle["decoded_media"]["routes"]:
            candidate = route["video"].get("original_source_frame_witness")
            need(candidate and candidate.get("profile") == PROFILE and candidate.get("collection_passed") is True,
                 "incomplete-or-unrequested-source-candidate")
            if candidate["pairs"]:
                need(candidate.get("initial_ordinary_receiver") is True, "prepared-receiver-remains-strict")
                need(not any(move.get("source_index")==route["route"] for move in cycle["programme"]["moves"]),
                     "moved-receiver-remains-strict")
                original_receivers=[v for v in cycle["source_startup"]["sources"] if v.get("index")==route["route"]]
                need(len(original_receivers)==1 and len(original_receivers[0]["attachments"])==1,
                     "initial-receiver-owner-unproved")
                original_owner=original_receivers[0]["attachments"][0]["connection_id"]
                need(all(pair["first"]["owner"]==pair["current"]["owner"]==original_owner for pair in candidate["pairs"]),
                     "replacement-receiver-remains-strict")
            for pair in candidate["pairs"]:
                need(len(pairs) < MAX_PAIRS, "global-source-pair-cap")
                prepared = prepare_pair(cycle, route, pair, summaries)
                pairs.append((route, prepared))
                windows.extend([(prepared["source_flow"], *prepared["input_ts"]),
                                (prepared["target_flow"], *prepared["output_ts"])])
            route["video"]["raw_draw_duplicate_frames"] = route["video"]["duplicate_frames"]
        cycles.append(final_cycle)
    headers = capture_headers(pcap_path, report["packet_capture"], windows, cycles=len(cycles))
    facts = []
    for route, pair in pairs:
        fact = prove_headers(headers, pair["source_flow"], pair["target_flow"], pair["epoch"], pair["input_ts"], pair["output_ts"])
        facts.append({"route": route["route"], "timeline": pair["timeline"], "epoch": pair["epoch"]["ordinal"], **fact})
    # Resolve ONLY the source-frame predicate. Existing row flags, bins, counts,
    # transitions, gaps, identity, invalid, audio and AV facts remain untouched.
    for cycle in cycles:
        for route in cycle["decoded_media"]["routes"]:
            v = route["video"]
            v["actual_source_duplicate_frames"] = 0
            v["source_frame_profile"] = PROFILE
            v["duplicate_frames"] = 0
            v["passed"] = v["original_source_frame_witness"]["non_duplicate_passed"] is True
            route["passed"] = not route["invalid"] and route["audio"]["passed"] and v["passed"] and route["sync"]["status"] == "passed"
        cycle["decoded_media"]["passed"] = all(r["passed"] for r in cycle["decoded_media"]["routes"])
        cycle["programme"]["decoded_media"] = copy.deepcopy(cycle["decoded_media"])
        cycle["raw_quality_passed"] = cycle["passed"]
        cycle["passed"] = cycle["collection_passed"] and cycle["decoded_media"]["passed"]
        if cycle.get("failure") == {"stage": "decoded-media-quality", "name": "ProfileBudgetsFailed"}:
            cycle["raw_failure"] = cycle.pop("failure")
    final = copy.deepcopy(raw)
    first = next((c for c in cycles if not c["passed"]), cycles[0])
    final.update(first)
    final["passed"] = all(c["passed"] for c in cycles)
    final["raw_quality_pending_original_source_witness"] = final.get("quality_pending_original_source_witness", False)
    final["quality_pending_original_source_witness"] = False
    final["cycles"] = [{**s, "raw_passed": s["passed"], "passed": c["passed"]} for s, c in zip(raw["cycles"], cycles)]
    for summary in final["cycles"]:
        if summary.get("failure")=={"stage":"decoded-media-quality","name":"ProfileBudgetsFailed"}:
            summary["raw_failure"]=summary.pop("failure")
    if final.get("failure")=={"stage":"decoded-media-quality","name":"ProfileBudgetsFailed"}:
        final["raw_failure"]=final.pop("failure")
    if final["passed"]:
        final.pop("first_failed_cycle",None)
    final["original_source_frame_adjudication"] = {"profile": PROFILE, "complete": True, "passed": final["passed"],
        "raw_browser_sha256": report["browser_observation_sha256"], "pcap_sha256": report["packet_capture"]["pcap_sha256"],
        "server_log_sha256": log_sha, "source_revision": report["source_revision"],
        "original_server_pid": actual[0]["original_pid"], "pairs": facts,
        "scope": "Prospective original-source frame predicate only; changing/FPS/AV/raw rows unchanged; no historical regrade"}
    return final, cycles


def report_bound(report):
    """JSON-only grader fence; never treats pending collection as media pass."""
    observation = report.get('browser_observation', {})
    def marked(value):
        if not isinstance(value,dict):return False
        if any(key in value for key in ('source_frame_witness_profile','original_source_frame_adjudication')):return True
        decoded=value.get('decoded_media',{})
        return any(any(key in row.get('video',{}) for key in
            ('source_frame_profile','original_source_frame_witness','actual_source_duplicate_frames','raw_draw_duplicate_frames'))
            for row in decoded.get('routes',[]))
    markers=('source_frame_adjudication' in report or 'raw_browser_observation_sha256' in report
        or marked(observation) or any(marked(c) for c in report.get('cycle_observations',[])))
    enabled=report.get('diagnostic_options', {}).get('rtc_source_frame_witness')
    if enabled is not True:
        return not markers and enabled in (None,False)
    if not markers:return False
    value = report.get('source_frame_adjudication', {})
    return (report.get('transport') == 'firefox-wss-webrtc' and report.get('stage') == 'quality'
            and report.get('browser_exit_code') == 0 and report.get('browser_owner_joined') is True
            and all(report.get(k) is True for k in ('source_inputs_unchanged','sdk_dist_inputs_unchanged','executable_unchanged'))
            and value.get('profile') == PROFILE and value.get('complete') is True and value.get('passed') is True
            and value.get('source_revision') == report.get('source_revision')
            and value.get('pcap_sha256') == report.get('packet_capture', {}).get('pcap_sha256')
            and value.get('raw_browser_sha256') == report.get('raw_browser_observation_sha256')
            and isinstance(value.get('raw_browser_sha256'), str) and len(value['raw_browser_sha256']) == 64
            and all(c in '0123456789abcdef' for c in value['raw_browser_sha256'])
            and observation.get('source_frame_witness_profile') == PROFILE
            and observation.get('original_source_frame_adjudication') == value
            and observation.get('passed') is True)
