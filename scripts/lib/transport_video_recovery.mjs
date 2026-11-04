// Fixture workers use only the selected SDK stream's typed feedback authority.
import assert from "node:assert/strict";
import { Vp8Assembler } from "../../sdk/typescript/dist/browser.js";

export class PublisherVideoFeedback {
    #stream;
    #signal;
    #hint;
    received = 0;
    consumed = 0;
    expired = 0;
    constructor(stream, signal) { this.#stream = stream; this.#signal = signal; }

    async run(until) {
        try {
            while (!this.#signal.aborted && performance.now() < until) {
                try {
                    // One pending read and one retained original hint. Reading
                    // the next hint does not consume or renew the previous one.
                    this.#hint = await this.#stream.receiveVideoFeedback({ timeoutMs: 500, signal: this.#signal });
                    this.received++;
                } catch (error) {
                    if (this.#signal.aborted && error.kind === "cancelled") return;
                    if (error.kind === "deadline" && error.certainty === "read-only") continue;
                    throw error;
                }
            }
        } finally { this.#hint = undefined; }
    }

    async take(ssrc, timeoutMs) {
        const hint = this.#hint;
        this.#hint = undefined;
        if (!hint || hint.ssrc !== ssrc || this.#signal.aborted) return undefined;
        // Consumption rechecks current Rust binding/history. An uncertain RPC
        // is propagated, never retried or converted into encoder permission.
        const permit = await this.#stream.consumeVideoFeedback(hint, { timeoutMs, signal: this.#signal });
        if (!permit?.isFresh()) { this.expired++; return undefined; }
        this.consumed++;
        return permit;
    }
}

export class VideoReceiveHistory {
    #assembler = new Vp8Assembler();
    #identity;
    #epoch;
    #lastFrame;
    #timestamp;
    #ready = false;
    #need = false;
    #lastRequest = -Infinity;
    #history = 0;
    constructor(requireEpoch) { this.requireEpoch = requireEpoch; }
    get pendingFragments() { return this.#assembler.queuedFragments; }

    #reset() {
        this.#assembler.clear(); this.#lastFrame = undefined;
        this.#ready = false; this.#history++;
    }
    gap() { this.#reset(); this.#need = this.#identity !== undefined; }
    expire(now = performance.now()) {
        if (this.#assembler.expire(now)) this.gap();
    }
    takeRequest(now = performance.now()) {
        if (!this.#need || now - this.#lastRequest < 100) return false;
        this.#lastRequest = now;
        return true;
    }
    push(streamId, packet, now = performance.now()) {
        const epoch = packet.videoEpoch;
        if (this.requireEpoch) assert.ok(epoch, "selected video requires authenticated packet history");
        const identity = JSON.stringify([streamId, packet.header.ssrc,
            epoch?.incarnation, epoch?.epoch, epoch?.revision]);
        if (identity !== this.#identity) {
            this.#reset(); this.#identity = identity; this.#epoch = epoch;
            this.#need = false; this.#timestamp = undefined;
        }
        if (this.#timestamp !== undefined) {
            const distance = (packet.header.timestamp - this.#timestamp) >>> 0;
            if (!distance || distance >= 0x80000000) return undefined;
        }
        this.expire(now);
        const frame = this.#assembler.push({ ...packet.header,
            sequence: packet.header.sequenceNumber, payload: packet.payload }, now);
        if (!frame) return undefined;
        this.#timestamp = frame.timestamp;
        if (this.#lastFrame) {
            const distance = (frame.timestamp - this.#lastFrame.timestamp) >>> 0;
            if (!distance || distance >= 0x80000000) return undefined;
            if (frame.firstSequence !== ((this.#lastFrame.lastSequence + 1) & 0xffff)) {
                this.#reset(); this.#need = true;
            }
        }
        this.#lastFrame = frame;
        if (frame.key) { this.#ready = true; this.#need = false; }
        if (!this.#ready) { this.#need = true; return undefined; }
        return { ...frame, decoderHistory: this.#history, videoEpoch: this.#epoch };
    }
}
