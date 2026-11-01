import { rtpTimestamp } from "./media-clock.mjs";

// Pull only already queued original frames. A fragmented frame can span HTTP
// requests; no sender report may overtake its preallocated packet sequences.
export function takeMediaSendBatch(route, now, drop) {
    const packets = [];
    while (route.queue.length && packets.length < 8) {
        const frame = route.queue[0], remainingMs = Math.floor(200 - (now() - frame.queuedAt));
        if (remainingMs <= 0) {
            route.queue.shift(); route.queuedBytes -= frame.bytes.length; drop(); continue;
        }
        frame.datagrams ??= route.sender.packetize(frame.bytes, rtpTimestamp(frame.timestampUs, route.rate));
        frame.cursor ??= 0;
        while (frame.cursor < frame.datagrams.length && packets.length < 8)
            packets.push({ datagram: frame.datagrams[frame.cursor++], remainingMs });
        if (frame.cursor === frame.datagrams.length) {
            route.queue.shift(); route.queuedBytes -= frame.bytes.length; route.pendingFrames++;
        }
    }
    return { packets, frameComplete: !route.queue[0]?.datagrams };
}
