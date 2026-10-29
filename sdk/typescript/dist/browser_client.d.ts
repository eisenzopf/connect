import { ConnectError } from "./errors.js";
import { type JsonObject } from "./uctp_json.js";
import { type ServerTrust, type SigningIdentity } from "./uctp_signing.js";
import { type Envelope, type Scope } from "./uctp_wire.js";
export interface BrowserClientOptions {
    /** Explicit physical control transport; WebTransport is the default. No mutation is retried on transport loss. */
    transport?: "webtransport" | "websocket";
    url: string;
    credential: string;
    identityId: string;
    device: {
        id: string;
        kind: string;
        platform: string;
    };
    signing: SigningIdentity;
    server: ServerTrust;
    /** Explicit short-lived development WebTransport certificate pins; WSS uses platform TLS trust. */
    serverCertificateHashes?: WebTransportHash[];
    timeoutMs?: number;
    signal?: AbortSignal;
}
export interface RequestOptions {
    timeoutMs?: number;
    signal?: AbortSignal;
}
/** No application command is submitted by open. The phase distinguishes a
 * physical opening failure from an unresolved/refused authentication exchange. */
export declare class BrowserClientOpenError extends ConnectError {
    readonly phase: "transport" | "authentication";
    readonly cause: unknown;
    constructor(phase: "transport" | "authentication", cause: unknown);
}
export declare function browserTransportSupport(): {
    secureContext: boolean;
    webTransport: boolean;
    webSocket: boolean;
    webCrypto: boolean;
};
/** Canonical connect/2 client. A failed mutation retains its caller's logical ID. */
export declare class BrowserClient {
    private readonly options;
    private readonly transport;
    private readonly selectedTransport;
    private reader?;
    private writer?;
    private readonly pending;
    private readonly seen;
    private events;
    private eventBytes;
    private readonly listeners;
    private readonly closeListeners;
    private failure?;
    private writes;
    private queuedWrites;
    private auth?;
    private constructor();
    static open(options: BrowserClientOptions): Promise<BrowserClient>;
    /** Actual physical control selected at construction; caller option edits cannot change it. */
    get controlTransport(): "webtransport" | "websocket";
    get identityId(): string;
    get participantId(): string | undefined;
    /** Expiry of the signed authentication observation; not a current server grant. */
    get authenticationExpiresAt(): number | undefined;
    get closed(): boolean;
    /** Media owns these streams; one datagram reader/writer per physical peer. */
    get datagrams(): WebTransportDatagramDuplexStream;
    private timeout;
    private check;
    private bound;
    private pump;
    /** Notifications are hints. Use operation.get/snapshots for authoritative state. */
    onEvent(callback: (event: Envelope) => void): () => void;
    /** Owned capture, codecs and media routes retire with the physical control peer. */
    onClose(callback: (error: Error) => void): () => void;
    /** Advanced protocol escape hatch; caller supplies a stable operation_id for mutations. */
    request(type: string, payload: JsonObject, scope?: Scope, options?: RequestOptions & {
        terminalType?: string;
        validateCanonical?: boolean;
    }): Promise<Envelope[]>;
    /** Fresh request ID binds the server's native media parameter exchange. */
    begin(type: string, payload: JsonObject, scope?: Scope, options?: RequestOptions & {
        terminalType?: string;
        validateCanonical?: boolean;
    }): {
        id: string;
        response: Promise<Envelope[]>;
    };
    /** Signed one-way protocol response, such as confirming server media parameters. */
    send(type: string, payload: JsonObject, scope: Scope, options?: RequestOptions): Promise<string>;
    private write;
    private exchange;
    createConversation(operationId: string, options?: {
        tenantId?: string;
        participants?: string[];
        policy?: "persistent" | "ephemeral";
    }, request?: RequestOptions): Promise<{
        id: string;
        receipt: JsonObject;
    }>;
    operationStatus(conversationId: string, operationId: string, options?: RequestOptions): Promise<JsonObject>;
    keepAlive(options?: RequestOptions): Promise<void>;
    close(reason?: unknown): void;
}
