import { BrowserClient, type BrowserClientOptions, type RequestOptions } from "./browser_client.js";
import { type JsonObject } from "./uctp_json.js";
import { type Envelope } from "./uctp_wire.js";
/** The WSS label identifies a control/media path, never media readiness. */
export type BrowserTransportPath = "webtransport" | "wss-webrtc";
export type BrowserFallbackReason = "preferred-not-configured" | "preferred-unsupported" | "preferred-transport-timeout" | "preferred-transport-failed";
export type BrowserEnrollment = Omit<BrowserClientOptions, "transport" | "url" | "timeoutMs" | "signal">;
export interface BrowserTransportClientOptions {
    identityId: string;
    deviceId: string;
    endpoints: Partial<Record<BrowserTransportPath, string>>;
    preferredTransport?: BrowserTransportPath;
    allowFallback?: boolean;
    /** Resolve current enrollment separately for each usable physical contact.
     * A credential approved for WT cannot authorize a WSS fallback. Persist
     * original enrollment intent; resolution must never invent a replacement
     * mutation after uncertainty. All resolutions share the round deadline. */
    resolveEnrollment(context: {
        signal: AbortSignal;
        reason: "initial" | "reconnect";
        physicalEpoch: number;
        transport: BrowserTransportPath;
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
/** Owns physical selection/reconnect. It never queues or resubmits mutations.
 * A new physical epoch retires old capture/media owners; applications must
 * explicitly reconnect media or submit a distinct device-replacement intent. */
export declare class BrowserTransportClient {
    private readonly identityId;
    private readonly deviceId;
    private readonly endpoints;
    private readonly resolveEnrollment;
    private readonly allowFallback;
    private readonly timeoutMs;
    private readonly attemptTimeoutMs;
    private preferred;
    private current?;
    private stopWatching?;
    private round?;
    private phase;
    private selected?;
    private fallback?;
    private epoch;
    private constructor();
    static connect(options: BrowserTransportClientOptions, request?: RequestOptions): Promise<BrowserTransportClient>;
    get status(): BrowserTransportState;
    /** The actual authenticated physical client. It grants no media readiness. */
    get client(): BrowserClient;
    reconnect(options?: RequestOptions & {
        preferredTransport?: BrowserTransportPath;
    }): Promise<void>;
    private openRound;
    close(): void;
    private observed;
    private malformed;
    /** Status-first recovery only. It never dispatches the referenced command. */
    recoverOperation(reference: BrowserOperationReference, options?: RequestOptions): Promise<Readonly<JsonObject>>;
    /** One bounded catch-up/snapshot page. Persist returned cursor/page metadata;
     * notifications and old physical leases are never replayed as authority. */
    recoverConversation(point: BrowserConversationRecoveryPoint, options?: RequestOptions): Promise<BrowserConversationRecoveryPage>;
}
