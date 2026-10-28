import {
    canonicalJson,
    jsonObject,
    parseCanonicalJson,
    type JsonObject,
} from "./uctp_json.js";
import {
    envelope,
    identifier,
    validateEnvelope,
    type Envelope,
    type Scope,
} from "./uctp_wire.js";
import {
    CanonicalError,
    type CanonicalErrorKind,
    type CanonicalProblem,
    type OperationKey,
} from "./canonical_error.js";

import {
    duration,
    problem,
    refused,
    checkScope,
    commands,
    reads,
    topFields,
    maximumRequest,
    maximumResponse,
    type CanonicalRequestOptions,
    type CanonicalCommandScope,
} from "./canonical_validation.js";
export type {
    CanonicalRequestOptions,
    CanonicalCommandScope,
} from "./canonical_validation.js";

/** Explicit finite connect/2 control. No credential refresh, retry, transport
 * fallback, native media ownership or live-event lease is implied by HTTP. */
export class CanonicalHttpClient {
    private readonly endpoint: string;
    private readonly credential: string;
    private readonly active = new Map<AbortController, Promise<void>>();
    private closed = false;
    constructor(config: {
        readonly endpoint: string;
        readonly credential: string;
    }) {
        try {
            const url = new URL(config.endpoint);
            if (
                url.protocol !== "http:" ||
                !["127.0.0.1", "[::1]", "localhost"].includes(url.hostname) ||
                url.username ||
                url.password ||
                url.search ||
                url.hash ||
                (url.pathname !== "/" && url.pathname !== "") ||
                !config.credential ||
                config.credential.length > 16_384 ||
                /[\x00-\x20\x7f]/.test(config.credential)
            )
                throw new Error();
            this.endpoint = url.origin + "/v2/commands";
            this.credential = config.credential;
        } catch {
            throw new CanonicalError("configuration", "not-submitted");
        }
    }
    async request(
        type: string,
        payload: JsonObject,
        scope: CanonicalCommandScope,
        options: CanonicalRequestOptions,
    ): Promise<readonly Envelope[]> {
        let frame: Envelope, body: string, key: OperationKey | undefined;
        try {
            duration(options);
            if (
                Object.keys(scope).some(
                    (k) =>
                        ![
                            "cid",
                            "sid",
                            "connid",
                            "operation_id",
                            "expected_revision",
                        ].includes(k),
                )
            )
                throw new Error();
            frame = envelope(type, payload, scope);
            if ((scope.sid && !scope.cid) || (scope.connid && !scope.sid))
                throw new Error();
            if (
                !commands.has(type) ||
                (type === "conversation.subscribe" && payload.live !== false)
            )
                throw new CanonicalError("unsupported", "not-submitted");
            if (!reads.has(type)) {
                if (!identifier(scope.operation_id)) throw new Error();
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
            if (Buffer.byteLength(body) > maximumRequest) throw new Error();
        } catch (e) {
            if (e instanceof CanonicalError) throw e;
            throw new CanonicalError("validation", "not-submitted", key);
        }
        // Capture intent before yielding: callers may mutate the original objects.
        const savedScope = { ...scope };
        const error = (kind: CanonicalErrorKind, sent: boolean) =>
            new CanonicalError(
                kind,
                key ? (sent ? "uncertain" : "not-submitted") : "read-only",
                key,
            );
        if (options.signal?.aborted) throw error("cancelled", false);
        if (this.closed) throw error("configuration", false);
        if (this.active.size >= 16) throw error("application", false);
        const controller = new AbortController();
        let done!: () => void;
        this.active.set(
            controller,
            new Promise<void>((resolve) => {
                done = resolve;
            }),
        );
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
            if (
                response.headers
                    .get("content-type")
                    ?.split(";", 1)[0]
                    .trim()
                    .toLowerCase() !== "application/json"
            ) {
                await response.body?.cancel();
                throw error("protocol", true);
            }
            const length = response.headers.get("content-length");
            if (
                length !== null &&
                (!/^\d+$/.test(length) || Number(length) > maximumResponse)
            ) {
                await response.body?.cancel();
                throw error("protocol", true);
            }
            const reader = response.body?.getReader();
            if (!reader) throw error("protocol", true);
            const buffer = new Uint8Array(maximumResponse);
            let size = 0;
            try {
                for (;;) {
                    const part = await reader.read();
                    if (part.done) break;
                    if (size + part.value.byteLength > maximumResponse)
                        throw error("protocol", true);
                    buffer.set(part.value, size);
                    size += part.value.byteLength;
                }
            } finally {
                await reader.cancel().catch(() => {});
                reader.releaseLock();
            }
            const value = parseCanonicalJson(
                buffer.subarray(0, size),
                maximumResponse,
            );
            if (performance.now() >= until) throw error("deadline", true);
            if (!jsonObject(value) || value.profile !== "connect/2")
                throw new Error();
            if ("problem" in value) {
                if (
                    Object.keys(value).some(
                        (k) => !["profile", "problem"].includes(k),
                    )
                )
                    throw new Error();
                const p = problem(value.problem);
                if (p.code !== response.status) throw new Error();
                throw refused(p, key);
            }
            if (
                Object.keys(value).some(
                    (k) => !["profile", "request_id", "envelopes"].includes(k),
                ) ||
                value.request_id !== frame.id ||
                !Array.isArray(value.envelopes) ||
                !value.envelopes.length ||
                value.envelopes.length > 128
            )
                throw new Error();
            const frames: Envelope[] = [];
            const ids = new Set<string>();
            for (const item of value.envelopes) {
                validateEnvelope(item);
                canonicalJson(item); // Per-envelope byte, depth and scalar bounds.
                if (
                    Object.keys(item).some((k) => !topFields.has(k)) ||
                    item.in_reply_to !== frame.id ||
                    ids.has(item.id) ||
                    frames.at(-1)?.type === "ack" ||
                    frames.at(-1)?.type === "error"
                )
                    throw new Error();
                checkScope(
                    item,
                    savedScope,
                    type === "conversation.create" ||
                        type === "conversation.list",
                );
                ids.add(item.id);
                frames.push(item);
            }
            const terminal = frames.at(-1)!;
            if (performance.now() >= until) throw error("deadline", true);
            if (terminal.type === "error") {
                const p = problem(terminal.payload);
                if (response.status !== p.code) throw new Error();
                throw refused(p, key);
            }
            if (terminal.type !== "ack" || response.status !== 200)
                throw new Error();
            return frames;
        } catch (e) {
            if (e instanceof CanonicalError) throw e;
            if (controller.signal.aborted)
                throw error(timeout ? "deadline" : "cancelled", true);
            // JSON/schema/correlation errors are private and never echo the body.
            if (e instanceof TypeError)
                throw error("transport-uncertain", true);
            throw error("protocol", true);
        } finally {
            clearTimeout(timer);
            signal?.removeEventListener("abort", abort);
            this.active.delete(controller);
            done();
        }
    }
    async close(options: CanonicalRequestOptions): Promise<void> {
        duration(options);
        this.closed = true;
        const work = [...this.active.entries()];
        for (const [controller] of work) controller.abort();
        let timer: ReturnType<typeof setTimeout> | undefined;
        let abort!: () => void;
        try {
            await Promise.race([
                Promise.all(work.map(([, done]) => done)),
                new Promise<never>((_, reject) => {
                    abort = () =>
                        reject(
                            new CanonicalError(
                                "cleanup-incomplete",
                                "read-only",
                            ),
                        );
                    timer = setTimeout(abort, options.timeoutMs);
                    options.signal?.addEventListener("abort", abort, {
                        once: true,
                    });
                    if (options.signal?.aborted) abort();
                }),
            ]);
        } finally {
            clearTimeout(timer);
            options.signal?.removeEventListener("abort", abort);
        }
    }
}
