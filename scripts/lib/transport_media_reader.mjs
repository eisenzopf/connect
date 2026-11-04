// One packet reader follows signed selection completion without replaying IO.
import assert from "node:assert/strict";
const readError = kind => Object.assign(Error("original media read " + kind), { kind, certainty: "read-only" });

export class SwitchingMediaReader {
    #current;
    #parent;
    #selection;
    #failure;
    #switches = 0;
    #cancelledReads = 0;
    #reads = new WeakMap();

    constructor(streamId, media, parent) {
        this.#current = { streamId, media, controller: new AbortController() };
        this.#parent = parent;
    }

    get observation() {
        return { completed_switches: this.#switches, original_read_waits_cancelled: this.#cancelledReads };
    }

    async switchWith(operationId, select) {
        assert.equal(this.#parent.aborted, false);
        assert.equal(this.#selection, undefined, "one original selection per reader at a time");
        assert.ok(typeof operationId === "string" && /^[A-Za-z0-9._~-]{1,128}$/.test(operationId));
        assert.equal(typeof select, "function");
        if (this.#failure) throw this.#failure;
        let settle;
        const observed = new Promise(done => { settle = done; });
        const pending = { operationId, observed };
        this.#selection = pending;
        try {
            // The caller owns the exact mutation, its signed receipt and current
            // native binding checks. No read or mutation is resubmitted here.
            const next = await select();
            assert.equal(this.#parent.aborted, false);
            assert.ok(typeof next.streamId === "string" && next.streamId.length > 0);
            assert.equal(typeof next.media.receivePacket, "function");
            const previous = this.#current;
            this.#current = { ...next, controller: new AbortController() };
            this.#switches++;
            previous.controller.abort();
            settle(true);
        } catch (failure) {
            this.#failure = failure;
            this.#current.controller.abort();
            settle(false);
            throw failure;
        } finally { if (this.#selection === pending) this.#selection = undefined; }
    }

    async receivePacket(options) {
        const until = performance.now() + options.timeoutMs;
        for (;;) {
            if (this.#failure) throw this.#failure;
            if (this.#parent.aborted) throw readError("cancelled");
            const remaining = Math.floor(until - performance.now());
            if (remaining <= 0) throw readError("deadline");
            const current = this.#current;
            try {
                const value = await current.media.receivePacket({ ...options,
                    timeoutMs: remaining,
                    signal: AbortSignal.any([this.#parent, current.controller.signal]) });
                const result = { value, streamId: current.streamId };
                this.#reads.set(result, current);
                return result;
            } catch (failure) {
                if (this.#failure) throw this.#failure;
                if (this.#parent.aborted) throw failure;
                if (current !== this.#current && current.controller.signal.aborted && failure.kind === "cancelled" && failure.certainty === "read-only") {
                    this.#cancelledReads++;
                    continue;
                }
                const pending = this.#selection;
                if (pending && failure.certainty === "read-only" && failure.kind === "resync-required") {
                    // An old receive binding can retire just before the same
                    // original selection's paired capture reaches its caller.
                    const aborted = new AbortController();
                    let timer;
                    const stopped = new Promise(done => {
                        this.#parent.addEventListener("abort", () => done(false), { once: true, signal: aborted.signal });
                        timer = setTimeout(() => done(false), Math.max(0, until - performance.now()));
                    });
                    try {
                        const confirmed = await Promise.race([pending.observed, stopped]);
                        if (this.#parent.aborted) throw readError("cancelled");
                        if (performance.now() >= until) throw readError("deadline");
                        if (!confirmed) throw this.#failure ?? failure;
                    } finally { clearTimeout(timer); aborted.abort(); }
                    continue;
                }
                throw failure;
            }
        }
    }

    async requestKeyframe(read, options) {
        const current = this.#reads.get(read);
        // Same Stream ID does not make a retired physical read current. Do not
        // retarget feedback across a selection, including one still in flight.
        if (!current || current !== this.#current || this.#selection || this.#failure || this.#parent.aborted)
            return undefined;
        return current.media.requestKeyframe({ ...options,
            signal: AbortSignal.any([this.#parent, current.controller.signal]) });
    }
}
