import { randomUUID } from "node:crypto";
import { SdkHelper, NodeSdkError, object } from "./sdk_helper.js";
import { identifier } from "./uctp_wire.js";
import { canonicalJson, type Json } from "./uctp_json.js";
import type { CanonicalRequestOptions } from "./canonical_validation.js";
import type { ConversationId, OperationId, ParticipantId } from "./node_sdk.js";
import type {
    SessionId,
    SdkSession,
    SdkSessionMedium,
} from "./node_sessions.js";

/** Signed metadata hint. Refresh the Session before any separate decision. */
export interface SdkIncomingInvitation {
    readonly session: SdkSession;
    readonly from: ParticipantId;
    readonly to: ParticipantId;
    readonly operationId: OperationId;
    readonly source: Readonly<{
        kind: "session.invite";
        correlationId: OperationId;
    }>;
    readonly medium: SdkSessionMedium;
    readonly intent: string;
    readonly capabilities: Json;
}
export interface SdkIncomingInvitations extends AsyncIterableIterator<SdkIncomingInvitation> {
    next(): Promise<IteratorResult<SdkIncomingInvitation, undefined>>;
    close(): Promise<void>;
}

function valid(options: CanonicalRequestOptions): void {
    if (
        !options ||
        !Number.isSafeInteger(options.timeoutMs) ||
        options.timeoutMs < 1 ||
        options.timeoutMs > 120000 ||
        (options.signal !== undefined &&
            !(options.signal instanceof AbortSignal))
    )
        throw new NodeSdkError("validation", "read-only");
}
function wait<T>(
    promise: Promise<T>,
    expires: number,
    signal: AbortSignal | undefined,
    closed: AbortSignal,
): Promise<T> {
    return new Promise((resolve, reject) => {
        let timer: ReturnType<typeof setTimeout> | undefined;
        const cleanup = () => {
            clearTimeout(timer);
            signal?.removeEventListener("abort", cancelled);
            closed.removeEventListener("abort", cancelled);
        };
        const cancelled = () => {
            cleanup();
            reject(new NodeSdkError("cancelled", "read-only"));
        };
        const remaining = expires - performance.now();
        if (signal?.aborted || closed.aborted) {
            cancelled();
            return;
        }
        if (remaining <= 0) {
            reject(new NodeSdkError("deadline", "read-only"));
            return;
        }
        timer = setTimeout(() => {
            cleanup();
            reject(new NodeSdkError("deadline", "read-only"));
        }, remaining);
        signal?.addEventListener("abort", cancelled, { once: true });
        closed.addEventListener("abort", cancelled, { once: true });
        promise.then(
            (value) => {
                cleanup();
                resolve(value);
            },
            (error) => {
                cleanup();
                reject(error);
            },
        );
    });
}
function freeze(value: Json): Json {
    if (value !== null && typeof value === "object") {
        for (const child of Object.values(value)) freeze(child);
        Object.freeze(value);
    }
    return value;
}
type Delivery = { id: string; hint: SdkIncomingInvitation } | null;
export function createIncoming(
    helper: SdkHelper,
    session: (cid: ConversationId, sid: SessionId) => SdkSession,
    options: CanonicalRequestOptions,
): SdkIncomingInvitations {
    valid(options);
    const captured = Object.freeze({
        timeoutMs: options.timeoutMs,
        signal: options.signal,
    });

    const reader = "reader-" + randomUUID(),
        stopped = new AbortController();
    let acknowledged: string | null = null,
        flight: Promise<Delivery> | undefined;
    let busy = false,
        closed = false,
        started = false,
        detached = false;
    let closing: Promise<void> | undefined;
    const bad = (): never => {
        throw new NodeSdkError("protocol", "read-only");
    };
    const decode = (value: Json): Delivery => {
        if (
            !object(value, ["reader_id", "delivery_id", "invitation"]) ||
            value.reader_id !== reader
        )
            return bad();
        if (value.delivery_id === null && value.invitation === null)
            return null;
        const hint = value.invitation;
        if (
            !identifier(value.delivery_id) ||
            !object(hint, [
                "conversation_id",
                "session_id",
                "from_participant",
                "to_participant",
                "operation_id",
                "source",
                "medium",
                "intent",
                "capabilities",
            ]) ||
            !identifier(hint.conversation_id) ||
            !identifier(hint.session_id) ||
            !identifier(hint.from_participant) ||
            !identifier(hint.to_participant) ||
            hint.from_participant === hint.to_participant ||
            !identifier(hint.operation_id) ||
            !object(hint.source, ["kind", "correlation_id"]) ||
            hint.source.kind !== "session.invite" ||
            hint.source.correlation_id !== hint.operation_id ||
            typeof hint.medium !== "string" ||
            ![
                "voice",
                "video",
                "voice+video",
                "screen-share",
                "text-chat",
                "mixed",
            ].includes(hint.medium) ||
            typeof hint.intent !== "string" ||
            !hint.intent ||
            Buffer.byteLength(hint.intent) > 256 ||
            /[\x00-\x1f\x7f-\x9f]/u.test(hint.intent) ||
            hint.capabilities === undefined ||
            Buffer.byteLength(canonicalJson(hint)) > 63 * 1024
        )
            return bad();
        return {
            id: value.delivery_id,
            hint: Object.freeze({
                session: session(
                    hint.conversation_id as ConversationId,
                    hint.session_id as SessionId,
                ),
                from: hint.from_participant as ParticipantId,
                to: hint.to_participant as ParticipantId,
                operationId: hint.operation_id as OperationId,
                source: Object.freeze({
                    kind: "session.invite",
                    correlationId: hint.operation_id as OperationId,
                }),
                medium: hint.medium as SdkSessionMedium,
                intent: hint.intent,
                capabilities: freeze(hint.capabilities),
            }),
        };
    };
    const owner = Object.freeze({
        async next(options: CanonicalRequestOptions) {
            valid(options);
            const signal = options.signal;
            if (closed) return undefined;
            if (signal?.aborted)
                throw new NodeSdkError("cancelled", "read-only");
            if (busy) throw new NodeSdkError("conflict", "read-only");
            busy = true;
            const expires = performance.now() + options.timeoutMs;
            try {
                // A reply abandoned by an earlier wait remains unacknowledged
                // in Rust. Re-poll that retained delivery under current health;
                // never return a Node-cached hint after revocation/overflow.
                const abandoned = flight;
                if (abandoned) {
                    await wait(abandoned, expires, signal, stopped.signal);
                    if (flight === abandoned) flight = undefined;
                }
                while (true) {
                    if (performance.now() >= expires)
                        throw new NodeSdkError("deadline", "read-only");
                    if (!flight) {
                        started = true;
                        // Caller cancellation detaches this wait only. The bounded
                        // original poll and retained reply remain owned for next().
                        flight = helper.request(
                            {
                                type: "incoming_poll",
                                reader_id: reader,
                                acknowledged,
                            },
                            { timeoutMs: 2000 },
                            decode,
                        );
                        const original = flight;
                        void original.catch(() => {
                            if (flight === original) flight = undefined;
                        });
                    }
                    const original = flight,
                        value = await wait(
                            original,
                            expires,
                            signal,
                            stopped.signal,
                        );
                    if (signal?.aborted || closed)
                        throw new NodeSdkError("cancelled", "read-only");
                    if (flight === original) flight = undefined;
                    if (value) {
                        acknowledged = value.id;
                        return value.hint;
                    }
                    // Prevent a fast synthetic/idle helper from a busy loop.
                    await wait(
                        new Promise((resolve) => setTimeout(resolve, 5)),
                        expires,
                        signal,
                        stopped.signal,
                    );
                }
            } finally {
                busy = false;
            }
        },
        async close(options: CanonicalRequestOptions) {
            valid(options);
            const signal = options.signal,
                expires = performance.now() + options.timeoutMs;
            if (signal?.aborted)
                throw new NodeSdkError("cancelled", "read-only");
            if (detached) return;
            closed = true;
            stopped.abort();
            if (!started) {
                detached = true;
                return;
            }
            if (!closing) {
                const original = helper.request(
                    { type: "incoming_close", reader_id: reader, acknowledged },
                    { timeoutMs: 2000 },
                    (value) => {
                        if (
                            !object(value, ["reader_id", "closed"]) ||
                            value.reader_id !== reader ||
                            value.closed !== true
                        )
                            return bad();
                        detached = true;
                    },
                );
                closing = original;
                void original.catch(() => {
                    if (closing === original) closing = undefined;
                });
            }
            await wait(closing, expires, signal, new AbortController().signal);
        },
    });
    const stream: SdkIncomingInvitations = {
        async next() {
            const hint = await owner.next(captured);
            return hint === undefined
                ? { done: true, value: undefined }
                : { done: false, value: hint };
        },
        async close() {
            await owner.close({ timeoutMs: 5000 });
        },
        async return() {
            await owner.close({ timeoutMs: 5000 });
            return { done: true, value: undefined };
        },
        [Symbol.asyncIterator]() {
            return stream;
        },
    };
    return Object.freeze(stream);
}
