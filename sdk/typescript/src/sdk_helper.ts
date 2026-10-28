import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { randomUUID } from "node:crypto";
import {
    CanonicalError,
    type CanonicalErrorKind,
    type OperationKey,
} from "./canonical_error.js";
import {
    canonicalJson,
    parseCanonicalJson,
    jsonObject,
    type Json,
    type JsonObject,
} from "./uctp_json.js";
import {
    duration,
    type CanonicalRequestOptions,
} from "./canonical_validation.js";

const profile = "connect-sdk-helper/4";
export function object(
    value: unknown,
    fields: readonly string[],
): value is JsonObject {
    return (
        jsonObject(value as JsonObject) &&
        Object.keys(value as object).every((k) => fields.includes(k))
    );
}
export class NodeSdkError extends CanonicalError {
    constructor(
        kind: CanonicalErrorKind,
        certainty: CanonicalError["certainty"],
        operation?: OperationKey,
        problem?: CanonicalError["problem"],
        readonly observation?: Json,
    ) {
        super(kind, certainty, operation, problem);
        this.name = "NodeSdkError";
    }
}
interface Job {
    id: string;
    action: JsonObject;
    expires: number;
    key?: OperationKey;
    mutation: boolean;
    sent: boolean;
    options: CanonicalRequestOptions;
    abort: () => void;
    timer: ReturnType<typeof setTimeout>;
    decode: (value: Json) => unknown;
    resolve: (value: any) => void;
    reject: (error: NodeSdkError) => void;
}
/** Internal framing only. The Rust SDK owns trust, journals and typed behavior. */
export class SdkHelper {
    private readonly child: ChildProcessWithoutNullStreams;
    private readonly wireProfile: string;
    private readonly administrative: boolean;
    private readonly exited: Promise<void>;
    private joined = false;
    private terminating = false;
    private terminationTimer?: ReturnType<typeof setTimeout>;
    private closed = false;
    private readonly queue: Job[] = [];
    private active?: Job;
    private chunks: Buffer[] = [];
    private bytes = 0;
    constructor(
        config: {
            readonly config: string;
            readonly executable?: string;
        },
        backend: "participant" | "administrative" = "participant",
    ) {
        if (backend !== "participant" && backend !== "administrative")
            throw new NodeSdkError("configuration", "not-submitted");
        this.administrative = backend === "administrative";
        this.wireProfile = this.administrative
            ? "connect-sdk-administration/2"
            : profile;
        const executable = config?.executable ?? "connect";
        if (
            !config ||
            [config.config, executable].some(
                (v) =>
                    typeof v !== "string" ||
                    !v ||
                    v.length > 4096 ||
                    v.includes("\0"),
            )
        )
            throw new NodeSdkError("configuration", "not-submitted");
        this.child = spawn(
            executable,
            [
                this.administrative ? "sdk-admin-stdio" : "sdk-stdio",
                "--config",
                config.config,
            ],
            { stdio: ["pipe", "pipe", "pipe"] },
        );
        this.child.stderr.resume();
        this.exited = new Promise((resolve) =>
            this.child.once("close", () => {
                this.joined = true;
                clearTimeout(this.terminationTimer);
                this.fence("transport-uncertain");
                resolve();
            }),
        );
        const failed = () => {
            if (this.child.pid === undefined && this.active)
                this.active.sent = false;
            this.fence(
                this.child.pid === undefined
                    ? "configuration"
                    : "transport-uncertain",
            );
        };
        this.child.once("error", failed);
        this.child.stdin.on("error", failed);
        this.child.stdout.on("data", (chunk: Buffer) => this.receive(chunk));
        this.child.stdout.once("end", () => this.fence("transport-uncertain"));
    }
    request<T>(
        action: JsonObject,
        options: CanonicalRequestOptions,
        decode: (value: Json) => T,
        key?: OperationKey,
        mutation = false,
    ): Promise<T> {
        const started = performance.now();
        let captured: JsonObject;
        try {
            duration(options);
            const raw = action.type === "raw_request";
            const bytes = Buffer.from(
                canonicalJson(
                    action,
                    raw ? 2 * 1024 * 1024 : 1024 * 1024,
                    raw || this.administrative ? 40 : 32,
                    raw ? 1100000 : 100000,
                ),
            );
            if (
                bytes.length >
                (action.type === "raw_request"
                    ? 1024 * 1024 + 1024
                    : this.administrative
                      ? 65536 + 1024
                      : 63 * 1024)
            )
                throw new Error();
            captured = parseCanonicalJson(
                bytes,
                raw ? 2 * 1024 * 1024 : 1024 * 1024,
                raw || this.administrative ? 40 : 32,
                raw ? 1100000 : 100000,
            ) as JsonObject;
        } catch {
            throw new NodeSdkError(
                "validation",
                mutation ? "not-submitted" : "read-only",
                key,
            );
        }
        const error = (kind: CanonicalErrorKind) =>
            new NodeSdkError(
                kind,
                mutation ? "not-submitted" : "read-only",
                key,
            );
        if (options.signal?.aborted) throw error("cancelled");
        const expires = started + options.timeoutMs;
        if (performance.now() >= expires) throw error("deadline");
        if (this.closed) throw error("configuration");
        if (this.queue.length + Number(!!this.active) >= 16)
            throw error("application");
        return new Promise((resolve, reject) => {
            let job: Job;
            const abort = () => this.cancel(job, "cancelled");
            job = {
                id: "sdk_" + randomUUID().replaceAll("-", ""),
                action: captured,
                options: { ...options },
                expires,
                key,
                mutation,
                sent: false,
                decode,
                resolve,
                reject,
                abort,
                timer: setTimeout(
                    () => this.cancel(job, "deadline"),
                    Math.max(1, expires - performance.now()),
                ),
            };
            options.signal?.addEventListener("abort", abort, { once: true });
            this.queue.push(job);
            this.next();
        });
    }
    private error(job: Job, kind: CanonicalErrorKind): NodeSdkError {
        return new NodeSdkError(
            kind,
            job.mutation
                ? job.sent
                    ? "uncertain"
                    : "not-submitted"
                : "read-only",
            job.key,
        );
    }
    private release(job: Job): void {
        clearTimeout(job.timer);
        job.options.signal?.removeEventListener("abort", job.abort);
    }
    private cancel(job: Job, kind: "deadline" | "cancelled"): void {
        if (this.active === job) {
            this.fence(kind);
            return;
        }
        const i = this.queue.indexOf(job);
        if (i < 0) return;
        this.queue.splice(i, 1);
        this.release(job);
        job.reject(this.error(job, kind));
    }
    private next(): void {
        if (this.closed || this.active) return;
        const job = this.queue.shift();
        if (!job) return;
        const remaining = Math.floor(job.expires - performance.now());
        if (remaining < 1 || job.options.signal?.aborted) {
            this.release(job);
            job.reject(
                this.error(
                    job,
                    job.options.signal?.aborted ? "cancelled" : "deadline",
                ),
            );
            this.next();
            return;
        }
        this.active = job;
        try {
            const line =
                canonicalJson(
                    {
                        profile: this.wireProfile,
                        id: job.id,
                        request: { timeout_ms: remaining, action: job.action },
                    },
                    job.action.type === "raw_request"
                        ? 2 * 1024 * 1024
                        : 1024 * 1024,
                    job.action.type === "raw_request" || this.administrative
                        ? 40
                        : 32,
                    job.action.type === "raw_request" ? 1100000 : 100000,
                ) + "\n";
            if (
                Buffer.byteLength(line) >
                (job.action.type === "raw_request"
                    ? 2 * 1024 * 1024
                    : this.administrative
                      ? 128 * 1024
                      : 65536)
            )
                throw new Error();
            job.sent = true;
            this.child.stdin.write(line, (error) => {
                if (error) this.fence("transport-uncertain");
            });
        } catch {
            this.fence("protocol");
        }
    }
    fence(kind: CanonicalErrorKind): void {
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
        this.terminate(false);
    }
    private terminate(force: boolean): void {
        if (
            !this.joined &&
            this.child.exitCode === null &&
            this.child.signalCode === null
        ) {
            if (force) {
                clearTimeout(this.terminationTimer);
                this.child.kill("SIGKILL");
            } else if (!this.terminating) {
                this.terminating = true;
                // EOF also releases Tokio's blocking stdin reader during an
                // idle shutdown; no further logical request may be admitted.
                this.child.stdin.end();
                this.child.kill("SIGTERM");
                this.terminationTimer = setTimeout(
                    () => this.terminate(true),
                    5000,
                );
                this.terminationTimer.unref();
            }
        }
    }
    private receive(chunk: Buffer): void {
        if (this.closed) return;
        const job = this.active;
        if (!job || this.bytes + chunk.length > 2 * 1024 * 1024 + 1) {
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
        if (newline < 0) return;
        try {
            const v = parseCanonicalJson(
                Buffer.concat(this.chunks, this.bytes).subarray(0, -1),
                2 * 1024 * 1024,
                job.action.type === "raw_request" ||
                    job.action.type === "events_poll" ||
                    this.administrative
                    ? 40
                    : 32,
                job.action.type === "raw_request" ||
                    job.action.type === "events_poll"
                    ? 1100000
                    : 100000,
            );
            this.chunks = [];
            this.bytes = 0;
            if (
                !object(v, ["profile", "id", "phase", "result"]) ||
                v.profile !== this.wireProfile ||
                v.id !== job.id ||
                typeof v.phase !== "string" ||
                !["startup", "request"].includes(v.phase) ||
                !object(v.result, ["Ok", "Err"]) ||
                Object.keys(v.result).length !== 1
            )
                throw new Error();
            let error: NodeSdkError | undefined, result: unknown;
            if ("Err" in v.result) {
                const e = v.result.Err;
                if (
                    !object(e, [
                        "kind",
                        "certainty",
                        "operation",
                        "problem",
                        "observation",
                    ]) ||
                    typeof e.kind !== "string" ||
                    ![
                        "validation",
                        "configuration",
                        "trust",
                        "authentication",
                        "protocol",
                        "deadline",
                        "cancelled",
                        "transport-uncertain",
                        "unsupported",
                        "conflict",
                        "application",
                        "receipt-unavailable",
                        "resync-required",
                        "cleanup-incomplete",
                    ].includes(e.kind) ||
                    typeof e.certainty !== "string" ||
                    ![
                        "not-submitted",
                        "refused",
                        "uncertain",
                        "recorded",
                        "read-only",
                    ].includes(e.certainty)
                )
                    throw new Error();
                // The trusted SDK's operation must agree with our captured intent.
                // Null is allowed for failures before Rust constructed its key.
                if (
                    e.operation !== null &&
                    canonicalJson(e.operation) !==
                        canonicalJson(wireKey(job.key))
                )
                    throw new Error();
                const p = e.problem;
                if (
                    p !== null &&
                    (!object(p, ["code", "category", "reason"]) ||
                        !Number.isSafeInteger(p.code) ||
                        Number(p.code) < 0 ||
                        typeof p.category !== "string" ||
                        typeof p.reason !== "string")
                )
                    throw new Error();
                if (
                    v.phase === "startup" &&
                    (e.operation !== null ||
                        p !== null ||
                        e.observation !== null ||
                        !["not-submitted", "read-only"].includes(e.certainty))
                )
                    throw new Error();
                error = new NodeSdkError(
                    e.kind as CanonicalErrorKind,
                    v.phase === "startup"
                        ? job.mutation
                            ? "not-submitted"
                            : "read-only"
                        : (e.certainty as NodeSdkError["certainty"]),
                    job.key,
                    p === null
                        ? undefined
                        : (p as unknown as NodeSdkError["problem"]),
                    e.observation === null ? undefined : e.observation,
                );
            } else {
                if (v.phase !== "request") throw new Error();
                result = job.decode(v.result.Ok);
            }
            if (performance.now() >= job.expires) {
                this.fence("deadline");
                return;
            }
            this.active = undefined;
            this.release(job);
            if (error) {
                job.reject(error);
                if (
                    [
                        "protocol",
                        "transport-uncertain",
                        "cleanup-incomplete",
                    ].includes(error.kind) &&
                    !(
                        error.kind === "protocol" &&
                        job.action.type === "events_poll" &&
                        v.phase === "request"
                    )
                ) {
                    this.fence(error.kind);
                    return;
                }
            } else job.resolve(result);
            this.next();
        } catch {
            this.fence("protocol");
        }
    }
    async close(options: CanonicalRequestOptions): Promise<void> {
        duration(options);
        if (!this.closed) {
            this.closed = true;
            if (this.active || this.queue.length) this.fence("cancelled");
            else this.child.stdin.end();
        }
        if (this.joined) return;
        await new Promise<void>((resolve, reject) => {
            const cleanup = () => {
                clearTimeout(timer);
                options.signal?.removeEventListener("abort", abort);
            };
            const abort = () => {
                this.fence("cancelled");
                this.terminate(true);
                cleanup();
                reject(new NodeSdkError("cleanup-incomplete", "read-only"));
            };
            const timer = setTimeout(abort, options.timeoutMs);
            this.exited.then(() => {
                cleanup();
                resolve();
            });
            options.signal?.addEventListener("abort", abort, { once: true });
            if (options.signal?.aborted) abort();
        });
    }
}
function wireKey(key?: OperationKey): Json {
    return key
        ? {
              operationId: key.operationId,
              command: key.command,
              scope: {
                  conversationId: key.scope.conversationId ?? null,
                  sessionId: key.scope.sessionId ?? null,
                  connectionId: key.scope.connectionId ?? null,
              },
          }
        : null;
}
