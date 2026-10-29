import { fields } from "./browser_sdk_sessions.js";
import { jsonObject } from "./uctp_json.js";
import { identifier } from "./uctp_wire.js";
const states = new Set([
    "accepted",
    "preparing",
    "ready",
    "committing",
    "completed",
    "cancelled",
    "failed",
    "interrupted",
    "unknown",
]);
export function operationDecisionResult(value, key, bad) {
    if (!jsonObject(value) ||
        !identifier(value.target_operation_id) ||
        value.target_operation_id === key.operationId ||
        !states.has(String(value.state_at_decision)) ||
        typeof value.disposition !== "string" ||
        !value.disposition ||
        new TextEncoder().encode(value.disposition).length > 256 ||
        /[\x00-\x1f\x7f-\x9f]/u.test(value.disposition))
        return bad();
    fields(value, [
        "target_operation_id",
        "state_at_decision",
        "disposition",
        "evidence_id",
    ], bad);
    if (key.command === "operation.cancel"
        ? value.evidence_id !== undefined && value.evidence_id !== null
        : key.command !== "operation.reconcile" ||
            !identifier(value.evidence_id))
        return bad();
    return Object.freeze({
        decision: key,
        target: value.target_operation_id,
        observedState: value.state_at_decision,
        disposition: value.disposition,
        ...(value.evidence_id === undefined || value.evidence_id === null
            ? {}
            : { evidenceId: value.evidence_id }),
    });
}
