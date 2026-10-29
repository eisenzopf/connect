import { fields } from "./browser_sdk_sessions.js";
import { jsonObject, type Json } from "./uctp_json.js";
import { identifier } from "./uctp_wire.js";
import type { BrowserOperationReference } from "./browser_transport_client.js";
import type { BrowserOperationState } from "./browser_sdk.js";
import type { OperationId, ConversationId, SessionId } from "./sdk_identity.js";
export interface BrowserDecisionReceipt {
    readonly decision: {
        readonly operationId: OperationId;
        readonly command: "operation.cancel" | "operation.reconcile";
        readonly scope: {
            readonly conversationId?: ConversationId;
            readonly sessionId?: SessionId;
        };
    };
    readonly target: OperationId;
    readonly observedState: BrowserOperationState;
    readonly disposition: string;
    readonly evidenceId?: string;
}
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
export function operationDecisionResult(
    value: Json,
    key: BrowserOperationReference,
    bad: () => never,
): BrowserDecisionReceipt {
    if (
        !jsonObject(value) ||
        !identifier(value.target_operation_id) ||
        value.target_operation_id === key.operationId ||
        !states.has(String(value.state_at_decision)) ||
        typeof value.disposition !== "string" ||
        !value.disposition ||
        new TextEncoder().encode(value.disposition).length > 256 ||
        /[\x00-\x1f\x7f-\x9f]/u.test(value.disposition)
    )
        return bad();
    fields(
        value,
        [
            "target_operation_id",
            "state_at_decision",
            "disposition",
            "evidence_id",
        ],
        bad,
    );
    if (
        key.command === "operation.cancel"
            ? value.evidence_id !== undefined && value.evidence_id !== null
            : key.command !== "operation.reconcile" ||
              !identifier(value.evidence_id)
    )
        return bad();
    return Object.freeze({
        decision: key as BrowserDecisionReceipt["decision"],
        target: value.target_operation_id as OperationId,
        observedState: value.state_at_decision as BrowserOperationState,
        disposition: value.disposition,
        ...(value.evidence_id === undefined || value.evidence_id === null
            ? {}
            : { evidenceId: value.evidence_id as string }),
    });
}
