import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { unbase64url } from "./uctp_signing.js";
import { canonicalJson, jsonObject, parseCanonicalJson, } from "./uctp_json.js";
import { envelope, identifier, validateEnvelope, } from "./uctp_wire.js";
import { CanonicalError, } from "./canonical_error.js";
import { checkScope, commands, duration, maximumRequest, maximumResponse, reads, topFields, } from "./canonical_validation.js";
const closedObject = (value, fields) => jsonObject(value) &&
    Object.keys(value).every((k) => fields.includes(k));
/** Current finite canonical control through one owned Rust helper. Rust verifies
 * TLS, enrollment, signatures and current authority. This adapter validates the
 * fixed helper protocol and retains its original verified signed envelopes.
 * Failure fences this instance; recovery uses a new client and the original
 * saved mutation key. No restart, credential reload or mutation retry occurs. */
export class CanonicalNativeClient {
    child;
    exited;
    queue = [];
    active;
    chunks = [];
    bytes = 0;
    closed = false;
    joined = false;
    constructor(config) {
        if (!config || typeof config !== "object")
            throw new CanonicalError("configuration", "not-submitted");
        const executable = config.executable ?? "connect";
        for (const value of [config.config, executable])
            if (typeof value !== "string" ||
                !value ||
                value.length > 4096 ||
                value.includes("\0"))
                throw new CanonicalError("configuration", "not-submitted");
        this.child = spawn(executable, ["stdio", "--config", config.config], {
            stdio: ["pipe", "pipe", "pipe"],
        });
        // Diagnostics are private; neither credentials nor raw helper output are
        // copied into SDK errors or the application's stdout/stderr.
        this.child.stderr.resume();
        this.exited = new Promise((resolve) => this.child.once("close", () => {
            this.joined = true;
            this.fence("transport-uncertain");
            resolve();
        }));
        this.child.once("error", () => this.pipeFailure());
        this.child.stdin.on("error", () => this.pipeFailure());
        this.child.stdout.on("data", (chunk) => this.receive(chunk));
        this.child.stdout.once("end", () => this.fence("transport-uncertain"));
    }
    async request(type, payload, scope, options) {
        const started = performance.now();
        let frame, key, line;
        const id = "node_" + randomUUID().replaceAll("-", "");
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
            const nativeScope = {};
            for (const [wire, native] of [
                ["cid", "conversation_id"],
                ["sid", "session_id"],
                ["connid", "connection_id"],
                ["operation_id", "operation_id"],
                ["expected_revision", "expected_revision"],
            ])
                if (frame[wire] !== undefined)
                    nativeScope[native] = frame[wire];
            line =
                canonicalJson({
                    id,
                    command: { type, payload: frame.payload },
                    scope: nativeScope,
                }) + "\n";
            if (Buffer.byteLength(line) > maximumRequest)
                throw new Error();
        }
        catch (error) {
            if (error instanceof CanonicalError)
                throw error;
            throw new CanonicalError("validation", "not-submitted", key);
        }
        const localError = (kind) => new CanonicalError(kind, key ? "not-submitted" : "read-only", key);
        if (options.signal?.aborted)
            throw localError("cancelled");
        if (performance.now() >= started + options.timeoutMs)
            throw localError("deadline");
        if (this.closed)
            throw localError("configuration");
        if (this.queue.length + Number(!!this.active) >= 16)
            throw localError("application");
        const savedScope = { ...scope }, expires = started + options.timeoutMs;
        return new Promise((resolve, reject) => {
            let job;
            const abort = () => this.cancel(job, "cancelled");
            const timer = setTimeout(() => this.cancel(job, "deadline"), options.timeoutMs);
            job = {
                id,
                line,
                command: type,
                scope: savedScope,
                key,
                expires,
                signal: options.signal,
                abort,
                timer,
                resolve,
                reject,
                sent: false,
            };
            options.signal?.addEventListener("abort", abort, { once: true });
            this.queue.push(job);
            this.next();
        });
    }
    pipeFailure() {
        const missing = this.child.pid === undefined;
        if (missing && this.active)
            this.active.sent = false;
        this.fence(missing ? "configuration" : "transport-uncertain");
    }
    error(job, kind) {
        return new CanonicalError(kind, job.key ? (job.sent ? "uncertain" : "not-submitted") : "read-only", job.key);
    }
    release(job) {
        clearTimeout(job.timer);
        job.signal?.removeEventListener("abort", job.abort);
    }
    cancel(job, kind) {
        if (this.active === job) {
            this.fence(kind);
            return;
        }
        const index = this.queue.indexOf(job);
        if (index < 0)
            return;
        this.queue.splice(index, 1);
        this.release(job);
        job.reject(this.error(job, kind));
    }
    next() {
        if (this.closed || this.active)
            return;
        const job = this.queue.shift();
        if (!job)
            return;
        if (job.signal?.aborted || performance.now() >= job.expires) {
            this.release(job);
            job.reject(this.error(job, job.signal?.aborted ? "cancelled" : "deadline"));
            this.next();
            return;
        }
        this.active = job;
        job.sent = true;
        try {
            this.child.stdin.write(job.line, (error) => {
                if (error)
                    this.pipeFailure();
            });
        }
        catch {
            this.pipeFailure();
        }
    }
    fence(kind) {
        this.closed = true;
        const jobs = [...(this.active ? [this.active] : []), ...this.queue];
        this.active = undefined;
        this.queue.length = 0;
        this.chunks = [];
        this.bytes = 0;
        for (const job of jobs) {
            this.release(job);
            job.reject(this.error(job, kind));
        }
        if (!this.joined &&
            this.child.exitCode === null &&
            this.child.signalCode === null)
            this.child.kill("SIGKILL");
    }
    receive(chunk) {
        if (this.closed)
            return;
        const job = this.active;
        if (!job || this.bytes + chunk.length > maximumResponse + 1) {
            this.fence("protocol");
            return;
        }
        if (performance.now() >= job.expires) {
            this.fence("deadline");
            return;
        }
        const newline = chunk.indexOf(10);
        if (newline >= 0 && newline !== chunk.length - 1) {
            this.fence("protocol");
            return;
        }
        this.chunks.push(chunk);
        this.bytes += chunk.length;
        if (newline < 0)
            return;
        const bytes = Buffer.concat(this.chunks, this.bytes).subarray(0, -1);
        this.chunks = [];
        this.bytes = 0;
        try {
            const value = parseCanonicalJson(bytes, maximumResponse);
            if (!closedObject(value, ["id", "response"]) ||
                value.id !== job.id ||
                !closedObject(value.response, ["profile", "result"]) ||
                value.response.profile !== "connect-control/1" ||
                !closedObject(value.response.result, ["Ok", "Err"]) ||
                Object.keys(value.response.result).length !== 1)
                throw new Error();
            const result = value.response.result;
            if ("Err" in result) {
                if (!closedObject(result.Err, ["code", "message"]) ||
                    typeof result.Err.code !== "string" ||
                    !/^[a-z][a-z0-9-]{0,63}$/.test(result.Err.code) ||
                    typeof result.Err.message !== "string")
                    throw new Error();
                const code = result.Err.code;
                if (code === "protocol" || code === "transport-uncertain") {
                    this.fence(code);
                    return;
                }
                const kind = code === "configuration"
                    ? "configuration"
                    : ["unauthorized", "forbidden"].includes(code)
                        ? "authentication"
                        : code === "conflict"
                            ? "conflict"
                            : code === "invalid-request"
                                ? "validation"
                                : [
                                    "unsupported",
                                    "not-supported",
                                    "unsupported-command",
                                    "incompatible-capabilities",
                                ].includes(code)
                                    ? "unsupported"
                                    : code === "resync-required"
                                        ? "resync-required"
                                        : "application";
                this.active = undefined;
                this.release(job);
                job.reject(new CanonicalError(kind, job.key ? "refused" : "read-only", job.key));
                this.next();
                return;
            }
            const ok = result.Ok;
            if (!closedObject(ok, ["kind", "data"]) ||
                ok.kind !== "canonical_reply" ||
                !closedObject(ok.data, ["request_id", "frames"]) ||
                !identifier(ok.data.request_id) ||
                !Array.isArray(ok.data.frames) ||
                ok.data.frames.length < 1 ||
                ok.data.frames.length > 128)
                throw new Error();
            const frames = [], ids = new Set();
            for (const item of ok.data.frames) {
                validateEnvelope(item);
                if (Buffer.byteLength(canonicalJson(item)) > 1024 * 1024 ||
                    Object.keys(item).some((k) => !topFields.has(k) && k !== "signature") ||
                    item.in_reply_to !== ok.data.request_id ||
                    ids.has(item.id) ||
                    ["ack", "error"].includes(frames.at(-1)?.type ?? ""))
                    throw new Error();
                // The owned Rust process already verified this exact signature.
                // This shape check makes no independent JavaScript trust claim.
                {
                    const sig = item.signature;
                    if (!closedObject(sig, ["keyid", "alg", "sig"]) ||
                        typeof sig.keyid !== "string" ||
                        !sig.keyid ||
                        sig.keyid.length > 512 ||
                        !["EdDSA", "ES256"].includes(String(sig.alg)) ||
                        typeof sig.sig !== "string" ||
                        unbase64url(sig.sig).length !== 64)
                        throw new Error();
                }
                checkScope(item, job.scope, job.command === "conversation.create" ||
                    job.command === "conversation.list");
                ids.add(item.id);
                frames.push(item);
            }
            if (frames.at(-1)?.type !== "ack")
                throw new Error();
            if (performance.now() >= job.expires) {
                this.fence("deadline");
                return;
            }
            this.active = undefined;
            this.release(job);
            job.resolve(frames);
            this.next();
        }
        catch {
            this.fence("protocol");
        }
    }
    /** EOF joins the original helper. An active abandoned exchange is uncertain;
     * close never submits Session/Connection end or retries its saved mutation. */
    async close(options) {
        duration(options);
        if (!this.closed) {
            this.closed = true;
            if (this.active || this.queue.length)
                this.fence("cancelled");
            else
                this.child.stdin.end();
        }
        if (this.joined)
            return;
        const signal = options.signal;
        await new Promise((resolve, reject) => {
            const abort = () => {
                this.fence("cancelled");
                cleanup();
                reject(new CanonicalError("cleanup-incomplete", "read-only"));
            };
            const timer = setTimeout(abort, options.timeoutMs);
            const cleanup = () => {
                clearTimeout(timer);
                signal?.removeEventListener("abort", abort);
            };
            this.exited.then(() => {
                cleanup();
                resolve();
            });
            signal?.addEventListener("abort", abort, { once: true });
            if (signal?.aborted)
                abort();
        });
    }
}
