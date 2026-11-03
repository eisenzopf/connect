// Current authorization/topology reads keep all three control owners observed
// while the media owners retain the original62..67second programme deadline.
import assert from "node:assert/strict";
import { setTimeout as sleep } from "node:timers/promises";

export async function observeMovementSessions({ workload, primarySessions, controllerSessions, run,
    now = () => performance.now(), wait = (ms, signal) => sleep(ms, undefined, { signal }) }) {
    assert.equal(primarySessions.length, 2); assert.equal(controllerSessions.length, 2);
    assert.ok(Number.isFinite(workload.startedAt));
    assert.deepEqual(controllerSessions.map(session => session.id), primarySessions.map(session => session.id));
    const stop = new AbortController(), stopObserving = new AbortController();
    const parentAbort = () => stop.abort(workload.signal.reason);
    workload.signal.addEventListener("abort", parentAbort, { once: true });
    if (workload.signal.aborted) parentAbort();
    let firstFailure, reads = 0, cycles = 0;
    const fail = failure => {
        if (!firstFailure) {
            firstFailure = failure;
            workload.reportFailure?.(firstFailure);
        }
        stop.abort(firstFailure);
    };
    const cutoff = workload.startedAt + 67000;
    const observing = (async () => {
        while (!stopObserving.signal.aborted && !stop.signal.aborted) {
            const timeoutMs = Math.min(8000, Math.floor(cutoff - now()));
            assert.ok(timeoutMs > 0, "original-movement-observation-cutoff");
            // The shared primary SDK and both independent controller SDKs each
            // perform current-authorized reads. No identity/helper is replaced.
            const results = await Promise.allSettled([...primarySessions, ...controllerSessions]
                .map(session => Promise.resolve().then(() => session.refresh({ timeoutMs }))));
            reads += results.length;
            const failure = results.find(result => result.status === "rejected");
            if (failure) throw failure.reason;
            for (const [index, result] of results.entries()) {
                assert.equal(result.value.id, [...primarySessions, ...controllerSessions][index].id);
                assert.ok(!["ended", "failed", "cancelled", "rejected"].includes(result.value.state));
            }
            assert.equal(results.slice(0, 2).filter(result => result.value.isMain).length, 1);
            cycles++;
            if (!stopObserving.signal.aborted && !stop.signal.aborted) {
                try { await wait(Math.min(8000, Math.max(0, cutoff - now())), stopObserving.signal); }
                catch (failure) { if (!stopObserving.signal.aborted) throw failure; }
            }
        }
    })().catch(fail);
    let value;
    try {
        if (stop.signal.aborted) throw stop.signal.reason;
        value = await run(Object.freeze({ ...workload, signal: stop.signal }));
    } catch (failure) { fail(failure); }
    finally {
        stopObserving.abort();
        await observing;
        workload.signal.removeEventListener("abort", parentAbort);
    }
    if (firstFailure) throw firstFailure;
    if (stop.signal.aborted) throw stop.signal.reason;
    return { value, observation: { original_control_sdks: 3, current_session_reads: reads,
        completed_cycles: cycles, observers_joined: true, original_cutoff_monotonic_ms: cutoff,
        replacement_sdk_created: false, mutation_submissions: 0 } };
}
