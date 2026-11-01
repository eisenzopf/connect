import test from "node:test";
import assert from "node:assert/strict";
import { physicalAvSegment } from "../lib/transport_replacement_observation.mjs";
import { encodeRtcpSenderReport } from "../../sdk/typescript/dist/browser_rtcp.js";

const expected = { programStartUnixUs: 1_800_000_000_000_000,
    audioStreamId: "own-audio", sendVideoStreamId: "own-video", receiveVideoStreamId: "remote-video" };
function fixture(received) {
    const binding = (strm_id, kind, direction, stream_local_id) => ({ strm_id, kind, direction, stream_local_id,
        codec: { name: kind === "audio" ? "opus" : "vp8", params: {
            sample_rate: kind === "audio" ? 48000 : 90000, channels: kind === "audio" ? 1 : 0,
            payload_type: kind === "audio" ? 111 : 96 } } });
    return { format: "connect/native-replacement-peer-observation/1", passed: true,
        original_client_joined: true, video_enabled: true, original_root_completed_observed: true,
        video_controls_completed: true, operation_completed_inferred: false, decoded_media_verified: false,
        program_start_unix_us: expected.programStartUnixUs,
        negotiated_streams: [binding("remote-video", "video", "sendonly", 9),
            binding("own-video", "video", "sendrecv", 8), binding("own-audio", "audio", "sendrecv", 7)], received };
}
function rtp({ handle = 7, pt = 111, ssrc = 12, timestamp = 960, sequence = 1,
    marker = true, payload = [1, 2] } = {}) {
    const b = new Uint8Array(20 + payload.length), v = new DataView(b.buffer);
    b.set([1, 0]); v.setUint16(2, handle); b[8] = 0x80; b[9] = pt | (marker ? 128 : 0);
    v.setUint16(10, sequence); v.setUint32(12, timestamp); v.setUint32(16, ssrc); b.set(payload, 20);
    return [...b];
}
const row = (elapsed_us, datagram, completed = true) => ({ elapsed_us, datagram,
    original_root_completed_observed: completed });
const video = options => rtp({ handle: 9, pt: 96, payload: [0x10, 0, 1, 2], ...options });
function clock() {
    const body = encodeRtcpSenderReport({ ssrc: 12, ntp: 1234n, rtpTimestamp: 960,
        packets: 2, octets: 4, cname: "synthetic" });
    const b = new Uint8Array(body.length + 8); b[0] = 1; new DataView(b.buffer).setUint16(2, 7); b.set(body, 8);
    return [...b];
}

test("pre-completion packets cannot initialize later audio, clock or VP8 state", () => {
    const capture = fixture([
        row(100, rtp(), false), row(200, clock(), false),
        row(300, video({ marker: false, payload: [0x10, 0, 1] }), false),
        row(400, video({ sequence: 2, payload: [0, 2, 3] })),
        row(500, rtp({ timestamp: 1920 })), row(600, clock()),
        row(700, video({ sequence: 3, timestamp: 3000 })),
    ]), original = structuredClone(capture);
    const result = physicalAvSegment(capture, expected);
    assert.deepEqual(capture, original);
    assert.deepEqual(result.quarantined_precompletion_rows, [0, 1, 2]);
    assert.equal(result.audio[0].rows.length, 1);
    assert.equal(result.audio[0].rows[0].elapsed_ms, .5);
    assert.equal(result.audio[0].rows[0].timestamp, 1920);
    assert.equal(result.clock_reports.audio.length, 1);
    assert.equal(result.clock_reports.audio[0].physical_observation_row, 5);
    assert.equal(result.video[0].rows.length, 1);
    assert.equal(result.video[0].rows[0].timestamp, 3000);
    assert.equal(result.decoded_media_verified, false);
    assert.equal(result.full_timeline_decoded, false);
});

test("physical SSRC histories stay separate even when RTP timestamps repeat", () => {
    const capture = fixture([row(2_000_000, rtp()), row(2_000_010, rtp({ ssrc: 13 })),
        row(2_000_020, video()), row(2_000_030, video({ ssrc: 13 }))]);
    const result = physicalAvSegment(capture, expected);
    for (const medium of ["audio", "video"]) {
        assert.deepEqual(result[medium].map(v => v.ssrc), [12, 13]);
        assert.deepEqual(result[medium].map(v => v.rows[0].timestamp), [960, 960]);
    }
    assert.equal(result.audio[0].rows[0].elapsed_ms, 2000);
    assert.equal(result.audio[1].rows[0].elapsed_ms, 2000.01);
});

test("assembly expiry uses original capture arrival intervals during fast offline replay", () => {
    const result = physicalAvSegment(fixture([
        row(1_000, video({ marker: false, payload: [0x10, 0, 1] })),
        row(202_000, video({ sequence: 2, payload: [0, 2, 3] })),
    ]), expected);
    assert.equal(result.video[0].rows.length, 0);
    assert.equal(result.pending_video_fragments, 1);
});

test("unknown handles, changed negotiated resources and payload types refuse", () => {
    for (const change of [
        c => { c.received[0].datagram = rtp({ handle: 8 }); },
        c => { c.received[0].datagram = rtp({ pt: 96 }); },
        c => { c.negotiated_streams[0].strm_id = "wrong-camera"; },
        c => { c.negotiated_streams[0].stream_local_id = 7; },
        c => { c.negotiated_streams[0].codec.params.sample_rate = 48000; },
    ]) {
        const capture = fixture([row(100, rtp())]); change(capture);
        assert.throws(() => physicalAvSegment(capture, expected));
    }
});

test("a failed or unjoined original, changed program and regressed boundary refuse", () => {
    for (const change of [
        c => { c.passed = false; }, c => { c.original_client_joined = false; },
        c => { c.original_root_completed_observed = false; },
        c => { c.program_start_unix_us++; },
        c => { c.received[1].original_root_completed_observed = false; },
        c => { c.received[1].elapsed_us = 0; },
        c => { c.received[1].elapsed_us = 62_000_001; },
        c => { c.received.forEach(r => { r.original_root_completed_observed = false; }); },
    ]) {
        const capture = fixture([row(100, rtp()), row(200, rtp())]); change(capture);
        assert.throws(() => physicalAvSegment(capture, expected));
    }
});

test("malformed RTCP cannot fall back to RTP or become a clock observation", () => {
    const datagram = rtp(); datagram[9] = 200;
    assert.throws(() => physicalAvSegment(fixture([row(100, datagram)]), expected));
});
