/** One owned authenticated C6M2 channel. Application waits never own its IO. */
import { createServer, type Socket } from "node:net";
import { randomBytes, timingSafeEqual } from "node:crypto";
import { SdkHelper, NodeSdkError } from "./sdk_helper.js";
import {
    canonicalJson,
    parseCanonicalJson,
    type Json,
    type JsonObject,
} from "./uctp_json.js";
import {
    duration,
    type CanonicalRequestOptions,
} from "./canonical_validation.js";
import type {
    CanonicalErrorKind,
    OperationKey,
    Certainty,
} from "./canonical_error.js";
import { mediaFrozen, mediaJson } from "./node_media_types.js";
export const MEDIA_PROFILE = "connect-sdk-media/2";
export const MEDIA_METADATA = 16384,
    MEDIA_PAYLOAD = 65536,
    MEDIA_WINDOW = 16;
export interface MediaFrame {
    kind: number;
    id: number;
    metadata: JsonObject;
    payload: Buffer;
}
export function mediaError(
    kind: CanonicalErrorKind,
    mutation = false,
    sent = false,
    key?: OperationKey,
): NodeSdkError {
    return new NodeSdkError(
        kind,
        mutation ? (sent ? "uncertain" : "not-submitted") : "read-only",
        key,
    );
}
export function exact(v: unknown, keys: readonly string[]): v is JsonObject {
    return (
        v !== null &&
        typeof v === "object" &&
        !Array.isArray(v) &&
        Object.keys(v).length === keys.length &&
        keys.every((k) => Object.hasOwn(v, k))
    );
}
export function mediaDuration(o: CanonicalRequestOptions): void {
    try {
        duration(o);
        if (
            o.timeoutMs > 60000 ||
            (o.signal !== undefined && !(o.signal instanceof AbortSignal))
        )
            throw new Error();
    } catch {
        throw mediaError("validation");
    }
}
export function encodeFrame(
    kind: number,
    id: number,
    metadata: JsonObject,
    payload: Buffer = Buffer.alloc(0),
): Buffer {
    if (
        ![1, 0x81, 0x82, 0x83, 0xff].includes(kind) ||
        !Number.isInteger(id) ||
        id < 1 ||
        id > 0xffffffff ||
        payload.length > MEDIA_PAYLOAD
    )
        throw new Error();
    const meta = mediaJson(metadata, MEDIA_METADATA);
    const header = Buffer.alloc(17);
    header.write("C6M2");
    header[4] = kind;
    header.writeUInt32BE(id, 5);
    header.writeUInt32BE(meta.length, 9);
    header.writeUInt32BE(payload.length, 13);
    return Buffer.concat([header, meta, payload]);
}
/** Header bounds precede allocation. Partial bytes belong to this reader. */
export class MediaFrameReader {
    private readonly header = Buffer.alloc(17);
    private used = 0;
    private body?: Buffer;
    private filled = 0;
    private metadataBytes = 0;
    private failed = false;
    constructor(
        public onFrame: (frame: MediaFrame) => void,
        public onFailure: (kind: CanonicalErrorKind) => void,
    ) {}
    push(chunk: Buffer): void {
        if (this.failed) return;
        try {
            let offset = 0;
            while (offset < chunk.length) {
                if (!this.body) {
                    const size = Math.min(
                        17 - this.used,
                        chunk.length - offset,
                    );
                    chunk.copy(this.header, this.used, offset, offset + size);
                    this.used += size;
                    offset += size;
                    if (this.used < 17) continue;
                    const h = this.header,
                        kind = h[4],
                        id = h.readUInt32BE(5),
                        m = h.readUInt32BE(9),
                        p = h.readUInt32BE(13);
                    if (
                        !h.subarray(0, 4).equals(Buffer.from("C6M2")) ||
                        ![1, 0x81, 0x82, 0x83, 0xff].includes(kind) ||
                        !id ||
                        m > MEDIA_METADATA ||
                        p > MEDIA_PAYLOAD
                    )
                        throw new Error();
                    this.metadataBytes = m;
                    this.body = Buffer.alloc(m + p);
                    this.filled = 0;
                }
                const size = Math.min(
                    this.body.length - this.filled,
                    chunk.length - offset,
                );
                chunk.copy(this.body, this.filled, offset, offset + size);
                offset += size;
                this.filled += size;
                if (this.filled === this.body.length) {
                    const metadata = parseCanonicalJson(
                        this.body.subarray(0, this.metadataBytes),
                        MEDIA_METADATA,
                    );
                    if (
                        metadata === null ||
                        typeof metadata !== "object" ||
                        Array.isArray(metadata)
                    )
                        throw new Error();
                    const frame = {
                        kind: this.header[4],
                        id: this.header.readUInt32BE(5),
                        metadata,
                        payload: Buffer.from(
                            this.body.subarray(this.metadataBytes),
                        ),
                    };
                    this.used = 0;
                    this.body = undefined;
                    this.filled = 0;
                    this.onFrame(frame);
                    if (this.failed) return;
                }
            }
        } catch {
            this.fence("protocol");
        }
    }
    end(): void {
        this.fence(this.used || this.body ? "protocol" : "transport-uncertain");
    }
    fence(kind: CanonicalErrorKind): void {
        if (this.failed) return;
        this.failed = true;
        this.body = undefined;
        this.onFailure(kind);
    }
}
export function owned<T>(p: Promise<T>): Promise<T> {
    void p.catch(() => {});
    return p;
}
export function localWait<T>(
    promise: Promise<T>,
    options: CanonicalRequestOptions,
    mutation = false,
    sent = () => false,
    key?: OperationKey,
    closing = false,
): Promise<T> {
    try {
        options = { timeoutMs: options?.timeoutMs, signal: options?.signal };
        duration(options);
        if (
            options.signal !== undefined &&
            !(options.signal instanceof AbortSignal)
        )
            throw new Error();
    } catch {
        return Promise.reject(mediaError("validation", mutation, false, key));
    }
    return new Promise((resolve, reject) => {
        let timer: ReturnType<typeof setTimeout> | undefined;
        const cleanup = () => {
            clearTimeout(timer);
            options.signal?.removeEventListener("abort", abort);
        };
        const abort = () => {
            cleanup();
            reject(
                mediaError(
                    closing ? "cleanup-incomplete" : "cancelled",
                    mutation,
                    sent(),
                    key,
                ),
            );
        };
        if (options.signal?.aborted) {
            abort();
            return;
        }
        timer = setTimeout(() => {
            cleanup();
            reject(
                mediaError(
                    closing ? "cleanup-incomplete" : "deadline",
                    mutation,
                    sent(),
                    key,
                ),
            );
        }, options.timeoutMs);
        options.signal?.addEventListener("abort", abort, { once: true });
        promise.then(
            (v) => {
                cleanup();
                resolve(v);
            },
            (e) => {
                cleanup();
                reject(e);
            },
        );
    });
}
function write(socket: Socket, bytes: Buffer): Promise<void> {
    return new Promise((resolve, reject) =>
        socket.write(bytes, (e) => (e ? reject(e) : resolve())),
    );
}
interface SocketOwner {
    socket: Socket;
    closed: Promise<void>;
    reader: MediaFrameReader;
}
interface Job {
    id: number;
    command: JsonObject;
    payload: Buffer;
    expires: number;
    options: CanonicalRequestOptions;
    mutation: boolean;
    key?: OperationKey;
    resolve: (v: MediaFrame) => void;
    reject: (e: NodeSdkError) => void;
    settled: boolean;
    sent: boolean;
}
const errorKinds = new Set([
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
]);
const certainties = new Set([
    "not-submitted",
    "refused",
    "uncertain",
    "recorded",
    "read-only",
]);
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
export class MediaChannel {
    private sequence = 1;
    private closed = false;
    private closing = false;
    private readonly pending = new Map<number, Job>();
    private readonly queue: Job[] = [];
    private writer?: Promise<void>;
    constructor(
        private readonly owner: SocketOwner,
        private readonly listenerClosed: Promise<void>,
        private readonly observe: (v: Json, key: OperationKey) => void,
    ) {
        if (owner.socket.destroyed) throw mediaError("transport-uncertain");
        owner.reader.onFrame = (f) => this.receive(f);
        owner.reader.onFailure = (k) => this.fence(k);
    }
    get pendingCount(): number {
        return this.pending.size;
    }
    exchange(
        command: JsonObject,
        payload: Buffer,
        options: CanonicalRequestOptions,
        mutation = false,
        key?: OperationKey,
    ): Promise<MediaFrame> {
        try {
            mediaDuration(options);
        } catch {
            throw mediaError("validation", mutation, false, key);
        }
        if (options.signal?.aborted)
            throw mediaError("cancelled", mutation, false, key);
        if (this.closed || this.closing)
            throw mediaError("configuration", mutation, false, key);
        if (
            this.pending.size >= MEDIA_WINDOW ||
            this.queue.length >= MEDIA_WINDOW ||
            this.sequence >= 0xfffffffe
        )
            throw mediaError("application", mutation, false, key);
        let captured: JsonObject;
        try {
            captured = parseCanonicalJson(
                mediaJson(command, MEDIA_METADATA),
                MEDIA_METADATA,
            ) as JsonObject;
            encodeFrame(
                1,
                2,
                { timeout_ms: options.timeoutMs, command: captured },
                payload,
            );
        } catch {
            throw mediaError("validation", mutation, false, key);
        }
        let job!: Job;
        const original = owned(
            new Promise<MediaFrame>((resolve, reject) => {
                job = {
                    id: ++this.sequence,
                    command: captured,
                    payload: Buffer.from(payload),
                    options: { ...options },
                    expires: performance.now() + options.timeoutMs,
                    mutation,
                    key,
                    resolve,
                    reject,
                    settled: false,
                    sent: false,
                };
            }),
        );
        this.pending.set(job.id, job);
        this.queue.push(job);
        this.pump();
        return localWait(
            original,
            options,
            mutation,
            () => job.sent,
            key,
        ).catch((e) => {
            if (!job.sent) {
                this.pending.delete(job.id);
                job.settled = true;
                job.reject(mediaError("cancelled", mutation, false, key));
            }
            throw e;
        });
    }
    private pump(): void {
        if (this.writer || this.closed) return;
        this.writer = owned(
            (async () => {
                while (this.queue.length && !this.closed) {
                    const job = this.queue.shift()!;
                    if (job.settled) continue;
                    const remaining = Math.floor(
                        job.expires - performance.now(),
                    );
                    if (
                        job.options.signal?.aborted ||
                        remaining <= 0 ||
                        this.closing
                    ) {
                        this.pending.delete(job.id);
                        job.settled = true;
                        job.reject(
                            mediaError(
                                job.options.signal?.aborted || this.closing
                                    ? "cancelled"
                                    : "deadline",
                                job.mutation,
                                false,
                                job.key,
                            ),
                        );
                        continue;
                    }
                    const bytes = encodeFrame(
                        1,
                        job.id,
                        { timeout_ms: remaining, command: job.command },
                        job.payload,
                    );
                    job.sent = true;
                    await write(this.owner.socket, bytes);
                }
            })()
                .catch(() => this.fence("transport-uncertain"))
                .finally(() => {
                    this.writer = undefined;
                    if (this.queue.length && !this.closed) this.pump();
                }),
        );
    }
    private receive(frame: MediaFrame): void {
        const job = this.pending.get(frame.id);
        if (!job || ![0x81, 0x82, 0xff].includes(frame.kind)) {
            this.fence("protocol");
            return;
        }
        try {
            if (frame.kind === 0xff) {
                const e = frame.metadata;
                if (
                    frame.payload.length ||
                    !exact(e, [
                        "kind",
                        "certainty",
                        "operation",
                        "problem",
                        "observation",
                    ]) ||
                    !errorKinds.has(e.kind as string) ||
                    !certainties.has(e.certainty as string) ||
                    (e.operation !== null &&
                        canonicalJson(e.operation) !==
                            canonicalJson(wireKey(job.key))) ||
                    (!job.key &&
                        (e.observation !== null ||
                            e.certainty === "recorded")) ||
                    (job.key && e.certainty === "read-only")
                )
                    throw new Error();
                if (e.observation !== null) {
                    if (
                        !job.key ||
                        !exact(e.observation, [
                            "key",
                            "state",
                            "revision",
                            "recordId",
                            "cancelRequested",
                            "expiresAt",
                            "problem",
                        ])
                    )
                        throw new Error();
                    this.observe(e.observation, job.key);
                } else if (e.certainty === "recorded") throw new Error();
                const p = e.problem;
                if (
                    p !== null &&
                    (!exact(p, ["code", "category", "reason"]) ||
                        !Number.isSafeInteger(p.code) ||
                        Number(p.code) < 0 ||
                        Number(p.code) > 65535 ||
                        typeof p.category !== "string" ||
                        typeof p.reason !== "string")
                )
                    throw new Error();
                job.reject(
                    new NodeSdkError(
                        e.kind as CanonicalErrorKind,
                        job.mutation ? (e.certainty as Certainty) : "read-only",
                        job.key,
                        p === null ? undefined : (mediaFrozen(p) as any),
                        e.observation === null
                            ? undefined
                            : mediaFrozen(e.observation),
                    ),
                );
            } else job.resolve(frame);
            job.settled = true;
            this.pending.delete(frame.id);
        } catch {
            this.fence("protocol");
        }
    }
    markClosing(): void {
        this.closing = true;
        for (const job of this.pending.values())
            job.reject(
                mediaError("cancelled", job.mutation, job.sent, job.key),
            );
    }
    fence(kind: CanonicalErrorKind): void {
        if (this.closed) return;
        this.closed = true;
        this.owner.reader.fence(kind);
        for (const job of this.pending.values()) {
            job.settled = true;
            job.reject(mediaError(kind, job.mutation, job.sent, job.key));
        }
        this.pending.clear();
        this.queue.length = 0;
        this.owner.socket.destroy();
    }
    async close(): Promise<void> {
        this.fence("cancelled");
        const pending = Promise.all([
            this.owner.closed,
            this.listenerClosed,
            this.writer,
        ]);
        await localWait(
            pending,
            { timeoutMs: 1000 },
            false,
            () => false,
            undefined,
            true,
        );
    }
}
/** Attachment owns the listener/accepted sockets independently of caller waits. */
export async function openMediaChannel(
    helper: SdkHelper,
    observe: (v: Json, key: OperationKey) => void,
): Promise<MediaChannel> {
    const expires = performance.now() + 4000;
    const remaining = () => {
        const timeoutMs = Math.floor(expires - performance.now());
        if (timeoutMs <= 0) throw mediaError("deadline");
        return { timeoutMs };
    };
    const supports = await helper.request(
        { type: "capabilities_refresh" },
        remaining(),
        (value) => {
            if (
                !exact(value, [
                    "profile",
                    "runtime_role",
                    "selected_transport",
                    "fallback_reason",
                    "features",
                ]) ||
                value.profile !== "connect/2" ||
                value.runtime_role !== "rust" ||
                value.selected_transport !== "quic" ||
                value.fallback_reason !== null ||
                !Array.isArray(value.features) ||
                value.features.length > 128 ||
                value.features.some(
                    (f) =>
                        typeof f !== "string" || !/^[a-z0-9-]{1,128}$/.test(f),
                ) ||
                new Set(value.features).size !== value.features.length
            )
                throw new Error();
            return (
                value.features.includes("native-programmatic-media") &&
                value.features.includes("managed-media-ipc-v2")
            );
        },
    );
    if (!supports) throw mediaError("unsupported");
    const token = randomBytes(32).toString("hex");
    const sockets = new Set<SocketOwner>();
    let accepting = true,
        selected: SocketOwner | undefined;
    let resolveAccepted!: (owner: SocketOwner) => void;
    const accepted = owned(
        new Promise<SocketOwner>((resolve) => {
            resolveAccepted = resolve;
        }),
    );
    const server = createServer(
        { highWaterMark: MEDIA_METADATA + MEDIA_PAYLOAD + 17 },
        (socket) => {
            if (!accepting || sockets.size >= 2) {
                socket.destroy();
                return;
            }
            let owner!: SocketOwner;
            const closed = new Promise<void>((resolve) =>
                socket.once("close", () => {
                    sockets.delete(owner);
                    resolve();
                }),
            );
            const reject = () => socket.destroy();
            const reader = new MediaFrameReader((frame) => {
                if (
                    selected ||
                    frame.kind !== 1 ||
                    frame.id !== 1 ||
                    frame.payload.length ||
                    !exact(frame.metadata, ["profile", "token"]) ||
                    frame.metadata.profile !== MEDIA_PROFILE ||
                    typeof frame.metadata.token !== "string" ||
                    frame.metadata.token.length !== 64 ||
                    !timingSafeEqual(
                        Buffer.from(frame.metadata.token),
                        Buffer.from(token),
                    )
                ) {
                    reject();
                    return;
                }
                selected = owner;
                accepting = false;
                reader.onFrame = () => reader.fence("protocol");
                void owned(
                    write(
                        socket,
                        encodeFrame(0x81, 1, { profile: MEDIA_PROFILE, token }),
                    ).then(() => resolveAccepted(owner), reject),
                );
            }, reject);
            owner = { socket, reader, closed };
            sockets.add(owner);
            socket.on("data", (chunk) => reader.push(chunk as Buffer));
            socket.on("error", () => reader.fence("transport-uncertain"));
            socket.on("end", () => reader.end());
            socket.once("close", () => reader.fence("transport-uncertain"));
        },
    );
    server.on("error", () => {});
    let listenerClosed: Promise<void> | undefined;
    const stop = () =>
        (listenerClosed ??= new Promise<void>((resolve) => {
            accepting = false;
            server.close(() => resolve());
        }));
    let transferred = false;
    try {
        await localWait(
            new Promise<void>((resolve, reject) => {
                const fail = () => reject(mediaError("configuration"));
                server.once("error", fail);
                server.listen(0, "127.0.0.1", () => {
                    server.removeListener("error", fail);
                    resolve();
                });
            }),
            remaining(),
        );
        const address = server.address();
        if (!address || typeof address === "string")
            throw mediaError("configuration");
        const attach = owned(
            helper.request(
                {
                    type: "media_attach",
                    attachment: {
                        endpoint: `127.0.0.1:${address.port}`,
                        token,
                    },
                },
                remaining(),
                (value) => {
                    if (
                        !exact(value, [
                            "profile",
                            "attached",
                            "window",
                            "header_bytes",
                            "metadata_max_bytes",
                            "payload_max_bytes",
                        ]) ||
                        value.profile !== MEDIA_PROFILE ||
                        value.attached !== true ||
                        value.window !== 16 ||
                        value.header_bytes !== 17 ||
                        value.metadata_max_bytes !== MEDIA_METADATA ||
                        value.payload_max_bytes !== MEDIA_PAYLOAD
                    )
                        throw new Error();
                    return true;
                },
            ),
        );
        await localWait(attach, remaining());
        const peer = await localWait(accepted, remaining());
        for (const owner of sockets) if (owner !== peer) owner.socket.destroy();
        const channel = new MediaChannel(peer, stop(), observe);
        transferred = true;
        return channel;
    } finally {
        stop();
        if (!transferred) {
            for (const owner of sockets) owner.socket.destroy();
            await localWait(
                Promise.all(
                    [...sockets].map((s) => s.closed).concat(listenerClosed!),
                ),
                { timeoutMs: 1000 },
                false,
                () => false,
                undefined,
                true,
            );
        }
    }
}
