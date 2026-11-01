// Preserve useful terminal operation evidence without copying identifiers,
// response bodies, provider messages or the original operation reference.
const states = new Set(["accepted", "preparing", "ready", "committing", "completed", "failed", "cancelled", "unknown", "interrupted"]);
const codes = new Set(["authority-busy", "forbidden", "unavailable", "storage", "storage-error", "storage_error", "conflict",
    "operation_conflict", "deadline", "timeout", "not-ready", "not_ready", "unsupported", "unsupported_feature",
    "invalid-request", "invalid_request", "payload-too-large", "request_too_large", "cancelled"]);
export function operationFailure(error) {
    const result = {};
    if (Number.isSafeInteger(error?.problem?.code) && error.problem.code >= 100 && error.problem.code <= 599)
        result.protocol_problem_code = error.problem.code;
    const snapshot = error?.observation;
    if (snapshot && states.has(snapshot.state) && Number.isSafeInteger(snapshot.revision) && snapshot.revision >= 0) {
        result.original_recorded_observation = { state: snapshot.state, revision: snapshot.revision };
        if (snapshot.problem && typeof snapshot.problem.code === "string") {
            if (codes.has(snapshot.problem.code)) result.original_recorded_observation.problem_code = snapshot.problem.code;
            else result.original_recorded_observation.problem_code_unrecognized = true;
        }
    }
    return result;
}
