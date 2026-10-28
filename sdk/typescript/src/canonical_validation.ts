import { jsonObject, type JsonObject } from "./uctp_json.js";
import { identifier, type Envelope, type Scope } from "./uctp_wire.js";
import {
    CanonicalError,
    type CanonicalErrorKind,
    type CanonicalProblem,
    type OperationKey,
} from "./canonical_error.js";
export interface CanonicalRequestOptions {
    readonly timeoutMs: number;
    readonly signal?: AbortSignal;
}
export type CanonicalCommandScope = Omit<Scope, "in_reply_to">;
export const maximumRequest = 65_536,
    maximumResponse = 2 * 1024 * 1024;
export const reads = new Set([
    "conversation.get",
    "conversation.list",
    "conversation.subscribe",
    "message.history",
    "operation.get",
]);
export const commands = new Set([
    ...reads,
    "conversation.create",
    "conversation.close",
    "conversation.participant.add",
    "conversation.participant.update",
    "conversation.participant.remove",
    "session.invite",
    "session.accept",
    "session.reject",
    "session.cancel",
    "session.end",
    "session.update",
    "session.consult",
    "session.consult.return",
    "session.handoff",
    "connection.replace",
    "connection.update",
    "connection.end",
    "stream.subscribe",
    "stream.unsubscribe",
    "message.send",
    "message.read",
    "capability.advertise",
    "dtmf.send",
    "operation.cancel",
    "operation.reconcile",
]);
export const topFields = new Set([
    "v",
    "id",
    "ts",
    "type",
    "payload",
    "cid",
    "sid",
    "connid",
    "operation_id",
    "expected_revision",
    "in_reply_to",
]);
const eventKinds = new Set([
    "conversation.event",
    "session.started",
    "session.participant.joined",
    "session.participant.left",
    "session.ended",
    "recording.vcon-ready",
    "message.read",
    "message.delivered",
]);
export function duration(options: CanonicalRequestOptions): void {
    if (
        !Number.isSafeInteger(options.timeoutMs) ||
        options.timeoutMs < 1 ||
        options.timeoutMs > 120_000
    )
        throw new CanonicalError("validation", "not-submitted");
}
export function problem(value: unknown): CanonicalProblem {
    if (!jsonObject(value as JsonObject)) throw new Error();
    const p = value as JsonObject;
    if (
        !Number.isInteger(p.code) ||
        Number(p.code) < 400 ||
        Number(p.code) > 599 ||
        typeof p.category !== "string" ||
        !/^[a-z][a-z-]{0,63}$/.test(p.category) ||
        typeof p.reason !== "string" ||
        !/^[a-z][a-z_-]{0,127}$/.test(p.reason)
    )
        throw new Error();
    return { code: Number(p.code), category: p.category, reason: p.reason };
}
export function refused(
    p: CanonicalProblem,
    key?: OperationKey,
): CanonicalError {
    const kind: CanonicalErrorKind =
        p.code === 401 || p.code === 403
            ? "authentication"
            : p.reason === "resync_required"
              ? "resync-required"
              : p.code === 409
                ? "conflict"
                : p.code === 501 || p.code === 488
                  ? "unsupported"
                  : p.code === 400 || p.code === 413
                    ? "validation"
                    : "application";
    return new CanonicalError(kind, key ? "refused" : "read-only", key, p);
}
export function checkScope(
    frame: Envelope,
    scope: CanonicalCommandScope,
    assigned: boolean,
): void {
    if (
        !scope.sid &&
        !scope.connid &&
        !scope.operation_id &&
        eventKinds.has(frame.type)
    ) {
        if (frame.cid !== scope.cid) throw new Error();
        if (frame.type === "conversation.event") {
            const data = frame.payload.data;
            if (
                !jsonObject(data) ||
                frame.payload.event_id !== data.event_id ||
                frame.payload.cursor !== data.cursor ||
                frame.cid !== data.conversation_id ||
                frame.sid !== (data.session_id ?? undefined) ||
                frame.connid !== (data.connection_id ?? undefined) ||
                frame.operation_id !== (data.operation_id ?? undefined)
            )
                throw new Error();
        } else if (
            frame.type.startsWith("session.") ||
            frame.type === "recording.vcon-ready"
        ) {
            if (
                !identifier(frame.sid) ||
                frame.connid !== undefined ||
                frame.operation_id !== undefined
            )
                throw new Error();
            if (frame.type === "recording.vcon-ready") {
                const handle = frame.payload.vcon_handle;
                if (
                    !jsonObject(handle) ||
                    typeof handle.uuid !== "string" ||
                    !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(
                        handle.uuid,
                    ) ||
                    !Number.isSafeInteger(handle.revision) ||
                    Number(handle.revision) < 1
                )
                    throw new Error();
            }
        } else if (
            frame.sid !== undefined ||
            frame.connid !== undefined ||
            !jsonObject(frame.payload.evidence) ||
            !identifier(frame.payload.evidence.event_id)
        )
            throw new Error();
        return;
    }
    for (const field of ["sid", "connid", "operation_id"] as const)
        if (frame[field] !== scope[field]) throw new Error();
    if (frame.cid !== scope.cid && !(assigned && scope.cid === undefined))
        throw new Error();
}
