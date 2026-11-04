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
    const now = () => origin + 66500;
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
    assert.deepEqual(timeouts, [500, 500, 500, 500]); assert.deepEqual(waits, [500]);
    assert.equal(result.value, "same-original-programme");
    assert.equal(result.observation.original_cutoff_monotonic_ms, origin + 67000);
    assert.equal(result.observation.observers_joined, true);
});
