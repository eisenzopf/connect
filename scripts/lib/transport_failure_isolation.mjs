// Exact original errors carry private cleanup evidence through this WeakMap.
// A status string, copied receipt, or guessed current owner cannot qualify.
// Unknown completion remains fatal. Only a definitive refusal of the original
// submission before any target worker can preserve independent observations.
import assert from "node:assert/strict";
const cleanup = new WeakMap(), observerFailures = new WeakMap();
export function observationFailure(error) { return observerFailures.get(error); }
export function emitOriginalObservation(observe, args, originalError) {
    try { observe(...args); return { emitted: true, error: originalError }; }
    catch (observerError) {
        const error = originalError ?? observerError;
        if (error && (typeof error === "object" || typeof error === "function") && !observerFailures.has(error)) observerFailures.set(error, observerError);
        return { emitted: false, error, observerError };
    }
}

export function recordPrivateTargetCleanup(error, facts, cleanupError) {
    if (!(error instanceof Error)) return;
    const snapshot = Object.freeze({ ...facts });
    assert.equal(snapshot.format, "connect/original-private-target-cleanup/1");
    cleanup.set(error, Object.freeze({ facts: snapshot, cleanupError }));
}
export function privateTargetCleanup(error) { return cleanup.get(error); }
export function isolatedPrivateTargetFailure(error) {
    const value = cleanup.get(error), f = value?.facts;
    return Boolean(f && !observerFailures.has(error) && !value.cleanupError && f.transferred === false &&
        f.logical_outcome === "not-submitted" && f.submission_definitively_refused === true &&
        f.completion_observed === false && f.worker_started === false &&
        f.original_root_completed === false && f.original_source_owner_joined === false &&
        (f.worker_started === false || (f.worker_started === true && f.target_worker_joined === true)) &&
        f.target_worker_forced === false && f.target_executable_unchanged === true &&
        f.target_config_unchanged === true);
}
export function blockedMovementSlots(plan, sourceIndex, failedOrdinal, nowMs) {
    return plan.filter(step => step.sourceIndex === sourceIndex && step.ordinal > failedOrdinal)
        .map(step => Object.freeze({ source_index: sourceIndex, ordinal: step.ordinal,
            operation_id: step.operationId, scheduled_start_ms: step.scheduled_start_ms,
            status: "blocked", prerequisite: "original-predecessor-admitted-owner",
            failed_predecessor_ordinal: failedOrdinal, observed_ms: nowMs,
            mutation_submissions: 0, replacement_owner_inferred: false }));
}

const observers = new WeakMap();
export function recordReadObserverFailure(error, ownerIndex) {
    if (!(error instanceof Error) || error.certainty !== "read-only") return false;
    assert.ok(Number.isInteger(ownerIndex) && ownerIndex >= 0 && ownerIndex < 4);
    observers.set(error, Object.freeze({ ownerIndex, original_read_settled: true, mutation_submissions: 0 }));
    return true;
}
export function isolatedProgrammeFailure(error) {
    return !observerFailures.has(error) && (isolatedPrivateTargetFailure(error) || observers.has(error));
}
