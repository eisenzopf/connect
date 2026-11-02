import test from "node:test";
import assert from "node:assert/strict";
import { endRetainedConnections } from "../lib/transport_current_end.mjs";
function scenario(mode = "normal") {
    let clock = 0;
    const controller = new AbortController(), saved = [], stops = [], sends = [], waits = [], reads = [];
    const uncertain = Object.assign(Error("original end uncertain"), { certainty: "uncertain" });
    const routes = Array.from({ length: 6 }, (_, index) => ({ generation: 1, sessionIndex: index < 4 ? 0 : 1,
        async stopAndJoin() { stops.push(index); if (mode === "stop-at-cutoff") clock = 67000; },
        connection: { id: "connection-" + index,
            async end(input, options) {
                assert.equal(clock >= 62000, true); assert.equal(stops.includes(index), true);
                assert.deepEqual(input, { reasonCode: 200, reason: "Synthetic workload complete" });
                assert.equal(options.operationId, "end-" + index); sends.push(index);
                assert.equal(saved.filter(v => v.kind === "intent").length, 4);
                if (mode === "unknown" && index === 1) throw uncertain;
                return { referenceJson: () => ({ operation_id: options.operationId, command: "connection.end",
                    scope: { conversation_id: "room", session_id: "session-" + (index < 4 ? 0 : 1), connection_id: "connection-" + index } }),
                    async wait(options) { waits.push(index); assert.ok(options.timeoutMs <= 5000);
                        if (mode === "late-result") clock = 67000;
                        return { snapshot: { state: "completed" }, value: { id: mode === "foreign-result" && index === 1 ? "foreign" : "connection-" + index } };
                    } };
            },
            async refresh(options) {
                reads.push(index); assert.ok(options.timeoutMs <= 5000);
                if (mode === "read-503" && reads.filter(v => v === index).length === 1)
                    throw { problem: { code: 503, reason: "storage-unavailable" } };
                if (mode === "auth-refusal" && index === 1) throw { problem: { code: 403, reason: "forbidden" } };
                return { id: "connection-" + index, state: "retired", generation: 2 };
            } } }));
    const args = { routes, ends: [1, 2, 3, 5].map(index => ({ index, operationId: "end-" + index })),
        conversationId: "room", sessions: [{ id: "session-0" }, { id: "session-1" }], startedAt: 0,
        signal: controller.signal, now: () => clock,
        async wait(ms) { if (mode === "cancel-before-end") { controller.abort(uncertain); throw uncertain; } clock += ms; },
        async persist(index, kind, value) { saved.push({ index, kind, value }); } };
    return { args, saved, stops, sends, waits, reads, uncertain, controller };
}
test("four retained current owners end after original media; moved0/4 and shared SDK are untouched", async () => {
    const s = scenario(), result = await endRetainedConnections(s.args);
    assert.equal(result.retained_connections_retired, 4); assert.equal(result.shared_sdk_closed, false);
    assert.deepEqual(s.sends, [1, 2, 3, 5]); assert.deepEqual(s.stops, [1, 2, 3, 5]);
    assert.deepEqual(s.waits, [1, 2, 3, 5]); assert.equal(s.saved.filter(v => v.kind === "receipt").length, 4);
    assert.ok(result.receipts.every(v => v.mutation_submissions === 1 && v.original_cutoff_monotonic_ms === 67000));
});
test("uncertain original is never submitted twice and all sibling originals settle", async () => {
    const s = scenario("unknown"); await assert.rejects(endRetainedConnections(s.args), error => error === s.uncertain);
    assert.deepEqual(s.sends, [1, 2, 3, 5]); assert.deepEqual(s.waits, [2, 3, 5]);
    assert.equal(s.saved.filter(v => v.kind === "receipt").length, 3);
});
test("packet join consuming original cutoff prevents every mutation", async () => {
    const s = scenario("stop-at-cutoff"); await assert.rejects(endRetainedConnections(s.args)); assert.equal(s.sends.length, 0); assert.deepEqual(s.stops, [1, 2, 3, 5]);
});
test("late original completion cannot pass or renew a deadline", async () => {
    const s = scenario("late-result"); await assert.rejects(endRetainedConnections(s.args));
    assert.ok(s.sends.every((v, index, all) => all.indexOf(v) === index)); assert.equal(s.saved.filter(v => v.kind === "receipt").length, 0);
});
test("foreign current connection result fails while sibling outcomes remain joined", async () => {
    const s = scenario("foreign-result"); await assert.rejects(endRetainedConnections(s.args)); assert.equal(s.sends.length, 4);
    assert.equal(s.saved.filter(v => v.kind === "receipt").length, 3);
});
test("503 retries only retirement metadata under the unchanged cutoff", async () => {
    const s = scenario("read-503"), result = await endRetainedConnections(s.args);
    assert.equal(result.retained_connections_retired, 4); assert.equal(s.sends.length, 4); assert.equal(s.reads.length, 8);
    assert.ok(result.receipts.every(v => v.original_cutoff_monotonic_ms === 67000));
});
test("current authorization refusal has no read retry or new end", async () => {
    const s = scenario("auth-refusal"); await assert.rejects(endRetainedConnections(s.args));
    assert.equal(s.reads.filter(v => v === 1).length, 1); assert.equal(s.sends.length, 4);
});
test("cancellation before original programme end performs no mutation", async () => {
    const s = scenario("cancel-before-end"); await assert.rejects(endRetainedConnections(s.args), error => error === s.uncertain); assert.equal(s.sends.length, 0);
});
test("wrong route set and duplicated original end keys refuse before custody or IO", async () => {
    for (const change of [s => { s.args.ends[0].index = 0; }, s => { s.args.ends[0].operationId = s.args.ends[1].operationId; }]) {
        const s = scenario(); change(s); await assert.rejects(endRetainedConnections(s.args));
        assert.equal(s.saved.length, 0); assert.equal(s.sends.length, 0);
    }
});
