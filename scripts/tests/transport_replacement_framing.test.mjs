import test from "node:test";
import assert from "node:assert/strict";
import { physicalAvSegment } from "../lib/transport_replacement_observation.mjs";
import { decodeNativeVideoEpoch } from "../../sdk/typescript/dist/browser_video_wire.js";

const expected = { programStartUnixUs: 1_800_000_000_000_000,
    audioStreamId: "audio", sendVideoStreamId: "camera", receiveVideoStreamId: "selected" };
function capture(tagged, selected, length) {
    const bytes = new Uint8Array(length), view = new DataView(bytes.buffer);
    bytes[0] = 1; bytes[1] = tagged ? 1 : 0;
    view.setUint16(2, 3); view.setUint32(4, 1);
    const offset = tagged ? 24 : 8;
    if (tagged) bytes.fill(1, 8, 24);
    bytes[offset] = 0x80; bytes[offset + 1] = 0x80 | 96;
    view.setUint16(offset + 2, 1); view.setUint32(offset + 4, 3000); view.setUint32(offset + 8, 42);
    bytes[offset + 12] = 0x10; // VP8 partition start; following byte is a keyframe.
    const video_epoch = { epoch: "01".repeat(16), incarnation: "02".repeat(16), revision: 1,
        ssrc: 42, first_sequence: 1, first_timestamp: 3000 };
    const binding = (strm_id, kind, direction, stream_local_id) => ({ strm_id, kind, direction, stream_local_id,
        codec: { name: kind === "audio" ? "opus" : "vp8", params: {
            sample_rate: kind === "audio" ? 48000 : 90000, channels: kind === "audio" ? 1 : 0,
            payload_type: kind === "audio" ? 111 : 96 } },
        ...(selected && kind === "video" ? { video_feedback: { profile: "rtcp-pli-epoch/1",
            original_operation: "move", original_offer: "offer", stream_generation: 1, connection_generation: 2 } } : {}) });
    return { format: "connect/native-replacement-peer-observation/1", passed: true,
        original_client_joined: true, video_enabled: true, original_root_completed_observed: true,
        video_transfer_admitted: true, operation_completed_inferred: false, decoded_media_verified: false,
        program_start_unix_us: expected.programStartUnixUs, root_completed_observed_programme_us: 1000,
        negotiated_streams: [binding("audio", "audio", "sendrecv", 1),
            binding("camera", "video", "recvonly", 2), binding("selected", "video", "sendonly", 3)],
        received: [{ elapsed_us: 1000, original_root_completed_observed: true,
            datagram: [...bytes], ...(tagged ? { video_epoch } : {}) }] };
}

test("physical capture accepts selected epoch framing without enlarging RTP or ordinary bounds", () => {
    const original = capture(true, true, 1204), before = structuredClone(original);
    const decoded = decodeNativeVideoEpoch(Uint8Array.from(original.received[0].datagram));
    assert.equal(decoded.packet.payload.length, 1168);
    const result = physicalAvSegment(original, expected);
    assert.deepEqual(original, before);
    assert.equal(result.original_datagram_bytes, 1204);
    assert.equal(result.video[0].rows[0].timestamp, 3000);
    assert.equal(Buffer.from(result.video[0].rows[0].payload, "base64").length, 1167);
    assert.equal(result.decoded_media_verified, false);
    assert.equal(physicalAvSegment(capture(false, false, 1188), expected).original_datagram_bytes, 1188);
    for (const wrong of [capture(true, true, 1205), capture(false, false, 1189),
        capture(true, false, 1204)]) assert.throws(() => physicalAvSegment(wrong, expected));
    const foreign = structuredClone(original); foreign.received[0].datagram[3] = 1;
    assert.throws(() => physicalAvSegment(foreign, expected), /selected video allocation/);
    const quarantined = structuredClone(original);
    quarantined.received.unshift({ ...structuredClone(original.received[0]), elapsed_us: 500,
        original_root_completed_observed: false });
    const excluded = physicalAvSegment(quarantined, expected);
    assert.deepEqual(excluded.quarantined_precompletion_rows, [0]);
    assert.equal(excluded.video[0].rows.length, 1);
    quarantined.received[0].datagram.push(0);
    assert.throws(() => physicalAvSegment(quarantined, expected), /framing bound/);
});
