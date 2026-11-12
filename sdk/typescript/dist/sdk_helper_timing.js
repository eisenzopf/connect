/** Opt-in bounded control timing. No commands, keys or logical IDs are logged. */
import { createHash } from "node:crypto";
const actions = new Set([
    "media_attach",
    "media_detach",
    "events_poll",
    "events_close",
    "incoming_poll",
    "incoming_close",
    "raw_request",
    "session_handoff",
    "connection_move",
    "capabilities_refresh",
    "consultation_return",
    "conversation_create",
    "conversation_send",
    "message_receipts",
    "conversation_refresh",
    "participant_add",
    "participant_update",
    "participant_remove",
    "participant_refresh",
    "session",
    "connection",
    "operation",
]);
const nestedActions = new Set([
    "refresh",
    "status",
    "wait",
    "wait_until_ready",
    "invite",
    "accept",
    "reject",
    "cancel",
    "end",
    "update",
    "offer",
    "consult",
]);
const reasons = new Set([
    "deadline",
    "cancelled",
    "protocol",
    "transport-uncertain",
    "backpressure",
    "configuration",
    "validation",
    "trust",
    "authentication",
    "unsupported",
    "conflict",
    "application",
    "receipt-unavailable",
    "resync-required",
    "cleanup-incomplete",
]);
export class HelperTimingLog {
    enabled = process.env.CONNECT_SDK_HELPER_TIMING === "1";
    owner;
    sequence = 0;
    events = 0;
    constructor(configPath) {
        // Hash the identifier only; the Node process does not read credentials.
        this.owner = this.enabled
            ? createHash("sha256").update(configPath).digest("hex")
            : "";
    }
    capture(createdAt) {
        return this.enabled
            ? { sequence: ++this.sequence, createdAt }
            : undefined;
    }
    record(phase, job, pid, queued, reason) {
        if (!job.timing)
            return;
        const now = performance.now();
        if (phase === "dispatched")
            job.timing.dispatchedAt = now;
        if (this.events > 2048)
            return;
        const event = this.events++;
        const nested = job.action.action;
        const nestedType = nested && typeof nested === "object" && !Array.isArray(nested)
            ? nested.type
            : undefined;
        const milliseconds = (value) => Number.isFinite(value)
            ? Math.max(0, Math.round(value * 1000) / 1000)
            : null;
        const row = event === 2048
            ? {
                profile: "connect-sdk-helper-timing/1",
                owner: this.owner,
                truncated: true,
            }
            : {
                profile: "connect-sdk-helper-timing/1",
                owner: this.owner,
                helper_pid: pid ?? null,
                request: job.timing.sequence,
                action: typeof job.action.type === "string" &&
                    actions.has(job.action.type)
                    ? job.action.type
                    : "other",
                nested_action: typeof nestedType === "string" &&
                    nestedActions.has(nestedType)
                    ? nestedType
                    : null,
                phase,
                sent: job.sent,
                queued,
                reason: reason === undefined
                    ? null
                    : reasons.has(reason)
                        ? reason
                        : "other",
                elapsed_ms: milliseconds(now - job.timing.createdAt),
                queue_ms: job.timing.dispatchedAt === undefined
                    ? null
                    : milliseconds(job.timing.dispatchedAt -
                        job.timing.createdAt),
                active_ms: job.timing.dispatchedAt === undefined
                    ? null
                    : milliseconds(now - job.timing.dispatchedAt),
                original_timeout_ms: milliseconds(job.expires - job.timing.createdAt),
            };
        try {
            process.stderr.write(JSON.stringify(row) + "\n");
        }
        catch {
            /* Diagnostic output cannot change a request's outcome. */
        }
    }
}
