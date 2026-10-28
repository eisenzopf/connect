import { randomUUID } from "node:crypto";
import { inspect } from "node:util";
import { NodeSdkError, object } from "./sdk_helper.js";
import { readerOptions, readerWait } from "./node_readers.js";
import { identifier } from "./uctp_wire.js";
function cursor(value) {
    return (typeof value === "string" &&
        value.length > 0 &&
        value.length <= 2048 &&
        /^[\x21-\x7e]+$/.test(value));
}
export function sdkCursor(value) {
    if (!cursor(value))
        throw new NodeSdkError("validation", "read-only");
    return value;
}
function frozen(value) {
    if (value !== null && typeof value === "object") {
        for (const child of Object.values(value))
            frozen(child);
        Object.freeze(value);
    }
    return value;
}
const text = (v, max) => typeof v === "string" && v.length > 0 && Buffer.byteLength(v) <= max;
const integer = (v) => typeof v === "number" && Number.isSafeInteger(v) && v >= 0;
function event(value, cid) {
    const bad = () => {
        throw new NodeSdkError("protocol", "read-only");
    };
    if (!object(value, [
        "eventId",
        "cursor",
        "conversationId",
        "sessionId",
        "connectionId",
        "operationId",
        "participantId",
        "audience",
        "kind",
        "revision",
        "source",
        "observedAt",
        "data",
    ]) ||
        !identifier(value.eventId) ||
        !cursor(value.cursor) ||
        value.conversationId !== cid ||
        !text(value.kind, 128) ||
        !integer(value.revision) ||
        !Number.isSafeInteger(value.observedAt) ||
        !Array.isArray(value.audience) ||
        value.audience.length > 256 ||
        !object(value.source, ["kind", "correlationId"]) ||
        !text(value.source.kind, 128) ||
        !text(value.source.correlationId, 2048) ||
        value.data === undefined)
        return bad();
    for (const name of [
        "sessionId",
        "connectionId",
        "operationId",
        "participantId",
    ])
        if (value[name] !== null &&
            value[name] !== undefined &&
            !identifier(value[name]))
            return bad();
    const seen = new Set();
    for (const member of value.audience) {
        if (!object(member, ["participantId", "membershipEpoch"]) ||
            !identifier(member.participantId) ||
            seen.has(member.participantId) ||
            !integer(member.membershipEpoch) ||
            Number(member.membershipEpoch) < 1)
            return bad();
        seen.add(member.participantId);
    }
    const body = value.data !== null &&
        typeof value.data === "object" &&
        !Array.isArray(value.data)
        ? value.data.data
        : undefined;
    if (body !== null && typeof body === "object" && !Array.isArray(body)) {
        for (const nested of [
            body.conversation_id,
            body.session !== null &&
                typeof body.session === "object" &&
                !Array.isArray(body.session)
                ? body.session.conversation_id
                : undefined,
        ])
            if (nested !== undefined && nested !== cid)
                return bad();
        if (body.session !== null &&
            typeof body.session === "object" &&
            !Array.isArray(body.session) &&
            body.session.id !== undefined &&
            value.sessionId != null &&
            body.session.id !== value.sessionId)
            return bad();
        for (const [field, key] of [
            ["session_id", "sessionId"],
            ["connection_id", "connectionId"],
            ["operation_id", "operationId"],
        ])
            if (body[field] !== undefined &&
                value[key] !== null &&
                value[key] !== undefined &&
                body[field] !== value[key])
                return bad();
        const expected = body.resource_kind === "conversation"
            ? cid
            : body.resource_kind === "session"
                ? value.sessionId
                : body.resource_kind === "connection"
                    ? value.connectionId
                    : body.resource_kind === "operation"
                        ? value.operationId
                        : undefined;
        if (expected !== null &&
            expected !== undefined &&
            body.resource_id !== undefined &&
            body.resource_id !== expected)
            return bad();
    }
    const result = {
        eventId: value.eventId,
        cursor: value.cursor,
        conversationId: cid,
        sessionId: value.sessionId == null
            ? undefined
            : value.sessionId,
        connectionId: value.connectionId == null
            ? undefined
            : value.connectionId,
        operationId: value.operationId == null
            ? undefined
            : value.operationId,
        participantId: value.participantId == null
            ? undefined
            : value.participantId,
        audience: Object.freeze(value.audience.map((v) => Object.freeze({
            participantId: v
                .participantId,
            membershipEpoch: Number(v.membershipEpoch),
        }))),
        kind: value.kind,
        revision: Number(value.revision),
        source: Object.freeze({
            kind: value.source.kind,
            correlationId: value.source.correlationId,
        }),
        observedAt: Number(value.observedAt),
        data: frozen(value.data),
    };
    Object.defineProperty(result, inspect.custom, {
        value: () => ({
            eventId: result.eventId,
            conversationId: cid,
            kind: result.kind,
            revision: result.revision,
            data: "[redacted]",
        }),
    });
    return Object.freeze(result);
}
export function createEvents(helper, cid, input, options) {
    readerOptions(options);
    if (!identifier(cid) ||
        !object(input, ["after"]) ||
        (input.after !== undefined && !cursor(input.after)))
        throw new NodeSdkError("validation", "read-only");
    const captured = Object.freeze({
        timeoutMs: options.timeoutMs,
        signal: options.signal,
    }), after = input.after ?? null, reader = "events-" + randomUUID(), stopped = new AbortController();
    let acknowledged = null, flight, closing, baseline;
    let closed = false, detached = false, started = false, busy = false;
    const seen = new Set();
    const bad = () => {
        throw new NodeSdkError("protocol", "read-only");
    };
    const decode = (value) => {
        if (!object(value, [
            "reader_id",
            "conversation_id",
            "ready",
            "cursor",
            "event",
            "done",
        ]) ||
            value.reader_id !== reader ||
            value.conversation_id !== cid ||
            typeof value.ready !== "boolean" ||
            typeof value.done !== "boolean" ||
            (value.ready ? !cursor(value.cursor) : value.cursor !== null) ||
            (value.done && value.event !== null) ||
            (value.event !== null && !value.ready))
            return bad();
        if (value.ready) {
            if ((after !== null && value.cursor !== after) ||
                (baseline !== undefined && value.cursor !== baseline))
                return bad();
            baseline = value.cursor;
        }
        else if (baseline !== undefined)
            return bad();
        return {
            ready: value.ready,
            cursor: value.ready ? value.cursor : undefined,
            event: value.event === null ? undefined : event(value.event, cid),
            done: value.done,
        };
    };
    const close = async () => {
        if (detached)
            return;
        closed = true;
        stopped.abort();
        if (!started) {
            detached = true;
            return;
        }
        if (!closing) {
            const original = helper.request({
                type: "events_close",
                reader_id: reader,
                conversation_id: cid,
            }, { timeoutMs: 2000 }, (value) => {
                if (!object(value, [
                    "reader_id",
                    "conversation_id",
                    "closed",
                ]) ||
                    value.reader_id !== reader ||
                    value.conversation_id !== cid ||
                    value.closed !== true)
                    return bad();
                detached = true;
                flight = undefined;
            });
            closing = original;
            void original.catch(() => {
                if (closing === original)
                    closing = undefined;
            });
        }
        await readerWait(closing, performance.now() + 5000, undefined, new AbortController().signal);
    };
    const read = async (ready) => {
        if (closed)
            return undefined;
        if (captured.signal?.aborted)
            throw new NodeSdkError("cancelled", "read-only");
        if (busy)
            throw new NodeSdkError("conflict", "read-only");
        busy = true;
        const expires = performance.now() + captured.timeoutMs;
        try {
            const abandoned = flight;
            if (abandoned) {
                await readerWait(abandoned, expires, captured.signal, stopped.signal);
                if (flight === abandoned)
                    flight = undefined;
            }
            while (true) {
                if (performance.now() >= expires)
                    throw new NodeSdkError("deadline", "read-only");
                if (!flight) {
                    started = true;
                    const original = helper.request({
                        type: "events_poll",
                        reader_id: reader,
                        conversation_id: cid,
                        after,
                        acknowledged,
                        ready,
                    }, { timeoutMs: 2000 }, decode);
                    flight = original;
                    void original.catch(() => {
                        if (flight === original)
                            flight = undefined;
                    });
                }
                const original = flight, value = await readerWait(original, expires, captured.signal, stopped.signal);
                if (captured.signal?.aborted || closed)
                    throw new NodeSdkError("cancelled", "read-only");
                if (flight === original)
                    flight = undefined;
                if (value.done) {
                    await close();
                    return undefined;
                }
                if (ready && value.ready)
                    return value;
                if (!ready && value.event) {
                    if (seen.has(value.event.eventId)) {
                        helper.fence("protocol");
                        return bad();
                    }
                    if (seen.size === 256)
                        seen.delete(seen.values().next().value);
                    seen.add(value.event.eventId);
                    acknowledged = {
                        event_id: value.event.eventId,
                        cursor: value.event.cursor,
                    };
                    return value;
                }
                await readerWait(new Promise((resolve) => setTimeout(resolve, 5)), expires, captured.signal, stopped.signal);
            }
        }
        finally {
            busy = false;
        }
    };
    const stream = {
        async next() {
            const value = await read(false);
            return value?.event
                ? { done: false, value: value.event }
                : { done: true, value: undefined };
        },
        async ready() {
            const value = await read(true);
            if (!value?.cursor)
                throw new NodeSdkError("cancelled", "read-only");
            return value.cursor;
        },
        close,
        async return() {
            await close();
            return { done: true, value: undefined };
        },
        [Symbol.asyncIterator]() {
            return stream;
        },
    };
    return Object.freeze(stream);
}
