import {
    BrowserClient,
    BrowserClientOpenError,
    browserTransportSupport,
    type BrowserClientOptions,
    type RequestOptions,
} from "./browser_client.js";
import { ConnectError } from "./errors.js";
import { jsonObject, type JsonObject } from "./uctp_json.js";
import { identifier, type Envelope, type Scope } from "./uctp_wire.js";

/** The WSS label identifies a control/media path, never media readiness. */
export type BrowserTransportPath = "webtransport" | "wss-webrtc";
export type BrowserFallbackReason =
    | "preferred-not-configured"
    | "preferred-unsupported"
    | "preferred-transport-timeout"
    | "preferred-transport-failed";
export type BrowserEnrollment = Omit<
    BrowserClientOptions,
    "transport" | "url" | "timeoutMs" | "signal"
>;
export interface BrowserTransportClientOptions {
    identityId: string;
    deviceId: string;
    endpoints: Partial<Record<BrowserTransportPath, string>>;
    preferredTransport?: BrowserTransportPath;
    allowFallback?: boolean;
    /** Resolve fresh enrollment for each initial/reconnect round. The same
     * captured enrollment applies to both secure contacts in that round. */
    resolveEnrollment(context: {
        signal: AbortSignal;
        reason: "initial" | "reconnect";
        physicalEpoch: number;
    }): Promise<BrowserEnrollment>;
    timeoutMs?: number;
    attemptTimeoutMs?: number;
}
export interface BrowserTransportState {
    readonly state: "disconnected" | "connecting" | "connected" | "closed";
    readonly preferredTransport: BrowserTransportPath;
    readonly selectedTransport?: BrowserTransportPath;
    readonly fallbackReason?: BrowserFallbackReason;
    /** Local lifetime fence, not the server's durable Connection generation. */
    readonly physicalEpoch: number;
}
export interface BrowserOperationReference {
    readonly operationId: string;
    /** Exact canonical ledger kind, e.g. create, send or session.handoff. */
    readonly command: string;
    readonly scope: {
        readonly conversationId?: string;
        readonly sessionId?: string;
        readonly connectionId?: string;
    };
}
export interface BrowserConversationRecoveryPoint {
    readonly conversationId: string;
    readonly cursor?: string;
    readonly page?: string;
    readonly limit?: number;
}
export interface BrowserConversationRecoveryPage {
    readonly kind: "events" | "snapshot";
    readonly conversationId: string;
    readonly cursor: string;
    readonly hasMore: boolean;
    readonly nextPage?: string;
    /** Verified journal/snapshot frames only; advisory hints are excluded. */
    readonly frames: readonly Envelope[];
}
const paths: readonly BrowserTransportPath[] = ["webtransport", "wss-webrtc"];
const operationStates = new Set([
    "accepted",
    "preparing",
    "ready",
    "committing",
    "completed",
    "cancelled",
    "failed",
    "interrupted",
    "unknown",
]);
function duration(value: number | undefined, fallback: number): number {
    const result = value ?? fallback;
    if (!Number.isFinite(result) || result < 1 || result > 60000)
        throw new ConnectError(
            "invalid-request",
            "Deadline must be 1–60000 ms",
        );
    return result;
}
function opaque(value: unknown): value is string {
    return (
        typeof value === "string" &&
        value.length > 0 &&
        value.length <= 2048 &&
        /^[A-Za-z0-9._~-]+$/.test(value)
    );
}
function endpoint(
    value: string | undefined,
    path: BrowserTransportPath,
): string | undefined {
    if (value === undefined) return undefined;
    let parsed: URL;
    try {
        parsed = new URL(value);
    } catch {
        throw new ConnectError(
            "invalid-request",
            "Invalid canonical transport contact",
        );
    }
    if (
        value.length > 2048 ||
        parsed.href !== value ||
        parsed.username ||
        parsed.password ||
        parsed.search ||
        parsed.hash ||
        parsed.port === "0" ||
        parsed.protocol !== (path === "webtransport" ? "https:" : "wss:")
    )
        throw new ConnectError(
            "invalid-request",
            "Use an exact secure canonical transport contact",
        );
    return value;
}
function enrollment(value: BrowserEnrollment): BrowserEnrollment {
    const hashes = value.serverCertificateHashes;
    if (hashes !== undefined && (!Array.isArray(hashes) || hashes.length > 8))
        throw new ConnectError(
            "invalid-request",
            "At most eight explicit WebTransport certificate hashes",
        );
    return {
        identityId: value.identityId,
        credential: value.credential,
        device: {
            id: value.device.id,
            kind: value.device.kind,
            platform: value.device.platform,
        },
        signing: {
            key: value.signing.key,
            keyId: value.signing.keyId,
            algorithm: value.signing.algorithm,
        },
        server: {
            key: value.server.key,
            keyId: value.server.keyId,
            algorithm: value.server.algorithm,
            notBefore: value.server.notBefore,
            expiresAt: value.server.expiresAt,
        },
        serverCertificateHashes: hashes?.map((hash) => {
            const source = hash.value;
            if (
                hash.algorithm !== "sha-256" ||
                !(source instanceof ArrayBuffer || ArrayBuffer.isView(source))
            )
                throw new ConnectError(
                    "invalid-request",
                    "Use explicit SHA256 certificate hashes",
                );
            const bytes = ArrayBuffer.isView(source)
                ? new Uint8Array(
                      source.buffer,
                      source.byteOffset,
                      source.byteLength,
                  )
                : new Uint8Array(source);
            if (bytes.length !== 32)
                throw new ConnectError(
                    "invalid-request",
                    "A SHA256 certificate hash has 32 bytes",
                );
            return { algorithm: hash.algorithm, value: bytes.slice().buffer };
        }),
    };
}
class Budget {
    readonly expires: number;
    constructor(milliseconds: number) {
        this.expires = performance.now() + milliseconds;
    }
    remaining(): number {
        const result = this.expires - performance.now();
        if (result < 1)
            throw new ConnectError(
                "timeout",
                "Original browser transport budget expired",
            );
        return result;
    }
    run<T>(
        work: () => Promise<T>,
        signal: AbortSignal,
        expired: () => void,
    ): Promise<T> {
        const remaining = this.remaining();
        if (signal.aborted)
            return Promise.reject(
                new ConnectError(
                    "cancelled",
                    "Browser transport round cancelled",
                ),
            );
        return new Promise((resolve, reject) => {
            const cleanup = () => {
                clearTimeout(timer);
                signal.removeEventListener("abort", abort);
            };
            const abort = () => {
                cleanup();
                reject(
                    new ConnectError(
                        "cancelled",
                        "Browser transport round cancelled",
                    ),
                );
            };
            const timer = setTimeout(() => {
                cleanup();
                reject(
                    new ConnectError(
                        "timeout",
                        "Original browser transport budget expired",
                    ),
                );
                expired();
            }, remaining);
            signal.addEventListener("abort", abort, { once: true });
            if (signal.aborted) {
                abort();
                return;
            }
            try {
                work().then(
                    (v) => {
                        cleanup();
                        resolve(v);
                    },
                    (e) => {
                        cleanup();
                        reject(e);
                    },
                );
            } catch (error) {
                cleanup();
                reject(error);
            }
        });
    }
}

/** Owns physical selection/reconnect. It never queues or resubmits mutations.
 * A new physical epoch retires old capture/media owners; applications must
 * explicitly reconnect media or submit a distinct device-replacement intent. */
export class BrowserTransportClient {
    private readonly identityId: string;
    private readonly deviceId: string;
    private readonly endpoints: Partial<Record<BrowserTransportPath, string>>;
    private readonly resolveEnrollment: BrowserTransportClientOptions["resolveEnrollment"];
    private readonly allowFallback: boolean;
    private readonly timeoutMs: number;
    private readonly attemptTimeoutMs: number;
    private preferred: BrowserTransportPath;
    private current?: BrowserClient;
    private stopWatching?: () => void;
    private round?: AbortController;
    private phase: BrowserTransportState["state"] = "disconnected";
    private selected?: BrowserTransportPath;
    private fallback?: BrowserFallbackReason;
    private epoch = 0;
    private constructor(options: BrowserTransportClientOptions) {
        if (
            !identifier(options.identityId) ||
            !identifier(options.deviceId) ||
            typeof options.resolveEnrollment !== "function" ||
            (options.preferredTransport !== undefined &&
                !paths.includes(options.preferredTransport)) ||
            (options.allowFallback !== undefined &&
                typeof options.allowFallback !== "boolean")
        )
            throw new ConnectError(
                "invalid-request",
                "Explicit enrolled actor/device and transport policy required",
            );
        this.identityId = options.identityId;
        this.deviceId = options.deviceId;
        this.preferred = options.preferredTransport ?? "webtransport";
        this.allowFallback = options.allowFallback ?? true;
        this.endpoints = Object.freeze({
            webtransport: endpoint(
                options.endpoints.webtransport,
                "webtransport",
            ),
            "wss-webrtc": endpoint(
                options.endpoints["wss-webrtc"],
                "wss-webrtc",
            ),
        });
        if (!this.endpoints.webtransport && !this.endpoints["wss-webrtc"])
            throw new ConnectError(
                "invalid-request",
                "At least one configured canonical transport contact required",
            );
        this.resolveEnrollment = options.resolveEnrollment;
        this.timeoutMs = duration(options.timeoutMs, 10000);
        this.attemptTimeoutMs = duration(options.attemptTimeoutMs, 3000);
    }
    static async connect(
        options: BrowserTransportClientOptions,
        request: RequestOptions = {},
    ): Promise<BrowserTransportClient> {
        const owner = new BrowserTransportClient(options);
        try {
            await owner.openRound("initial", request);
            return owner;
        } catch (error) {
            owner.close();
            throw error;
        }
    }
    get status(): BrowserTransportState {
        return Object.freeze({
            state: this.phase,
            preferredTransport: this.preferred,
            selectedTransport: this.selected,
            fallbackReason: this.selected ? this.fallback : undefined,
            physicalEpoch: this.epoch,
        });
    }
    /** The actual authenticated physical client. It grants no media readiness. */
    get client(): BrowserClient {
        if (this.phase === "closed")
            throw new ConnectError("closed", "Browser transport owner closed");
        if (this.phase !== "connected" || !this.current || this.current.closed)
            throw new ConnectError(
                "transport",
                "No authenticated browser physical connection",
            );
        return this.current;
    }
    reconnect(
        options: RequestOptions & {
            preferredTransport?: BrowserTransportPath;
        } = {},
    ): Promise<void> {
        return this.openRound("reconnect", options);
    }
    private async openRound(
        reason: "initial" | "reconnect",
        options: RequestOptions & { preferredTransport?: BrowserTransportPath },
    ): Promise<void> {
        if (this.phase === "closed")
            throw new ConnectError("closed", "Browser transport owner closed");
        if (this.round)
            throw new ConnectError(
                "conflict",
                "A browser connection round is already in progress",
            );
        const preferred = options.preferredTransport ?? this.preferred;
        if (!paths.includes(preferred))
            throw new ConnectError(
                "invalid-request",
                "Unknown browser transport preference",
            );
        const budget = new Budget(duration(options.timeoutMs, this.timeoutMs));
        const signal = options.signal;
        if (signal?.aborted)
            throw new ConnectError(
                "cancelled",
                "Browser connection cancelled before opening",
            );
        const support = browserTransportSupport();
        if (!support.secureContext || !support.webCrypto)
            throw new ConnectError(
                "unsupported-transport",
                "Secure context and WebCrypto are required",
            );
        if (this.epoch === Number.MAX_SAFE_INTEGER)
            throw new ConnectError(
                "backpressure",
                "Physical browser lifetime counter exhausted",
            );
        const round = new AbortController();
        this.round = round;
        const abort = () => round.abort();
        signal?.addEventListener("abort", abort, { once: true });
        this.stopWatching?.();
        this.stopWatching = undefined;
        this.current?.close();
        this.current = undefined;
        this.preferred = preferred;
        this.phase = "connecting";
        this.selected = undefined;
        this.fallback = undefined;
        const epoch = ++this.epoch;
        try {
            const captured = enrollment(
                await budget.run(
                    () =>
                        this.resolveEnrollment({
                            signal: round.signal,
                            reason,
                            physicalEpoch: epoch,
                        }),
                    round.signal,
                    () => round.abort(),
                ),
            );
            if (
                captured.identityId !== this.identityId ||
                captured.device.id !== this.deviceId
            )
                throw new ConnectError(
                    "authentication",
                    "Reconnect must retain the enrolled actor/device; replacement is a separate intent",
                );
            const choices = this.allowFallback
                ? [preferred, ...paths.filter((p) => p !== preferred)]
                : [preferred];
            let last: unknown;
            for (let index = 0; index < choices.length; index++) {
                budget.remaining();
                if (round.signal.aborted)
                    throw new ConnectError(
                        "cancelled",
                        "Browser transport round cancelled",
                    );
                const path = choices[index],
                    url = this.endpoints[path];
                if (
                    !url ||
                    !(path === "webtransport"
                        ? support.webTransport
                        : support.webSocket)
                ) {
                    last ??= new ConnectError(
                        "unsupported-transport",
                        "Configured preferred browser transport is unavailable",
                    );
                    if (index === 0)
                        this.fallback = !url
                            ? "preferred-not-configured"
                            : "preferred-unsupported";
                    continue;
                }
                try {
                    const physical = await BrowserClient.open({
                        ...captured,
                        url,
                        transport:
                            path === "webtransport"
                                ? "webtransport"
                                : "websocket",
                        serverCertificateHashes:
                            path === "webtransport"
                                ? captured.serverCertificateHashes
                                : undefined,
                        timeoutMs: Math.min(
                            this.attemptTimeoutMs,
                            budget.remaining(),
                        ),
                        signal: round.signal,
                    });
                    try {
                        budget.remaining();
                        if (
                            round.signal.aborted ||
                            this.round !== round ||
                            this.epoch !== epoch
                        )
                            throw new ConnectError(
                                "cancelled",
                                "Browser physical result belongs to a retired round",
                            );
                    } catch (error) {
                        physical.close(error);
                        throw error;
                    }
                    this.current = physical;
                    this.selected = path;
                    if (path === preferred) this.fallback = undefined;
                    this.phase = "connected";
                    this.stopWatching = physical.onClose(() => {
                        if (this.current !== physical) return;
                        this.current = undefined;
                        this.selected = undefined;
                        if (this.phase !== "closed")
                            this.phase = "disconnected";
                    });
                    return;
                } catch (error) {
                    last = error;
                    if (
                        !(error instanceof BrowserClientOpenError) ||
                        error.phase !== "transport" ||
                        !["transport", "timeout"].includes(error.code) ||
                        round.signal.aborted
                    )
                        throw error;
                    if (index === 0)
                        this.fallback =
                            error.code === "timeout"
                                ? "preferred-transport-timeout"
                                : "preferred-transport-failed";
                }
            }
            throw (
                last ??
                new ConnectError(
                    "unsupported-transport",
                    "No usable configured browser transport",
                )
            );
        } finally {
            signal?.removeEventListener("abort", abort);
            if (this.round === round) this.round = undefined;
            if (this.phase === "connecting") this.phase = "disconnected";
        }
    }
    close(): void {
        this.phase = "closed";
        this.round?.abort();
        this.stopWatching?.();
        this.stopWatching = undefined;
        this.current?.close();
        this.current = undefined;
        this.selected = undefined;
    }
    private observed(client: BrowserClient, epoch: number): void {
        if (
            this.phase !== "connected" ||
            this.current !== client ||
            this.epoch !== epoch ||
            client.closed
        )
            throw new ConnectError(
                "cancelled",
                "Recovery response belongs to a retired physical peer",
            );
    }
    private malformed(client: BrowserClient, message: string): never {
        const error = new ConnectError("protocol", message);
        client.close(error);
        throw error;
    }
    /** Status-first recovery only. It never dispatches the referenced command. */
    async recoverOperation(
        reference: BrowserOperationReference,
        options: RequestOptions = {},
    ): Promise<Readonly<JsonObject>> {
        if (
            !reference ||
            typeof reference !== "object" ||
            !reference.scope ||
            typeof reference.scope !== "object"
        )
            throw new ConnectError(
                "invalid-request",
                "Complete original operation reference required",
            );
        const { operationId, command } = reference,
            saved = reference.scope;
        const scope: Scope = {
            cid: saved.conversationId,
            sid: saved.sessionId,
            connid: saved.connectionId,
        };
        if (
            !identifier(operationId) ||
            !identifier(command) ||
            [scope.cid, scope.sid, scope.connid].some(
                (v) => v !== undefined && !identifier(v),
            ) ||
            (scope.sid !== undefined && scope.cid === undefined) ||
            (scope.connid !== undefined && scope.sid === undefined)
        )
            throw new ConnectError(
                "invalid-request",
                "Complete original operation reference required",
            );
        const client = this.client,
            epoch = this.epoch;
        const frames = await client.request(
            "operation.get",
            { operation_id: operationId },
            scope,
            options,
        );
        this.observed(client, epoch);
        const records = frames.filter((f) => f.type === "operation.updated"),
            frame = records[0];
        if (
            records.length !== 1 ||
            !frame ||
            frame.cid !== scope.cid ||
            frame.sid !== scope.sid ||
            frame.connid !== scope.connid ||
            frame.payload.operation_id !== operationId ||
            frame.payload.command_type !== command ||
            typeof frame.payload.state !== "string" ||
            !operationStates.has(frame.payload.state) ||
            !Number.isSafeInteger(frame.payload.revision) ||
            Number(frame.payload.revision) < 0
        )
            return this.malformed(
                client,
                "Recovery did not identify the original scoped operation",
            );
        return Object.freeze({ ...frame.payload });
    }
    /** One bounded catch-up/snapshot page. Persist returned cursor/page metadata;
     * notifications and old physical leases are never replayed as authority. */
    async recoverConversation(
        point: BrowserConversationRecoveryPoint,
        options: RequestOptions = {},
    ): Promise<BrowserConversationRecoveryPage> {
        const { conversationId, cursor, page } = point,
            limit = point.limit ?? 32;
        if (
            !identifier(conversationId) ||
            !Number.isInteger(limit) ||
            limit < 1 ||
            limit > 63 ||
            (cursor !== undefined && !opaque(cursor)) ||
            (page !== undefined && !opaque(page)) ||
            (cursor !== undefined && page !== undefined)
        )
            throw new ConnectError(
                "invalid-request",
                "Bounded original Conversation cursor or snapshot page required",
            );
        const budget = new Budget(duration(options.timeoutMs, this.timeoutMs)),
            signal = options.signal;
        const client = this.client,
            epoch = this.epoch;
        const request = async (type: string, payload: JsonObject) => {
            const frames = await client.request(
                type,
                payload,
                { cid: conversationId },
                { timeoutMs: budget.remaining(), signal },
            );
            this.observed(client, epoch);
            return frames;
        };
        let frames: Envelope[];
        if (cursor !== undefined) {
            frames = await request("conversation.subscribe", {
                after: cursor,
                live: false,
                limit,
            });
            const summary = frames.find((f) => f.type === "ack"),
                details = summary?.payload.details;
            if (
                summary?.cid !== conversationId ||
                summary.sid !== undefined ||
                summary.connid !== undefined ||
                !jsonObject(details) ||
                typeof details.resync_required !== "boolean" ||
                typeof details.has_more !== "boolean" ||
                details.live !== false
            )
                return this.malformed(
                    client,
                    "Missing finite journal recovery summary",
                );
            if (!details.resync_required) {
                const events = frames.filter(
                    (f) => f.type === "conversation.event",
                );
                if (
                    !opaque(details.cursor) ||
                    events.length > limit ||
                    events.some(
                        (f) =>
                            f.cid !== conversationId ||
                            !opaque(f.payload.cursor),
                    )
                )
                    return this.malformed(
                        client,
                        "Journal recovery returned a foreign or unbounded cursor page",
                    );
                return Object.freeze({
                    kind: "events",
                    conversationId,
                    cursor: details.cursor,
                    hasMore: details.has_more,
                    frames: Object.freeze(events),
                });
            }
            if (!frames.some((f) => f.type === "conversation.snapshot"))
                frames = await request("conversation.get", { limit });
        } else {
            try {
                frames = await request("conversation.get", {
                    limit,
                    ...(page === undefined ? {} : { page }),
                });
            } catch (error) {
                if (
                    page === undefined ||
                    !(error instanceof ConnectError) ||
                    error.problem?.code !== 409 ||
                    error.problem.category !== "state" ||
                    error.problem.reason !== "resync_required"
                )
                    throw error;
                frames = await request("conversation.get", { limit });
            }
        }
        const snapshots = frames.filter(
                (f) => f.type === "conversation.snapshot",
            ),
            snapshot = snapshots[0],
            body = snapshot?.payload;
        if (
            snapshots.length !== 1 ||
            !snapshot ||
            snapshot.cid !== conversationId ||
            snapshot.sid !== undefined ||
            snapshot.connid !== undefined ||
            !body ||
            !opaque(body.cursor) ||
            !Number.isSafeInteger(body.revision) ||
            Number(body.revision) < 0 ||
            !Array.isArray(body.items) ||
            body.items.length > limit ||
            typeof body.has_more !== "boolean" ||
            (body.has_more
                ? !opaque(body.next_page)
                : body.next_page !== undefined)
        )
            return this.malformed(
                client,
                "Invalid current authorized Conversation snapshot page",
            );
        return Object.freeze({
            kind: "snapshot",
            conversationId,
            cursor: body.cursor,
            hasMore: body.has_more,
            ...(typeof body.next_page === "string"
                ? { nextPage: body.next_page }
                : {}),
            frames: Object.freeze([snapshot]),
        });
    }
}
