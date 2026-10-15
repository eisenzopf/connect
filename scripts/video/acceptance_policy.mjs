/** Versioned quality acceptance; raw coded-AV observations remain unchanged. */
import {ordinalSync} from "./marker_calibration.mjs";

export const acceptancePolicy = "connect-media-acceptance/2";
const object = value => value !== null && typeof value === "object" && !Array.isArray(value);
const edges = value => Array.isArray(value) && value.length <= 512 &&
    value.every((at, index) => Number.isFinite(at) && (!index || at > value[index - 1]));

// Cross-language arithmetic tolerance only. The recomputed values, not this
// comparison tolerance, decide the unchanged strict quality thresholds.
const discreteFields = new Set(["paired", "video_markers", "audio_markers", "ordinal", "ordinals",
    "candidate_peak", "candidate_bound", "candidate_assignments", "missing_internal_markers",
    "unmatched_video_markers", "unmatched_audio_markers"]);
function canonicalMatches(actual, expected, field = "") {
    if (typeof expected === "number" && discreteFields.has(field))
        return Number.isInteger(actual) && actual === expected;
    if (typeof expected === "number") return typeof actual === "number" && Number.isFinite(actual) &&
        Math.abs(actual - expected) <= 1e-7;
    if (Array.isArray(expected)) return Array.isArray(actual) && actual.length === expected.length &&
        expected.every((value, index) => canonicalMatches(actual[index], value, field));
    if (object(expected)) return object(actual) && Object.entries(expected).every(([key, value]) =>
        Object.hasOwn(actual, key) && canonicalMatches(actual[key], value, key));
    return actual === expected;
}

export function audioVideoAcceptance(sync, scenario, seconds) {
    const result = (status, quality_status, reason) => ({policy: acceptancePolicy, scenario, status, quality_status, reason});
    const invalid = reason => result("failed", "invalid", reason);
    if (!["clean", "mute", "impairment"].includes(scenario)) return invalid("invalid-scenario");
    if (!Number.isFinite(seconds) || seconds < 1 || seconds > 120) return invalid("invalid-duration");
    if (!object(sync) || !edges(sync.video_edge_seconds) || !edges(sync.audio_edge_seconds) ||
        !Number.isInteger(sync.video_markers) || sync.video_markers !== sync.video_edge_seconds.length ||
        !Number.isInteger(sync.audio_markers) || sync.audio_markers !== sync.audio_edge_seconds.length)
        return invalid("invalid-observation");
    if (Object.hasOwn(sync, "audio_envelope")) {
        const envelope = sync.audio_envelope;
        if (!object(envelope) || envelope.status !== "observer-valid" || envelope.overflow !== false ||
            envelope.invalid_input !== false || !Array.isArray(envelope.invalid) || envelope.invalid.length !== 0)
            return invalid("invalid-observer");
    }
    if (Object.hasOwn(sync, "observer_failure")) return invalid("invalid-observer");
    const canonical = ordinalSync(sync.video_edge_seconds, sync.audio_edge_seconds, seconds);
    if (!canonicalMatches(sync, canonical)) return invalid("inconsistent-observation");
    if (canonical.status === "passed") return result("passed", "passed", "quality-met");
    if (scenario === "impairment") return result("passed", "failed", "impaired-quality-advisory");
    return result("failed", "failed", "strict-quality-not-met");
}
