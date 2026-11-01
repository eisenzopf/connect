import assert from "node:assert/strict";
import test from "node:test";
import { takeMediaSendBatch } from "./ui/media-send.mjs";
import { rtpTimestamp } from "./ui/media-clock.mjs";

const frame = (fragments, timestampUs = 1000000, queuedAt = 0) => ({ bytes: Uint8Array.of(fragments), timestampUs, queuedAt });
function route(frames) {
    let sequence = 0; const packetized = [];
    return { queue: frames, queuedBytes: frames.length, pendingFrames: 0, rate: 90000, packetized,
        sender: { packetize(bytes, timestamp) {
            packetized.push(timestamp);
            return Array.from({ length: bytes[0] }, () => ({ sequence: sequence++, timestamp }));
        } } };
}
test("a fragmented frame keeps its preallocated sequence order and clock across bounded requests", () => {
    const r = route([frame(10), frame(1, 1033333)]), drop = () => assert.fail("fresh frame dropped");
    const first = takeMediaSendBatch(r, () => 10, drop);
    assert.deepEqual(first.packets.map(v => v.datagram.sequence), [0, 1, 2, 3, 4, 5, 6, 7]);
    assert.equal(first.frameComplete, false); assert.equal(r.pendingFrames, 0);
    const next = takeMediaSendBatch(r, () => 30, drop);
    assert.deepEqual(next.packets.map(v => v.datagram.sequence), [8, 9, 10]);
    assert.deepEqual(next.packets.map(v => v.datagram.timestamp), [90000, 90000, rtpTimestamp(1033333, 90000)]);
    assert.equal(next.frameComplete, true); assert.equal(r.pendingFrames, 2);
    assert.deepEqual(r.packetized, [90000, rtpTimestamp(1033333, 90000)]);
    assert.equal(r.queue.length, 0); assert.equal(r.queuedBytes, 0);
    assert.equal(next.packets[0].remainingMs, 170);
});
test("expiry discards unsent fragments at the original age without repacketizing or refreshing the next source", () => {
    const r = route([frame(10), frame(1, 1033333, 100)]); let drops = 0;
    takeMediaSendBatch(r, () => 0, () => drops++);
    const next = takeMediaSendBatch(r, () => 200, () => drops++);
    assert.equal(drops, 1); assert.deepEqual(next.packets.map(v => v.datagram.sequence), [10]);
    assert.equal(next.packets[0].remainingMs, 100);
    assert.equal(next.packets[0].datagram.timestamp, rtpTimestamp(1033333, 90000));
    assert.equal(next.frameComplete, true); assert.equal(r.queuedBytes, 0);
});
