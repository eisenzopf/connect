import { ConnectError } from "./errors.js";
import { jsonObject, type JsonObject } from "./uctp_json.js";
import {
    signEnvelope,
    validateSigningIdentity,
    verifyEnvelope,
    type ServerTrust,
    type SigningIdentity,
} from "./uctp_signing.js";
import {
    encodeEnvelope,
    envelope,
    EnvelopeReader,
    identifier,
    type Envelope,
    type Scope,
} from "./uctp_wire.js";

export interface BrowserClientOptions {
    url: string;
    credential: string;
    identityId: string;
    device: { id: string; kind: string; platform: string };
    signing: SigningIdentity;
    server: ServerTrust;
    /** Explicit short-lived development certificate pins; omit for platform TLS trust. */
    serverCertificateHashes?: WebTransportHash[];
    timeoutMs?: number;
    signal?: AbortSignal;
}
export interface RequestOptions {
    timeoutMs?: number;
    signal?: AbortSignal;
}
interface Pending {
    frames: Envelope[];
    bytes: number;
    finish: (frames: Envelope[]) => void;
    fail: (error: unknown) => void;
    terminal: (frame: Envelope) => boolean;
}

export function browserTransportSupport(): {
    secureContext: boolean;
    webTransport: boolean;
    webCrypto: boolean;
} {
    return {
        secureContext: globalThis.isSecureContext === true,
        webTransport: typeof WebTransport === "function",
        webCrypto: !!globalThis.crypto?.subtle,
    };
}

/** Canonical connect/2 client. A failed mutation retains its caller's logical ID. */
export class BrowserClient {
    private readonly transport: WebTransport;
    private reader?: ReadableStreamDefaultReader<Uint8Array>;
    private writer?: WritableStreamDefaultWriter<Uint8Array>;
    private readonly pending = new Map<string, Pending>();
    private readonly seen = new Map<string, number>();
    private events: Envelope[] = [];
    private eventBytes = 0;
    private readonly listeners = new Set<(event: Envelope) => void>();
    private readonly closeListeners = new Set<(error: Error) => void>();
    private failure?: Error;
    private writes: Promise<void> = Promise.resolve();
    private queuedWrites = 0;
    private auth?: Envelope;
    private constructor(private readonly options: BrowserClientOptions) {
        this.transport = new WebTransport(options.url, {
            allowPooling: false,
            serverCertificateHashes: options.serverCertificateHashes,
        });
        void this.transport.closed.then(
            () => this.close(),
            () =>
                this.close(
                    new ConnectError("transport", "WebTransport closed"),
                ),
        );
    }
    static async open(options: BrowserClientOptions): Promise<BrowserClient> {
        const support = browserTransportSupport();
        if (
            !support.secureContext ||
            !support.webTransport ||
            !support.webCrypto
        )
            throw new ConnectError(
                "unsupported-transport",
                "Secure WebTransport and WebCrypto are required",
            );
        let url: URL;
        try {
            url = new URL(options.url);
        } catch {
            throw new ConnectError(
                "invalid-request",
                "Invalid WebTransport URL",
            );
        }
        if (
            url.protocol !== "https:" ||
            url.username ||
            url.password ||
            url.search ||
            url.hash ||
            url.port === "0" ||
            /\s/.test(options.url) ||
            !identifier(options.identityId) ||
            !identifier(options.device.id) ||
            !options.device.kind ||
            !options.device.platform ||
            !options.credential ||
            options.credential.length > 16384
        )
            throw new ConnectError(
                "invalid-request",
                "Use an explicit HTTPS endpoint and enrolled participant credentials",
            );
        validateSigningIdentity(options.signing, "sign");
        validateSigningIdentity(options.server, "verify");
        const client = new BrowserClient(options);
        try {
            const until = performance.now() + client.timeout(options.timeoutMs);
            await client.bound(client.transport.ready, until, options.signal);
            const stream = await client.bound(
                client.transport.createBidirectionalStream(),
                until,
                options.signal,
            );
            client.reader = stream.readable.getReader();
            client.writer = stream.writable.getWriter();
            void client.pump().catch((error: unknown) => client.close(error));
            const hello = await client.exchange(
                envelope("auth.hello", {
                    device: {
                        ...options.device,
                        sdk_version: "connect-typescript/2",
                    },
                    auth_methods: ["bearer"],
                    application_profiles: ["connect/2"],
                }),
                false,
                (f) => f.type === "auth.challenge" || f.type === "error",
                {
                    timeoutMs: Math.max(1, until - performance.now()),
                    signal: options.signal,
                },
            );
            const challenge = hello.at(-1)!;
            const capabilities = challenge.payload.server_capabilities;
            if (
                challenge.type !== "auth.challenge" ||
                !Array.isArray(challenge.payload.accepted_methods) ||
                !challenge.payload.accepted_methods.includes("bearer") ||
                !jsonObject(capabilities) ||
                !Array.isArray(capabilities.application_profiles) ||
                !capabilities.application_profiles.includes("connect/2")
            )
                throw new ConnectError(
                    "authentication",
                    "Server did not offer connect/2 bearer authentication",
                );
            const frames = await client.exchange(
                envelope(
                    "auth.response",
                    { method: "bearer", credential: options.credential },
                    { in_reply_to: challenge.id },
                ),
                true,
                (f) => f.type === "auth.session" || f.type === "error",
                {
                    timeoutMs: Math.max(1, until - performance.now()),
                    signal: options.signal,
                },
            );
            const auth = frames.at(-1)!;
            if (
                auth.type !== "auth.session" ||
                auth.payload.application_profile !== "connect/2" ||
                auth.payload.identity_id !== options.identityId ||
                typeof auth.payload.session_token !== "string" ||
                typeof auth.payload.expires_at !== "string" ||
                !(Date.parse(auth.payload.expires_at) > Date.now())
            )
                throw new ConnectError(
                    "authentication",
                    "Server did not authenticate the enrolled participant",
                );
            client.auth = auth;
            return client;
        } catch (error) {
            client.close(error);
            throw error;
        }
    }
    get identityId(): string {
        return this.options.identityId;
    }
    get participantId(): string | undefined {
        return typeof this.auth?.payload.participant_id === "string"
            ? this.auth.payload.participant_id
            : undefined;
    }
    get closed(): boolean {
        return this.failure !== undefined;
    }
    /** Media owns these streams; one datagram reader/writer per physical peer. */
    get datagrams(): WebTransportDatagramDuplexStream {
        this.check();
        return this.transport.datagrams;
    }
    private timeout(value = 10000): number {
        if (!Number.isFinite(value) || value < 1 || value > 60000)
            throw new ConnectError(
                "invalid-request",
                "Deadline must be 1–60000 ms",
            );
        return value;
    }
    private check(): void {
        if (this.failure) throw this.failure;
    }
    private bound<T>(
        work: Promise<T>,
        until: number,
        signal?: AbortSignal,
    ): Promise<T> {
        return new Promise((resolve, reject) => {
            const abort = () => {
                cleanup();
                const e = new ConnectError(
                    "cancelled",
                    "Browser request cancelled; retain its logical operation ID",
                );
                this.close(e);
                reject(e);
            };
            const timer = setTimeout(
                () => {
                    cleanup();
                    const e = new ConnectError(
                        "timeout",
                        "Browser request deadline; retain its logical operation ID",
                    );
                    this.close(e);
                    reject(e);
                },
                Math.max(0, until - performance.now()),
            );
            const cleanup = () => {
                clearTimeout(timer);
                signal?.removeEventListener("abort", abort);
            };
            signal?.addEventListener("abort", abort, { once: true });
            if (signal?.aborted) abort();
            work.then(
                (v) => {
                    cleanup();
                    resolve(v);
                },
                (e: unknown) => {
                    cleanup();
                    reject(e);
                },
            );
        });
    }
    private async pump(): Promise<void> {
        const codec = new EnvelopeReader();
        for (;;) {
            const { value, done } = await this.reader!.read();
            if (done) {
                codec.end();
                throw new ConnectError("transport", "Canonical control EOF");
            }
            for (const frame of codec.push(value)) {
                await verifyEnvelope(frame, this.options.server);
                const now = Date.now();
                for (const [id, time] of this.seen)
                    if (time <= now - 60000) this.seen.delete(id);
                if (this.seen.has(frame.id) || this.seen.size >= 4096)
                    throw new ConnectError(
                        "protocol",
                        "Server replay or replay window exhausted",
                    );
                this.seen.set(frame.id, Date.parse(frame.ts));
                const p =
                    typeof frame.in_reply_to === "string"
                        ? this.pending.get(frame.in_reply_to)
                        : undefined;
                if (p) {
                    p.bytes += encodeEnvelope(frame).length;
                    if (p.frames.length >= 128 || p.bytes > 1048576)
                        throw new ConnectError(
                            "backpressure",
                            "Control reply bound exceeded",
                        );
                    p.frames.push(frame);
                    if (p.terminal(frame)) {
                        this.pending.delete(frame.in_reply_to as string);
                        p.finish(p.frames);
                    }
                } else if (this.listeners.size) {
                    for (const callback of this.listeners) callback(frame);
                } else {
                    const size = encodeEnvelope(frame).length;
                    if (
                        this.events.length >= 256 ||
                        this.eventBytes + size > 1048576
                    )
                        throw new ConnectError(
                            "backpressure",
                            "Control event queue exhausted",
                        );
                    this.events.push(frame);
                    this.eventBytes += size;
                }
            }
        }
    }
    /** Notifications are hints. Use operation.get/snapshots for authoritative state. */
    onEvent(callback: (event: Envelope) => void): () => void {
        this.check();
        if (this.listeners.size >= 16)
            throw new ConnectError("backpressure", "Too many event observers");
        this.listeners.add(callback);
        const retained = this.events;
        this.events = [];
        this.eventBytes = 0;
        try {
            for (const frame of retained) callback(frame);
        } catch (error) {
            this.listeners.delete(callback);
            this.close(error);
            throw error;
        }
        return () => {
            this.listeners.delete(callback);
        };
    }
    /** Owned capture, codecs and media routes retire with the physical control peer. */
    onClose(callback: (error: Error) => void): () => void {
        if (this.failure) {
            callback(this.failure);
            return () => {};
        }
        if (this.closeListeners.size >= 16)
            throw new ConnectError(
                "backpressure",
                "Too many lifetime observers",
            );
        this.closeListeners.add(callback);
        return () => {
            this.closeListeners.delete(callback);
        };
    }
    /** Advanced protocol escape hatch; caller supplies a stable operation_id for mutations. */
    request(
        type: string,
        payload: JsonObject,
        scope: Scope = {},
        options: RequestOptions & { terminalType?: string } = {},
    ): Promise<Envelope[]> {
        return this.begin(type, payload, scope, options).response;
    }
    /** Fresh request ID binds the server's native media parameter exchange. */
    begin(
        type: string,
        payload: JsonObject,
        scope: Scope = {},
        options: RequestOptions & { terminalType?: string } = {},
    ): { id: string; response: Promise<Envelope[]> } {
        const frame = envelope(type, payload, scope);
        const response = this.exchange(
            frame,
            true,
            (f) =>
                f.type === "ack" ||
                f.type === "error" ||
                f.type === options.terminalType,
            options,
        );
        return { id: frame.id, response };
    }
    /** Signed one-way protocol response, such as confirming server media parameters. */
    async send(
        type: string,
        payload: JsonObject,
        scope: Scope,
        options: RequestOptions = {},
    ): Promise<string> {
        this.check();
        const until = performance.now() + this.timeout(options.timeoutMs);
        if (options.signal?.aborted)
            throw new ConnectError(
                "cancelled",
                "Browser response cancelled before submission",
            );
        const frame = envelope(type, payload, scope);
        await this.bound(this.write(frame, true), until, options.signal);
        return frame.id;
    }
    private write(frame: Envelope, signed: boolean): Promise<void> {
        this.check();
        if (this.queuedWrites >= 16)
            throw new ConnectError(
                "backpressure",
                "Control write queue exhausted",
            );
        this.queuedWrites++;
        const write = this.writes
            .then(async () => {
                this.check();
                const signedFrame = signed
                    ? await signEnvelope(frame, this.options.signing)
                    : frame;
                this.check();
                await this.writer!.write(encodeEnvelope(signedFrame));
            })
            .finally(() => {
                this.queuedWrites--;
            });
        this.writes = write.catch((error: unknown) => this.close(error));
        return write;
    }
    private async exchange(
        frame: Envelope,
        signed: boolean,
        terminal: Pending["terminal"],
        options: RequestOptions,
    ): Promise<Envelope[]> {
        this.check();
        const until = performance.now() + this.timeout(options.timeoutMs);
        if (options.signal?.aborted)
            throw new ConnectError(
                "cancelled",
                "Browser request cancelled before submission",
            );
        if (this.pending.size >= 16)
            throw new ConnectError(
                "backpressure",
                "Too many outstanding requests",
            );
        let resolve!: Pending["finish"], reject!: Pending["fail"];
        const reply = new Promise<Envelope[]>((yes, no) => {
            resolve = yes;
            reject = no;
        });
        // Attach before asynchronous signing/writing: close can reject at any await.
        const result = this.bound(reply, until, options.signal);
        void result.catch(() => {});
        this.pending.set(frame.id, {
            frames: [],
            bytes: 0,
            finish: resolve,
            fail: reject,
            terminal,
        });
        try {
            const write = this.write(frame, signed);
            await this.bound(write, until, options.signal);
            const frames = await result;
            const error = frames.find((f) => f.type === "error");
            if (error)
                throw new ConnectError(
                    "refused",
                    frame.type +
                        ": " +
                        (typeof error.payload.reason === "string"
                            ? error.payload.reason
                            : typeof error.payload.message === "string"
                              ? error.payload.message
                              : "Canonical request refused"),
                );
            return frames;
        } catch (error) {
            reject(error);
            throw error;
        } finally {
            this.pending.delete(frame.id);
            void result.catch(() => {});
        }
    }
    async createConversation(
        operationId: string,
        options: {
            tenantId?: string;
            participants?: string[];
            policy?: "persistent" | "ephemeral";
        } = {},
        request: RequestOptions = {},
    ): Promise<{ id: string; receipt: JsonObject }> {
        if (
            !identifier(operationId) ||
            !identifier(options.tenantId ?? "tnt_default") ||
            (options.participants ?? []).some((id) => !identifier(id))
        )
            throw new ConnectError(
                "invalid-request",
                "Use stable operation and enrolled identity IDs",
            );
        const frames = await this.request(
            "conversation.create",
            {
                tenant_id: options.tenantId ?? "tnt_default",
                policy: options.policy ?? "persistent",
                idle_close_secs: null,
                metadata: null,
                initial_participants: (options.participants ?? []).map(
                    (identity_id) => ({ identity_id, role: "participant" }),
                ),
            },
            { operation_id: operationId },
            request,
        );
        const ack = frames.at(-1)!;
        const details = ack.payload.details;
        const conversation = jsonObject(details)
            ? details.conversation
            : undefined;
        const receipt = jsonObject(conversation)
            ? conversation.receipt
            : undefined;
        if (
            !jsonObject(details) ||
            details.reply_complete !== true ||
            !jsonObject(conversation) ||
            !identifier(conversation.id) ||
            !jsonObject(receipt) ||
            receipt.operation_id !== operationId ||
            receipt.conversation_id !== conversation.id ||
            receipt.state !== "completed"
        )
            throw new ConnectError(
                "protocol",
                "Missing durable conversation receipt",
            );
        return { id: conversation.id, receipt };
    }
    async operationStatus(
        conversationId: string,
        operationId: string,
        options: RequestOptions = {},
    ): Promise<JsonObject> {
        const frames = await this.request(
            "operation.get",
            { operation_id: operationId },
            { cid: conversationId },
            options,
        );
        const state = frames.find(
            (f) =>
                f.type === "operation.updated" &&
                f.payload.operation_id === operationId,
        )?.payload;
        if (!state)
            throw new ConnectError("protocol", "Missing operation status");
        return state;
    }
    async keepAlive(options: RequestOptions = {}): Promise<void> {
        await this.request(
            "auth.keepalive",
            { session_token: this.auth!.payload.session_token },
            {},
            options,
        );
    }
    close(
        reason: unknown = new ConnectError("closed", "Browser client closed"),
    ): void {
        if (this.failure) return;
        this.failure =
            reason instanceof Error
                ? reason
                : new ConnectError("transport", "Browser client failed");
        for (const p of this.pending.values()) p.fail(this.failure);
        this.pending.clear();
        this.events = [];
        this.eventBytes = 0;
        this.listeners.clear();
        void this.reader?.cancel().catch(() => {});
        void this.writer?.abort().catch(() => {});
        this.transport.close({ closeCode: 0, reason: "Connect client closed" });
        for (const callback of this.closeListeners) {
            try {
                callback(this.failure);
            } catch {
                /* Continue retiring every owner. */
            }
        }
        this.closeListeners.clear();
    }
}
