import assert from "node:assert/strict";
import { test } from "node:test";
import { waitForProgrammeOffset } from "../lib/transport_programme_clock.mjs";
import { observeMovementSessions } from "../lib/transport_movement_sessions.mjs";

test("warmup and observer share the source clock even when process-relative time is unrelated", async () => {
    const origin = 1000000000, parent = new AbortController();
    for (const [elapsed, delay] of [[0, 2000], [1000, 1000], [2500, 0]]) {
        await waitForProgrammeOffset({ startedAt: origin, now: () => origin + elapsed,
            signal: parent.signal }, 2000, async (ms, signal) => {
            assert.equal(ms, delay); assert.equal(signal, parent.signal);
        });
    }
    let releaseRun; const runGate = new Promise(resolve => { releaseRun = resolve; });
    const timeouts = [], waits = [];
    const session = index => ({ id: "session-" + index, async refresh({ timeoutMs }) {
        timeouts.push(timeoutMs); return { id: "session-" + index, state: "accepted", isMain: index === 0 };
    } });
    const now = () => origin + 61500;
    const result = await observeMovementSessions({
        workload: { startedAt: origin, now, signal: parent.signal },
        primarySessions: [0, 1].map(session), controllerSessions: [0, 1].map(session),
        async wait(ms, signal) {
            waits.push(ms); releaseRun();
            await new Promise(resolve => {
                if (signal.aborted) resolve(); else signal.addEventListener("abort", resolve, { once: true });
            });
        },
        async run(current) {
            assert.equal(current.now, now); assert.equal(current.startedAt, origin);
            await runGate; return "same-original-programme";
        },
    });
    assert.deepEqual(timeouts, [5500, 5500, 5500, 5500]); assert.deepEqual(waits, [500]);
    assert.equal(result.value, "same-original-programme");
    assert.equal(result.observation.original_cutoff_monotonic_ms, origin + 67000);
    assert.equal(result.observation.observers_joined, true);
});


test("a late retirement-phase observer admits no read and fails missing original four-owner coverage", async () => {
    const origin = 1000000000, parent = new AbortController();
    let reads = 0, originalRunJoined = false, observation;
    const session = index => ({ id: "session-" + index, refresh() { reads++; throw Error("late read"); } });
    const now = () => origin + 66500;
    await assert.rejects(observeMovementSessions({ workload: { startedAt: origin, now, signal: parent.signal },
        primarySessions: [0, 1].map(session), controllerSessions: [0, 1].map(session),
        wait() { throw Error("late observer sleep"); },
        observe(value) { observation = value; },
        async run(current) {
            assert.equal(current.now, now); assert.equal(current.signal.aborted, false);
            await Promise.resolve(); originalRunJoined = true; return "retirement-only";
        } }), error => error.code === "original-movement-observation-coverage-missed");
    assert.equal(reads, 0); assert.equal(originalRunJoined, true);
    assert.equal(observation.passed, false); assert.equal(observation.current_session_reads, 0);
    assert.equal(observation.completed_cycles, 0); assert.equal(observation.observers_joined, true);
    assert.equal(observation.original_cutoff_monotonic_ms, origin + 67000);
    assert.equal(parent.signal.aborted, false);
});

test("missing observation coverage cannot replace the original movement failure", async () => {
    const parent = new AbortController(), failure = Error("original movement failure");
    let reads = 0, originalRunJoined = false, observation;
    const session = index => ({ id: "session-" + index, refresh() { reads++; throw Error("late read"); } });
    await assert.rejects(observeMovementSessions({ workload: { startedAt: 0, signal: parent.signal },
        primarySessions: [0, 1].map(session), controllerSessions: [0, 1].map(session), now: () => 66500,
        observe(value) { observation = value; },
        async run() { await Promise.resolve(); originalRunJoined = true; throw failure; } }), error => error === failure);
    assert.equal(reads, 0); assert.equal(originalRunJoined, true);
    assert.equal(observation.passed, false); assert.equal(observation.current_session_reads, 0);
    assert.equal(observation.completed_cycles, 0); assert.equal(observation.observers_joined, true);
});


test("parent cancellation at the original run return remains primary when no observer cycle started", async () => {
    const parent = new AbortController(), reason = Error("original parent cancellation");
    let reads = 0, originalRunJoined = false, observation;
    const session = index => ({ id: "session-" + index, refresh() { reads++; throw Error("late read"); } });
    await assert.rejects(observeMovementSessions({ workload: { startedAt: 0, signal: parent.signal },
        primarySessions: [0, 1].map(session), controllerSessions: [0, 1].map(session), now: () => 66500,
        observe(value) { observation = value; },
        async run() {
            await Promise.resolve(); parent.abort(reason); originalRunJoined = true; return "same-run";
        } }), error => error === reason);
    assert.equal(reads, 0); assert.equal(originalRunJoined, true);
    assert.equal(observation.passed, false); assert.equal(observation.current_session_reads, 0);
    assert.equal(observation.completed_cycles, 0); assert.equal(observation.observers_joined, true);
    assert.equal(parent.signal.reason, reason);
});
