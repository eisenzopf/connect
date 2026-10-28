import { readerOptions, readerWait } from "./node_readers.js";
import { randomUUID } from "node:crypto";
import { NodeSdkError, object } from "./sdk_helper.js";
import { identifier } from "./uctp_wire.js";
import { canonicalJson } from "./uctp_json.js";
function freeze(value) {
    if (value !== null && typeof value === "object") {
        for (const child of Object.values(value))
            freeze(child);
        Object.freeze(value);
    }
    return value;
}
export function createIncoming(helper, session, options) {
    readerOptions(options);
    const captured = Object.freeze({
        timeoutMs: options.timeoutMs,
        signal: options.signal,
    });
    const reader = "reader-" + randomUUID(), stopped = new AbortController();
    let acknowledged = null, flight;
    let busy = false, closed = false, started = false, detached = false;
    let closing;
    const bad = () => {
        throw new NodeSdkError("protocol", "read-only");
    };
    const decode = (value) => {
        if (!object(value, ["reader_id", "delivery_id", "invitation"]) ||
            value.reader_id !== reader)
            return bad();
        if (value.delivery_id === null && value.invitation === null)
            return null;
        const hint = value.invitation;
        if (!identifier(value.delivery_id) ||
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
            Buffer.byteLength(canonicalJson(hint)) > 63 * 1024)
            return bad();
        return {
            id: value.delivery_id,
            hint: Object.freeze({
                session: session(hint.conversation_id, hint.session_id),
                from: hint.from_participant,
                to: hint.to_participant,
                operationId: hint.operation_id,
                source: Object.freeze({
                    kind: "session.invite",
                    correlationId: hint.operation_id,
                }),
                medium: hint.medium,
                intent: hint.intent,
                capabilities: freeze(hint.capabilities),
            }),
        };
    };
    const owner = Object.freeze({
        async next(options) {
            readerOptions(options);
            const signal = options.signal;
            if (closed)
                return undefined;
            if (signal?.aborted)
                throw new NodeSdkError("cancelled", "read-only");
            if (busy)
                throw new NodeSdkError("conflict", "read-only");
            busy = true;
            const expires = performance.now() + options.timeoutMs;
            try {
                // A reply abandoned by an earlier wait remains unacknowledged
                // in Rust. Re-poll that retained delivery under current health;
                // never return a Node-cached hint after revocation/overflow.
                const abandoned = flight;
                if (abandoned) {
                    await readerWait(abandoned, expires, signal, stopped.signal);
                    if (flight === abandoned)
                        flight = undefined;
                }
                while (true) {
                    if (performance.now() >= expires)
                        throw new NodeSdkError("deadline", "read-only");
                    if (!flight) {
                        started = true;
                        // Caller cancellation detaches this wait only. The bounded
                        // original poll and retained reply remain owned for next().
                        flight = helper.request({
                            type: "incoming_poll",
                            reader_id: reader,
                            acknowledged,
                        }, { timeoutMs: 2000 }, decode);
                        const original = flight;
                        void original.catch(() => {
                            if (flight === original)
                                flight = undefined;
                        });
                    }
                    const original = flight, value = await readerWait(original, expires, signal, stopped.signal);
                    if (signal?.aborted || closed)
                        throw new NodeSdkError("cancelled", "read-only");
                    if (flight === original)
                        flight = undefined;
                    if (value) {
                        acknowledged = value.id;
                        return value.hint;
                    }
                    // Prevent a fast synthetic/idle helper from a busy loop.
                    await readerWait(new Promise((resolve) => setTimeout(resolve, 5)), expires, signal, stopped.signal);
                }
            }
            finally {
                busy = false;
            }
        },
        async close(options) {
            readerOptions(options);
            const signal = options.signal, expires = performance.now() + options.timeoutMs;
            if (signal?.aborted)
                throw new NodeSdkError("cancelled", "read-only");
            if (detached)
                return;
            closed = true;
            stopped.abort();
            if (!started) {
                detached = true;
                return;
            }
            if (!closing) {
                const original = helper.request({ type: "incoming_close", reader_id: reader, acknowledged }, { timeoutMs: 2000 }, (value) => {
                    if (!object(value, ["reader_id", "closed"]) ||
                        value.reader_id !== reader ||
                        value.closed !== true)
                        return bad();
                    detached = true;
                });
                closing = original;
                void original.catch(() => {
                    if (closing === original)
                        closing = undefined;
                });
            }
            await readerWait(closing, expires, signal, new AbortController().signal);
        },
    });
    const stream = {
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
