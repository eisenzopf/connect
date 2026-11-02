import assert from "node:assert/strict";
import test from "node:test";
import { observeMovementSessions } from "../lib/transport_movement_sessions.mjs";

function scenario(mode) {
    const parent = new AbortController(), reads = [], waits = [];
    const refusal = Object.assign(Error("current authorization refused"), { certainty: "read-only" });
    let releaseRun, releaseWait, clock = 0, allReadsJoined = 0;
    const runGate = new Promise(done => { releaseRun = done; });
    const session = (index, controller) => ({ id: "session-" + index,
        async refresh(options) {
            reads.push({ index, controller, timeoutMs: options.timeoutMs });
            await Promise.resolve(); allReadsJoined++;
            if (mode === "refused" && controller && index === 1) throw refusal;
            return { id: mode === "foreign" && controller ? "foreign" : "session-" + index,
                state: mode === "ended" ? "ended" : "accepted", isMain: index === 0 };
        } });
    const args = { workload: { signal: parent.signal, startedAt: 0 },
        primarySessions: [0, 1].map(index => session(index, false)),
        controllerSessions: [0, 1].map(index => session(index, true)), now: () => clock,
        async wait(ms, signal) {
            waits.push(ms);
            if (mode === "late") { clock = 67000; return; }
            releaseRun();
            await new Promise(done => {
                releaseWait = done;
                if (signal.aborted) done();
                else signal.addEventListener("abort", done, { once: true });
            });
        },
        async run(workload) {
            if (["refused", "foreign", "ended", "late"].includes(mode)) {
                await new Promise(done => workload.signal.addEventListener("abort", done, { once: true }));
                throw workload.signal.reason;
            }
            if (mode === "run-failure") throw refusal;
            await runGate;
            if (mode === "parent-cancel") { parent.abort(refusal); throw workload.signal.reason; }
            return "original-workload-result";
        } };
    return { args, reads, waits, refusal, parent, get allReadsJoined() { return allReadsJoined; } };
}
test("both original Sessions remain observed through primary and independent controller SDKs; all reads join", async () => {
    const s = scenario("normal"), result = await observeMovementSessions(s.args);
    assert.equal(result.value, "original-workload-result");
    assert.equal(result.observation.original_control_sdks, 3);
    assert.equal(result.observation.current_session_reads, 4);
    assert.equal(result.observation.observers_joined, true);
    assert.equal(result.observation.original_cutoff_monotonic_ms, 67000);
    assert.equal(s.allReadsJoined, 4); assert.deepEqual(s.waits, [8000]);
});
test("current authorization failure cancels the live original workload and joins every read", async () => {
    const s = scenario("refused");
    await assert.rejects(observeMovementSessions(s.args), error => error === s.refusal);
    assert.equal(s.reads.length, 4); assert.equal(s.allReadsJoined, 4); assert.equal(s.waits.length, 0);
});
test("foreign or terminal Session evidence cancels movement without new helpers", async () => {
    for (const mode of ["foreign", "ended"]) {
        const s = scenario(mode); await assert.rejects(observeMovementSessions(s.args), { code: "ERR_ASSERTION" });
        assert.equal(s.allReadsJoined, 4);
    }
});
test("the observation cutoff cannot start another read or renew the original programme", async () => {
    const s = scenario("late"); await assert.rejects(observeMovementSessions(s.args), /original-movement-observation-cutoff/);
    assert.equal(s.reads.length, 4); assert.equal(s.allReadsJoined, 4);
});
test("workload failure and parent cancellation retain their original cause and join observers", async () => {
    for (const mode of ["run-failure", "parent-cancel"]) {
        const s = scenario(mode); await assert.rejects(observeMovementSessions(s.args), error => error === s.refusal);
        assert.equal(s.allReadsJoined, 4);
    }
});
test("an already cancelled parent starts no workload or read", async () => {
    const s = scenario("normal"); s.parent.abort(s.refusal);
    await assert.rejects(observeMovementSessions(s.args), error => error === s.refusal);
    assert.equal(s.reads.length, 0);
});
