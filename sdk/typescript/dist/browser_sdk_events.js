import { BrowserDeadline } from "./browser_deadline.js";
import { fields } from "./browser_sdk_sessions.js";
import { canonicalJson, MAX_ENVELOPE_BYTES, jsonObject, parseCanonicalJson, } from "./uctp_json.js";
import { identifier } from "./uctp_wire.js";
export function isBrowserCursor(v) {
    return (typeof v === "string" &&
        v.length > 0 &&
        v.length <= 2048 &&
        /^[A-Za-z0-9._~-]+$/.test(v));
}
const natural = (v) => typeof v === "number" && Number.isSafeInteger(v) && v >= 0;
const text = (v, n) => typeof v === "string" &&
    !!v &&
    new TextEncoder().encode(v).length <= n &&
    !/[\x00-\x1f\x7f-\x9f]/u.test(v);
function freeze(v) {
    if (v && typeof v === "object") {
        for (const x of Object.values(v))
            freeze(x);
        Object.freeze(v);
    }
    return v;
}
export function journalEvent(frame, cid, bad) {
    const p = frame.payload, v = p.data;
    if (frame.type !== "conversation.event" ||
        frame.cid !== cid ||
        frame.expected_revision !== undefined ||
        !jsonObject(v) ||
        v.conversation_id !== cid ||
        !identifier(v.event_id) ||
        p.event_id !== v.event_id ||
        !isBrowserCursor(v.cursor) ||
        p.cursor !== v.cursor ||
        !text(v.kind, 128) ||
        p.kind !== v.kind ||
        !natural(v.revision) ||
        p.revision !== v.revision ||
        !Number.isSafeInteger(v.observed_at) ||
        !jsonObject(v.source) ||
        !text(v.source.kind, 128) ||
        !text(v.source.correlation_id, 2048) ||
        !Array.isArray(v.audience) ||
        v.audience.length > 256 ||
        !jsonObject(v.data) ||
        !text(v.data.kind, 128) ||
        !Object.hasOwn(v.data, "data"))
        return bad();
    fields(v, [
        "event_id",
        "cursor",
        "conversation_id",
        "session_id",
        "connection_id",
        "participant_id",
        "operation_id",
        "kind",
        "revision",
        "source",
        "observed_at",
        "audience",
        "data",
    ], bad);
    fields(v.source, ["kind", "correlation_id"], bad);
    for (const [name, wire] of [
        ["session_id", "sid"],
        ["connection_id", "connid"],
        ["operation_id", "operation_id"],
    ]) {
        if (!(v[name] === null || identifier(v[name])) ||
            frame[wire] !== (v[name] === null ? undefined : v[name]))
            return bad();
    }
    if (!(v.participant_id === null || identifier(v.participant_id)))
        return bad();
    const seen = new Set();
    for (const a of v.audience) {
        if (!jsonObject(a) ||
            !identifier(a.participant_id) ||
            !natural(a.membership_epoch) ||
            Number(a.membership_epoch) < 1 ||
            seen.has(a.participant_id))
            return bad();
        fields(a, ["participant_id", "membership_epoch"], bad);
        seen.add(a.participant_id);
    }
    const body = v.data.data;
    if (jsonObject(body)) {
        if (body.conversation_id !== undefined && body.conversation_id !== cid)
            return bad();
        if (jsonObject(body.session) &&
            (body.session.conversation_id !== cid ||
                (v.session_id !== null && body.session.id !== v.session_id)))
            return bad();
        for (const field of ["session_id", "connection_id", "operation_id"])
            if (body[field] !== undefined &&
                v[field] !== null &&
                body[field] !== v[field])
                return bad();
        const expected = body.resource_kind === "conversation"
            ? cid
            : body.resource_kind === "session"
                ? v.session_id
                : body.resource_kind === "connection"
                    ? v.connection_id
                    : body.resource_kind === "operation"
                        ? v.operation_id
                        : undefined;
        if (expected != null &&
            body.resource_id !== undefined &&
            body.resource_id !== expected)
            return bad();
    }
    let data;
    try {
        data = parseCanonicalJson(new TextEncoder().encode(canonicalJson(v.data, MAX_ENVELOPE_BYTES)), MAX_ENVELOPE_BYTES);
    }
    catch {
        return bad();
    }
    return freeze({
        eventId: v.event_id,
        cursor: v.cursor,
        conversationId: cid,
        ...(v.session_id === null
            ? {}
            : { sessionId: v.session_id }),
        ...(v.connection_id === null
            ? {}
            : { connectionId: v.connection_id }),
        ...(v.operation_id === null
            ? {}
            : { operationId: v.operation_id }),
        ...(v.participant_id === null
            ? {}
            : { participantId: v.participant_id }),
        audience: v.audience.map((a) => ({
            participantId: a.participant_id,
            membershipEpoch: Number(a.membership_epoch),
        })),
        kind: String(v.kind),
        revision: Number(v.revision),
        source: {
            kind: String(v.source.kind),
            correlationId: String(v.source.correlation_id),
        },
        observedAt: Number(v.observed_at),
        data,
    });
}
export function createBrowserEvents(cid, input, options, ctx) {
    const invalid = () => {
        throw ctx.error("validation");
    };
    fields(input, ["after"], invalid);
    if (!options ||
        !Number.isFinite(options.timeoutMs) ||
        options.timeoutMs < 1 ||
        options.timeoutMs > 60000 ||
        (input.after !== undefined && !isBrowserCursor(input.after)))
        invalid();
    const timeout = options.timeoutMs, caller = options.signal, stopped = new AbortController();
    let after = input.after, baseline, flight, closed = false, retired = false, busy = false, released = false;
    const release = ctx.reserve(() => {
        retired = true;
        stopped.abort();
        cleanup();
    });
    function cleanup() {
        if ((closed || retired) && !flight && !released) {
            released = true;
            release();
        }
    }
    const check = () => {
        ctx.check();
        if (retired)
            throw ctx.error("cancelled");
    };
    const signal = AbortSignal.any([
        stopped.signal,
        ...(caller ? [caller] : []),
    ]);
    const wait = async (budget, work) => budget.run(() => work, signal);
    async function read(budget, position) {
        check();
        const previous = flight;
        if (previous) {
            await budget.run(() => previous.then(() => { }, () => { }), signal);
            check();
        }
        budget.remaining();
        if (signal.aborted)
            throw ctx.error("cancelled");
        const work = ctx.read(position);
        flight = work;
        const done = () => {
            if (flight === work)
                flight = undefined;
            cleanup();
        };
        void work.then(done, done);
        const page = await wait(budget, work);
        check();
        budget.remaining();
        if (signal.aborted)
            throw ctx.error("cancelled");
        return page;
    }
    function validate(page) {
        check();
        if (page.conversationId !== cid ||
            !isBrowserCursor(page.cursor) ||
            page.frames.length > 1)
            ctx.bad();
        if (page.kind === "snapshot")
            throw ctx.error("resync-required");
        const events = page.frames.map((frame) => journalEvent(frame, cid, ctx.bad));
        return events;
    }
    async function initialize(budget) {
        if (baseline !== undefined)
            return baseline;
        const page = await read(budget, after);
        if (after === undefined) {
            if (page.kind !== "snapshot" || !isBrowserCursor(page.cursor))
                ctx.bad();
            after = page.cursor;
        }
        else
            validate(page);
        baseline = after;
        return baseline;
    }
    async function run(work, commit) {
        if (busy)
            throw ctx.error("conflict");
        busy = true;
        try {
            check();
            if (signal.aborted)
                throw ctx.error("cancelled");
            const budget = new BrowserDeadline(timeout);
            const value = await work(budget);
            check();
            budget.remaining();
            if (signal.aborted)
                throw ctx.error("cancelled");
            commit?.(value);
            return value;
        }
        catch (error) {
            throw ctx.translated(error);
        }
        finally {
            busy = false;
            cleanup();
        }
    }
    const reader = {
        [Symbol.asyncIterator]() {
            return reader;
        },
        ready() {
            return run(initialize);
        },
        async next() {
            if (closed)
                return { done: true, value: undefined };
            try {
                return await run(async (budget) => {
                    await initialize(budget);
                    for (;;) {
                        const page = await read(budget, after), events = validate(page);
                        if (events.length) {
                            const value = events[0];
                            return { done: false, value };
                        }
                        if (page.hasMore && page.cursor === after)
                            ctx.bad();
                        after = page.cursor;
                        await budget.run(() => new Promise((resolve) => setTimeout(resolve, Math.min(25, budget.remaining()))), signal);
                    }
                }, (outcome) => {
                    after = outcome.value.cursor;
                });
            }
            catch (error) {
                if (closed)
                    return { done: true, value: undefined };
                throw error;
            }
        },
        async close() {
            closed = true;
            stopped.abort();
            cleanup();
        },
        async return() {
            await reader.close();
            return { done: true, value: undefined };
        },
    };
    return reader;
}
