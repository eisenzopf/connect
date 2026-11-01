import test from "node:test";
import assert from "node:assert/strict";
import { SwitchingMediaReader } from "../lib/transport_media_reader.mjs";

const deferred = () => { let resolve; const promise = new Promise(done => { resolve = done; }); return { promise, resolve }; };
const failure = (kind, certainty = "read-only") => Object.assign(Error("original " + kind), { kind, certainty });
const stoppedRead = options => new Promise((_, reject) => {
    const stop = () => reject(failure("cancelled"));
    if (options.signal.aborted) stop(); else options.signal.addEventListener("abort", stop, { once: true });
});

test("twenty camera switches cancel only old local waits and each original selection runs once", async () => {
    const parent = new AbortController(); let reads = 0, selections = 0;
    const media = { receivePacket(options) { reads++; return stoppedRead(options); } };
    const reader = new SwitchingMediaReader("initial-camera", media, parent.signal);
    const packet = { kind: "rtp", marker: "actual-new-binding" };
    const reading = reader.receivePacket({ timeoutMs: 1000 });
    for (let i = 0; i < 20; i++) {
        await new Promise(setImmediate);
        await reader.switchWith("selection-" + i, async () => {
            selections++;
            return { streamId: "camera-" + i, media: i === 19 ? { async receivePacket() { reads++; return packet; } } : media };
        });
    }
    assert.deepEqual(await reading, { value: packet, streamId: "camera-19" });
    assert.equal(selections, 20); assert.equal(reads, 21);
    assert.deepEqual(reader.observation, { completed_switches: 20, original_read_waits_cancelled: 20 });
    assert.equal(parent.signal.aborted, false);
});

test("an old read-only binding refusal cannot outrun its pending original selection receipt", async () => {
    const parent = new AbortController(), selected = deferred(); let newReads = 0;
    const reader = new SwitchingMediaReader("old", { async receivePacket() { throw failure("resync-required"); } }, parent.signal);
    const switching = reader.switchWith("original-selection", () => selected.promise);
    const reading = reader.receivePacket({ timeoutMs: 500 });
    await new Promise(setImmediate); assert.equal(newReads, 0);
    selected.resolve({ streamId: "new", media: { async receivePacket() { newReads++; return "new-packet"; } } });
    await switching; assert.deepEqual(await reading, { value: "new-packet", streamId: "new" });
    assert.equal(newReads, 1);
});

test("unknown IO remains an error even when the same pending selection later completes", async () => {
    const parent = new AbortController(), selected = deferred(), unknown = failure("resync-required", "uncertain");
    const reader = new SwitchingMediaReader("old", { async receivePacket() { throw unknown; } }, parent.signal);
    const switching = reader.switchWith("original-selection", () => selected.promise);
    await assert.rejects(reader.receivePacket({ timeoutMs: 500 }), error => error === unknown);
    selected.resolve({ streamId: "new", media: { async receivePacket() { return "new"; } } }); await switching;
    assert.equal(reader.observation.original_read_waits_cancelled, 0);
});

test("selection refusal remains the same error for the original wait and subsequent reads", async () => {
    const parent = new AbortController(), selected = deferred(), denied = failure("authority", "uncertain");
    const reader = new SwitchingMediaReader("old", { receivePacket: stoppedRead }, parent.signal);
    const reading = reader.receivePacket({ timeoutMs: 500 }); reading.catch(() => {});
    const switching = reader.switchWith("original-selection", async () => { await selected.promise; throw denied; });
    selected.resolve();
    await assert.rejects(switching, error => error === denied);
    await assert.rejects(reading, error => error === denied);
    await assert.rejects(reader.receivePacket({ timeoutMs: 500 }), error => error === denied);
    assert.equal(reader.observation.completed_switches, 0);
});

test("a pending receipt cannot extend the original finite read deadline", async () => {
    const parent = new AbortController(), selected = deferred();
    const reader = new SwitchingMediaReader("old", { async receivePacket() { throw failure("resync-required"); } }, parent.signal);
    const switching = reader.switchWith("original-selection", () => selected.promise);
    const began = performance.now();
    await assert.rejects(reader.receivePacket({ timeoutMs: 40 }), error => error.kind === "deadline" && error.certainty === "read-only");
    assert.ok(performance.now() - began < 300);
    selected.resolve({ streamId: "new", media: { async receivePacket() { return "new"; } } }); await switching;
});

test("parent cancellation joins a pending old-binding read without awaiting or repeating the selection", async () => {
    const parent = new AbortController(), selected = deferred(); let submissions = 0;
    const original = failure("resync-required");
    const reader = new SwitchingMediaReader("old", { async receivePacket() { throw original; } }, parent.signal);
    const switching = reader.switchWith("original-selection", () => { submissions++; return selected.promise; }); switching.catch(() => {});
    const reading = reader.receivePacket({ timeoutMs: 1000 });
    await new Promise(setImmediate); parent.abort();
    await assert.rejects(reading, error => error.kind === "cancelled" && error.certainty === "read-only");
    selected.resolve({ streamId: "new", media: { async receivePacket() { assert.fail("late selected owner cannot start"); } } });
    await assert.rejects(switching);
    assert.equal(submissions, 1); assert.equal(reader.observation.completed_switches, 0);
});
