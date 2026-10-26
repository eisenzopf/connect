import { canonicalJson, jsonObject, parseCanonicalJson, } from "./uctp_json.js";
import { envelope, identifier, validateEnvelope, } from "./uctp_wire.js";
import { CanonicalError, } from "./canonical_error.js";
const maximumRequest = 65_536, maximumResponse = 2 * 1024 * 1024;
const reads = new Set([
    "conversation.get",
    "conversation.list",
    "conversation.subscribe",
    "message.history",
    "operation.get",
]);
const commands = new Set([
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
const topFields = new Set([
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
function duration(options) {
    if (!Number.isSafeInteger(options.timeoutMs) ||
        options.timeoutMs < 1 ||
        options.timeoutMs > 120_000)
        throw new CanonicalError("validation", "not-submitted");
}
function problem(value) {
    if (!jsonObject(value))
        throw new Error();
    const p = value;
    if (!Number.isInteger(p.code) ||
        Number(p.code) < 400 ||
        Number(p.code) > 599 ||
        typeof p.category !== "string" ||
        !/^[a-z][a-z-]{0,63}$/.test(p.category) ||
        typeof p.reason !== "string" ||
        !/^[a-z][a-z_-]{0,127}$/.test(p.reason))
        throw new Error();
    return { code: Number(p.code), category: p.category, reason: p.reason };
}
function refused(p, key) {
    const kind = p.code === 401 || p.code === 403
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
function checkScope(frame, scope, assigned) {
    if (!scope.sid &&
        !scope.connid &&
        !scope.operation_id &&
        eventKinds.has(frame.type)) {
        if (frame.cid !== scope.cid)
            throw new Error();
        if (frame.type === "conversation.event") {
            const data = frame.payload.data;
            if (!jsonObject(data) ||
                frame.payload.event_id !== data.event_id ||
                frame.payload.cursor !== data.cursor ||
                frame.cid !== data.conversation_id ||
                frame.sid !== (data.session_id ?? undefined) ||
                frame.connid !== (data.connection_id ?? undefined) ||
                frame.operation_id !== (data.operation_id ?? undefined))
                throw new Error();
        }
        else if (frame.type.startsWith("session.") ||
            frame.type === "recording.vcon-ready") {
            if (!identifier(frame.sid) ||
                frame.connid !== undefined ||
                frame.operation_id !== undefined)
                throw new Error();
            if (frame.type === "recording.vcon-ready") {
                const handle = frame.payload.vcon_handle;
                if (!jsonObject(handle) ||
                    typeof handle.uuid !== "string" ||
                    !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(handle.uuid) ||
                    !Number.isSafeInteger(handle.revision) ||
                    Number(handle.revision) < 1)
                    throw new Error();
            }
        }
        else if (frame.sid !== undefined ||
            frame.connid !== undefined ||
            !jsonObject(frame.payload.evidence) ||
            !identifier(frame.payload.evidence.event_id))
            throw new Error();
        return;
    }
    for (const field of ["sid", "connid", "operation_id"])
        if (frame[field] !== scope[field])
            throw new Error();
    if (frame.cid !== scope.cid && !(assigned && scope.cid === undefined))
        throw new Error();
}
/** Explicit finite connect/2 control. No credential refresh, retry, transport
 * fallback, native media ownership or live-event lease is implied by HTTP. */
export class CanonicalHttpClient {
    endpoint;
    credential;
    active = new Map();
    closed = false;
    constructor(config) {
        try {
            const url = new URL(config.endpoint);
            if (url.protocol !== "http:" ||
                !["127.0.0.1", "[::1]", "localhost"].includes(url.hostname) ||
                url.username ||
                url.password ||
                url.search ||
                url.hash ||
                (url.pathname !== "/" && url.pathname !== "") ||
                !config.credential ||
                config.credential.length > 16_384 ||
                /[\x00-\x20\x7f]/.test(config.credential))
                throw new Error();
            this.endpoint = url.origin + "/v2/commands";
            this.credential = config.credential;
        }
        catch {
            throw new CanonicalError("configuration", "not-submitted");
        }
    }
    async request(type, payload, scope, options) {
        let frame, body, key;
        try {
            duration(options);
            if (Object.keys(scope).some((k) => ![
                "cid",
                "sid",
                "connid",
                "operation_id",
                "expected_revision",
            ].includes(k)))
                throw new Error();
            frame = envelope(type, payload, scope);
            if ((scope.sid && !scope.cid) || (scope.connid && !scope.sid))
                throw new Error();
            if (!commands.has(type) ||
                (type === "conversation.subscribe" && payload.live !== false))
                throw new CanonicalError("unsupported", "not-submitted");
            if (!reads.has(type)) {
                if (!identifier(scope.operation_id))
                    throw new Error();
                key = Object.freeze({
                    operationId: scope.operation_id,
                    command: type,
                    scope: Object.freeze({
                        ...(scope.cid === undefined
                            ? {}
                            : { conversationId: scope.cid }),
                        ...(scope.sid === undefined
                            ? {}
                            : { sessionId: scope.sid }),
                        ...(scope.connid === undefined
                            ? {}
                            : { connectionId: scope.connid }),
                    }),
                });
            }
            body = canonicalJson(frame);
            if (Buffer.byteLength(body) > maximumRequest)
                throw new Error();
        }
        catch (e) {
            if (e instanceof CanonicalError)
                throw e;
            throw new CanonicalError("validation", "not-submitted", key);
        }
        // Capture intent before yielding: callers may mutate the original objects.
        const savedScope = { ...scope };
        const error = (kind, sent) => new CanonicalError(kind, key ? (sent ? "uncertain" : "not-submitted") : "read-only", key);
        if (options.signal?.aborted)
            throw error("cancelled", false);
        if (this.closed)
            throw error("configuration", false);
        if (this.active.size >= 16)
            throw error("application", false);
        const controller = new AbortController();
        let done;
        this.active.set(controller, new Promise((resolve) => {
            done = resolve;
        }));
        let timeout = false;
        const until = performance.now() + options.timeoutMs;
        const timer = setTimeout(() => {
            timeout = true;
            controller.abort();
        }, options.timeoutMs);
        const abort = () => controller.abort();
        const signal = options.signal;
        signal?.addEventListener("abort", abort, { once: true });
        try {
            const response = await fetch(this.endpoint, {
                method: "POST",
                redirect: "manual",
                body,
                headers: {
                    "content-type": "application/json",
                    authorization: "Bearer " + this.credential,
                },
                signal: controller.signal,
            });
            if (response.status >= 300 && response.status < 400) {
                await response.body?.cancel();
                throw error("protocol", true);
            }
            if (response.headers
                .get("content-type")
                ?.split(";", 1)[0]
                .trim()
                .toLowerCase() !== "application/json") {
                await response.body?.cancel();
                throw error("protocol", true);
            }
            const length = response.headers.get("content-length");
            if (length !== null &&
                (!/^\d+$/.test(length) || Number(length) > maximumResponse)) {
                await response.body?.cancel();
                throw error("protocol", true);
            }
            const reader = response.body?.getReader();
            if (!reader)
                throw error("protocol", true);
            const buffer = new Uint8Array(maximumResponse);
            let size = 0;
            try {
                for (;;) {
                    const part = await reader.read();
                    if (part.done)
                        break;
                    if (size + part.value.byteLength > maximumResponse)
                        throw error("protocol", true);
                    buffer.set(part.value, size);
                    size += part.value.byteLength;
                }
            }
            finally {
                await reader.cancel().catch(() => { });
                reader.releaseLock();
            }
            const value = parseCanonicalJson(buffer.subarray(0, size), maximumResponse);
            if (performance.now() >= until)
                throw error("deadline", true);
            if (!jsonObject(value) || value.profile !== "connect/2")
                throw new Error();
            if ("problem" in value) {
                if (Object.keys(value).some((k) => !["profile", "problem"].includes(k)))
                    throw new Error();
                const p = problem(value.problem);
                if (p.code !== response.status)
                    throw new Error();
                throw refused(p, key);
            }
            if (Object.keys(value).some((k) => !["profile", "request_id", "envelopes"].includes(k)) ||
                value.request_id !== frame.id ||
                !Array.isArray(value.envelopes) ||
                !value.envelopes.length ||
                value.envelopes.length > 128)
                throw new Error();
            const frames = [];
            const ids = new Set();
            for (const item of value.envelopes) {
                validateEnvelope(item);
                canonicalJson(item); // Per-envelope byte, depth and scalar bounds.
                if (Object.keys(item).some((k) => !topFields.has(k)) ||
                    item.in_reply_to !== frame.id ||
                    ids.has(item.id) ||
                    frames.at(-1)?.type === "ack" ||
                    frames.at(-1)?.type === "error")
                    throw new Error();
                checkScope(item, savedScope, type === "conversation.create" ||
                    type === "conversation.list");
                ids.add(item.id);
                frames.push(item);
            }
            const terminal = frames.at(-1);
            if (performance.now() >= until)
                throw error("deadline", true);
            if (terminal.type === "error") {
                const p = problem(terminal.payload);
                if (response.status !== p.code)
                    throw new Error();
                throw refused(p, key);
            }
            if (terminal.type !== "ack" || response.status !== 200)
                throw new Error();
            return frames;
        }
        catch (e) {
            if (e instanceof CanonicalError)
                throw e;
            if (controller.signal.aborted)
                throw error(timeout ? "deadline" : "cancelled", true);
            // JSON/schema/correlation errors are private and never echo the body.
            if (e instanceof TypeError)
                throw error("transport-uncertain", true);
            throw error("protocol", true);
        }
        finally {
            clearTimeout(timer);
            signal?.removeEventListener("abort", abort);
            this.active.delete(controller);
            done();
        }
    }
    async close(options) {
        duration(options);
        this.closed = true;
        const work = [...this.active.entries()];
        for (const [controller] of work)
            controller.abort();
        let timer;
        let abort;
        try {
            await Promise.race([
                Promise.all(work.map(([, done]) => done)),
                new Promise((_, reject) => {
                    abort = () => reject(new CanonicalError("cleanup-incomplete", "read-only"));
                    timer = setTimeout(abort, options.timeoutMs);
                    options.signal?.addEventListener("abort", abort, {
                        once: true,
                    });
                    if (options.signal?.aborted)
                        abort();
                }),
            ]);
        }
        finally {
            clearTimeout(timer);
            options.signal?.removeEventListener("abort", abort);
        }
    }
}
