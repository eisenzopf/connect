import {
    subscribeBody,
    unsubscribeBody,
    updateBody,
    selectionResult,
    updateResult,
    type BrowserNonEmpty,
    type BrowserStreamSelector,
    type BrowserConnectionUpdate,
    type BrowserConnectionUpdateReceipt,
    type BrowserStreamSelectionReceipt,
} from "./browser_sdk_streams.js";
export type {
    BrowserNonEmpty,
    BrowserStreamSelector,
    BrowserConnectionUpdate,
    BrowserConnectionUpdateReceipt,
    BrowserStreamSelectionReceipt,
    BrowserCodecObservation,
    BrowserStreamGateObservation,
    BrowserMedium,
    BrowserDirection,
} from "./browser_sdk_streams.js";
import {
    BrowserRtcConnection,
    type BrowserRtcConfiguration,
} from "./browser_sdk_webrtc.js";
import { browserIceConfiguration } from "./browser_ice.js";
import {
    BrowserTransportClient,
    type BrowserTransportClientOptions,
    type BrowserOperationReference,
} from "./browser_transport_client.js";
import { BrowserClient, type RequestOptions } from "./browser_client.js";
import {
    CanonicalError,
    type CanonicalErrorKind,
    type Certainty,
} from "./canonical_error.js";
import {
    BrowserMediaPeer,
    type BrowserMediaConnection,
    type BrowserConnectionOptions,
} from "./browser_media.js";
import {
    BrowserMediaCapture,
    type BrowserMediaCaptureOptions,
} from "./browser_media_capture.js";
import { BrowserMediaRenderer } from "./browser_media_renderer.js";
import { ConnectError } from "./errors.js";
import {
    canonicalJson,
    parseCanonicalJson,
    jsonObject,
    type Json,
    type JsonObject,
} from "./uctp_json.js";
import { identifier, type Envelope, type Scope } from "./uctp_wire.js";
import type {
    SdkId,
    SdkRevision,
    ConversationId,
    IdentityId,
    ParticipantId,
    OperationId,
    SessionId,
    ConnectionId,
    StreamId,
} from "./sdk_identity.js";
export type {
    SdkId,
    SdkRevision,
    ConversationId,
    IdentityId,
    ParticipantId,
    OperationId,
    SessionId,
    ConnectionId,
    StreamId,
} from "./sdk_identity.js";

export class BrowserSdkError extends CanonicalError {
    constructor(
        kind: CanonicalErrorKind,
        certainty: Certainty,
        operation?: BrowserOperationReference,
        problem?: CanonicalError["problem"],
    ) {
        super(kind, certainty, operation, problem);
        this.name = "BrowserSdkError";
    }
}
function invalid(): never {
    throw new BrowserSdkError("validation", "not-submitted");
}
export function sdkId<K extends string>(value: string): SdkId<K> {
    if (!identifier(value)) invalid();
    return value as SdkId<K>;
}
export function sdkRevision<K extends string>(value: number): SdkRevision<K> {
    if (!Number.isSafeInteger(value) || value < 0) invalid();
    return value as SdkRevision<K>;
}
function object(v: unknown): v is JsonObject {
    return jsonObject(v as Json);
}
function freeze<T>(v: T): T {
    if (v && typeof v === "object") {
        for (const child of Object.values(v)) freeze(child);
        Object.freeze(v);
    }
    return v;
}
function copy<T>(v: T): T {
    return parseCanonicalJson(
        new TextEncoder().encode(canonicalJson(v as Json, 1048576)),
        1048576,
    ) as T;
}
function boundedText(v: unknown, max = 256, empty = false): v is string {
    return (
        typeof v === "string" &&
        (empty || v.length > 0) &&
        new TextEncoder().encode(v).length <= max &&
        !/[\u0000-\u001f\u007f-\u009f]/.test(v)
    );
}
export interface BrowserSdkRequestOptions {
    readonly timeoutMs: number;
    readonly signal?: AbortSignal;
}
export interface BrowserSdkMutationOptions extends BrowserSdkRequestOptions {
    readonly operationId: OperationId;
}
export interface BrowserSdkClientOptions extends BrowserTransportClientOptions {
    readonly tenantId: string;
    readonly media?: BrowserRtcConfiguration;
}
export type BrowserOperationState =
    | "accepted"
    | "preparing"
    | "ready"
    | "committing"
    | "completed"
    | "cancelled"
    | "failed"
    | "interrupted"
    | "unknown";
const states = new Set([
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
export interface BrowserOperationSnapshot {
    readonly key: BrowserOperationReference;
    readonly state: BrowserOperationState;
    readonly revision: number;
    readonly recordId?: string;
    readonly problem?: Json;
}
export interface BrowserCompletion<T> {
    readonly snapshot: BrowserOperationSnapshot;
    readonly value: T;
}
declare const result: unique symbol;
export interface BrowserSdkOperationReference<
    T,
> extends BrowserOperationReference {
    readonly operationId: OperationId;
    readonly scope: {
        readonly conversationId?: ConversationId;
        readonly sessionId?: SessionId;
        readonly connectionId?: ConnectionId;
    };
    readonly [result]: (value: T) => T;
}
export interface BrowserOperationKind<T> {
    readonly command: string;
    readonly [result]: (value: T) => T;
}
interface Kind<T> {
    command: string;
    scope: "root" | "session" | "connection";
    decode: (
        value: Json,
        client: BrowserSdkClient,
        key: BrowserOperationReference,
    ) => T;
}
const kinds = new WeakMap<object, Kind<unknown>>();
function kind<T>(
    command: string,
    scope: Kind<T>["scope"],
    decode: Kind<T>["decode"],
): BrowserOperationKind<T> {
    const k = Object.freeze({ command }) as BrowserOperationKind<T>;
    kinds.set(k, { command, scope, decode } as Kind<unknown>);
    return k;
}
export interface BrowserParticipantSnapshot {
    readonly id: ParticipantId;
    readonly conversationId: ConversationId;
    readonly identityId: IdentityId;
    readonly kind: string;
    readonly role: string;
    readonly membershipEpoch: number;
    readonly active: boolean;
    readonly removalReason: string | null;
}
export interface BrowserConversationSnapshot {
    readonly id: ConversationId;
    readonly revision: SdkRevision<"conversation">;
    readonly cursor: string;
    readonly closed: boolean;
    readonly participants: readonly BrowserParticipantSnapshot[];
}
export interface BrowserSessionSnapshot {
    readonly id: SessionId;
    readonly conversationId: ConversationId;
    readonly state: string;
    readonly media: readonly string[];
    readonly intent: string | null;
    readonly capabilitiesOffer: Json;
    readonly codecPreferences: readonly string[];
    readonly isMain: boolean;
    readonly revision: SdkRevision<"session">;
    readonly topologyRevision: SdkRevision<"topology">;
    readonly createdAt: number;
    readonly updatedAt: number;
    readonly endedReason: string | null;
}
export interface BrowserSessionDecision {
    readonly session: BrowserSession;
    readonly state: "rejected" | "cancelled";
    readonly topologyRevision: SdkRevision<"topology">;
}
function numeric(v: unknown): v is number {
    return Number.isSafeInteger(v) && Number(v) >= 0;
}
function sessionView(
    v: Json,
    cid: string,
    malformed: () => never,
): BrowserSessionSnapshot {
    if (
        !object(v) ||
        !identifier(v.id) ||
        v.conversation_id !== cid ||
        !boundedText(v.state, 64) ||
        !Array.isArray(v.media) ||
        v.media.length > 4 ||
        v.media.some(
            (m) =>
                typeof m !== "string" ||
                !["audio", "video", "screen", "data"].includes(m),
        ) ||
        new Set(v.media).size !== v.media.length ||
        !(v.intent === null || boundedText(v.intent)) ||
        !(v.capabilities_offer === null || object(v.capabilities_offer)) ||
        !Array.isArray(v.codec_preferences) ||
        v.codec_preferences.length > 32 ||
        v.codec_preferences.some((c) => !boundedText(c, 128)) ||
        typeof v.is_main !== "boolean" ||
        !numeric(v.revision) ||
        !numeric(v.topology_revision) ||
        !numeric(v.created_at) ||
        !numeric(v.updated_at) ||
        !(v.ended_reason === null || boundedText(v.ended_reason, 256, true))
    )
        return malformed();
    return freeze({
        id: v.id as SessionId,
        conversationId: cid as ConversationId,
        state: v.state,
        media: v.media as string[],
        intent: v.intent as string | null,
        capabilitiesOffer: v.capabilities_offer,
        codecPreferences: v.codec_preferences as string[],
        isMain: v.is_main,
        revision: v.revision as SdkRevision<"session">,
        topologyRevision: v.topology_revision as SdkRevision<"topology">,
        createdAt: v.created_at,
        updatedAt: v.updated_at,
        endedReason: v.ended_reason as string | null,
    });
}
function sessionResult(
    value: Json,
    client: BrowserSdkClient,
    key: BrowserOperationReference,
): BrowserSession {
    const bad = () => current(client).malformed(key, "recorded");
    if (
        !object(value) ||
        value.kind !== "session_control" ||
        !object(value.data)
    )
        return bad();
    const d = value.data;
    if (
        d.operation_id !== key.operationId ||
        d.action !== key.command ||
        !object(d.session)
    )
        return bad();
    const view = sessionView(d.session, key.scope.conversationId!, bad);
    if (view.id !== key.scope.sessionId) return bad();
    return new BrowserSession(client, view.conversationId, view.id, view);
}
function decisionResult(
    value: Json,
    client: BrowserSdkClient,
    key: BrowserOperationReference,
): BrowserSessionDecision {
    const bad = () => current(client).malformed(key, "recorded");
    if (
        !object(value) ||
        value.kind !== "session_decision" ||
        !object(value.data)
    )
        return bad();
    const d = value.data;
    if (
        d.operation_id !== key.operationId ||
        d.conversation_id !== key.scope.conversationId ||
        d.session_id !== key.scope.sessionId ||
        d.state !==
            (key.command === "session.reject" ? "rejected" : "cancelled") ||
        !numeric(d.revision) ||
        !boundedText(d.reason, 256, true) ||
        !Number.isInteger(d.reason_code) ||
        Number(d.reason_code) < 0 ||
        Number(d.reason_code) > 65535
    )
        return bad();
    return freeze({
        session: client.conversations
            .byId(d.conversation_id as ConversationId)
            .sessions.byId(d.session_id as SessionId),
        state: d.state as "rejected" | "cancelled",
        topologyRevision: d.revision as SdkRevision<"topology">,
    });
}
function connectionResult(
    value: Json,
    client: BrowserSdkClient,
    key: BrowserOperationReference,
): BrowserConnection {
    const bad = () => current(client).malformed(key, "recorded");
    if (!object(value)) return bad();
    let revision: SdkRevision<"stream"> | undefined;
    if (key.command === "connection.offer") {
        if (
            !numeric(value.stream_revision) ||
            value.stream_revision < 1 ||
            !Array.isArray(value.streams) ||
            !value.streams.length ||
            value.streams.length > 32 ||
            value.streams.some((v) => !identifier(v)) ||
            new Set(value.streams).size !== value.streams.length
        )
            return bad();
        revision = value.stream_revision as SdkRevision<"stream">;
        const s = current(client),
            local = s.local.get(key.scope.connectionId!);
        if (
            local &&
            local.offerOperation === key.operationId &&
            local.cid === key.scope.conversationId &&
            local.sid === key.scope.sessionId &&
            local.epoch === s.owner.status.physicalEpoch
        )
            local.revision = Math.max(local.revision ?? 0, revision);
    } else {
        if (
            value.ended !== true ||
            value.connection_id !== key.scope.connectionId
        )
            return bad();
        current(client).mediaPeer?.retireLocalConnection(
            key.scope.connectionId!,
        );
        current(client).local.get(key.scope.connectionId!)?.rtc?.close();
        current(client).local.delete(key.scope.connectionId!);
    }
    return new BrowserConnection(
        client,
        key.scope.conversationId as ConversationId,
        key.scope.sessionId as SessionId,
        key.scope.connectionId as ConnectionId,
        revision,
    );
}
function controlConnection(
    value: Json,
    client: BrowserSdkClient,
    key: BrowserOperationReference,
): BrowserConnection {
    if (
        !object(value) ||
        !numeric(value.stream_revision) ||
        value.stream_revision < 1
    )
        return current(client).malformed(key, "recorded");
    return new BrowserConnection(
        client,
        key.scope.conversationId as ConversationId,
        key.scope.sessionId as SessionId,
        key.scope.connectionId as ConnectionId,
        value.stream_revision as SdkRevision<"stream">,
    );
}
function observedControl<
    T extends BrowserConnectionUpdateReceipt | BrowserStreamSelectionReceipt,
>(value: T, client: BrowserSdkClient, key: BrowserOperationReference): T {
    const s = current(client),
        local = s.local.get(key.scope.connectionId!);
    if (
        local &&
        local.cid === key.scope.conversationId &&
        local.sid === key.scope.sessionId &&
        local.epoch === s.owner.status.physicalEpoch &&
        local.generation === value.connectionGeneration
    )
        local.revision = Math.max(local.revision ?? 0, value.streamRevision);
    return value;
}
export const browserOperationKinds = Object.freeze({
    connectionUpdate: kind("connection.update", "connection", (v, c, k) =>
        observedControl(
            updateResult(v, k, controlConnection(v, c, k), () =>
                current(c).malformed(k, "recorded"),
            ),
            c,
            k,
        ),
    ),
    streamSubscribe: kind("stream.subscribe", "connection", (v, c, k) =>
        observedControl(
            selectionResult(v, k, controlConnection(v, c, k), () =>
                current(c).malformed(k, "recorded"),
            ),
            c,
            k,
        ),
    ),
    streamUnsubscribe: kind("stream.unsubscribe", "connection", (v, c, k) =>
        observedControl(
            selectionResult(v, k, controlConnection(v, c, k), () =>
                current(c).malformed(k, "recorded"),
            ),
            c,
            k,
        ),
    ),
    connectionOffer: kind("connection.offer", "connection", connectionResult),
    connectionEnd: kind("connection.end", "connection", connectionResult),
    conversationCreate: kind("create", "root", (v, c, k) => {
        if (
            !object(v) ||
            v.kind !== "conversation" ||
            !object(v.data) ||
            !identifier(v.data.id) ||
            !object(v.data.receipt) ||
            v.data.receipt.operation_id !== k.operationId
        )
            return current(c).malformed(k, "recorded");
        return c.conversations.byId(v.data.id as ConversationId);
    }),
    sessionInvite: kind("session.invite", "session", sessionResult),
    sessionAccept: kind("session.accept", "session", sessionResult),
    sessionEnd: kind("session.end", "session", sessionResult),
    sessionReject: kind("session.reject", "session", decisionResult),
    sessionCancelInvitation: kind("session.cancel", "session", decisionResult),
});
function descriptor<T>(k: BrowserOperationKind<T>): Kind<T> {
    const d = kinds.get(k);
    if (!d) invalid();
    return d as Kind<T>;
}
function reference<T>(
    saved: Json,
    k: BrowserOperationKind<T>,
): BrowserOperationReference {
    const d = descriptor(k);
    if (
        !object(saved) ||
        Object.keys(saved).sort().join() !==
            "command,operation_id,schema,scope" ||
        saved.schema !== "connect-operation-reference/1" ||
        saved.command !== d.command ||
        !identifier(saved.operation_id) ||
        !object(saved.scope) ||
        Object.keys(saved.scope).some(
            (v) =>
                !["conversation_id", "session_id", "connection_id"].includes(v),
        )
    )
        invalid();
    const s = saved.scope;
    if (
        d.scope === "root"
            ? Object.keys(s).length !== 0
            : !identifier(s.conversation_id) ||
              !identifier(s.session_id) ||
              (d.scope === "connection"
                  ? !identifier(s.connection_id)
                  : s.connection_id !== undefined)
    )
        invalid();
    return freeze({
        operationId: saved.operation_id,
        command: d.command,
        scope: {
            ...(s.conversation_id === undefined
                ? {}
                : { conversationId: s.conversation_id as string }),
            ...(s.session_id === undefined
                ? {}
                : { sessionId: s.session_id as string }),
            ...(s.connection_id === undefined
                ? {}
                : { connectionId: s.connection_id as string }),
        },
    });
}
function referenceJson(key: BrowserOperationReference): JsonObject {
    if (!key || !object(key.scope)) invalid();
    return {
        schema: "connect-operation-reference/1",
        operation_id: key.operationId,
        command: key.command,
        scope: {
            ...(key.scope.conversationId === undefined
                ? {}
                : { conversation_id: key.scope.conversationId }),
            ...(key.scope.sessionId === undefined
                ? {}
                : { session_id: key.scope.sessionId }),
            ...(key.scope.connectionId === undefined
                ? {}
                : { connection_id: key.scope.connectionId }),
        },
    };
}
class Budget {
    readonly until: number;
    readonly signal?: AbortSignal;
    constructor(options: BrowserSdkRequestOptions) {
        if (
            !options ||
            !Number.isFinite(options.timeoutMs) ||
            options.timeoutMs < 1 ||
            options.timeoutMs > 60000
        )
            invalid();
        this.until = performance.now() + options.timeoutMs;
        this.signal = options.signal;
        this.options();
    }
    options(): RequestOptions {
        if (this.signal?.aborted)
            throw new BrowserSdkError("cancelled", "read-only");
        const left = this.until - performance.now();
        if (left < 1) throw new BrowserSdkError("deadline", "read-only");
        return { timeoutMs: left, signal: this.signal };
    }
    async delay(): Promise<void> {
        const wait = Math.min(50, this.options().timeoutMs!);
        await new Promise<void>((resolve, reject) => {
            const done = () => {
                    clearTimeout(timer);
                    this.signal?.removeEventListener("abort", abort);
                    resolve();
                },
                abort = () => {
                    clearTimeout(timer);
                    this.signal?.removeEventListener("abort", abort);
                    reject(new BrowserSdkError("cancelled", "read-only"));
                };
            const timer = setTimeout(done, wait);
            this.signal?.addEventListener("abort", abort, { once: true });
            if (this.signal?.aborted) abort();
        });
    }
}
function translated(
    error: unknown,
    certainty: Certainty,
    key?: BrowserOperationReference,
): BrowserSdkError {
    if (error instanceof BrowserSdkError) {
        if (key && !error.operation)
            return new BrowserSdkError(
                error.kind,
                error.certainty === "read-only" ? certainty : error.certainty,
                key,
                error.problem,
            );
        return error;
    }
    if (error instanceof ConnectError) {
        if (error.code === "media-denied")
            return new BrowserSdkError("application", certainty, key, {
                code: 403,
                category: "capture",
                reason: "permission_denied",
            });
        const kinds: Record<string, CanonicalErrorKind> = {
            timeout: "deadline",
            cancelled: "cancelled",
            protocol: "protocol",
            authentication: "authentication",
            forbidden: "authentication",
            unauthorized: "authentication",
            "invalid-request": "validation",
            configuration: "configuration",
            trust: "trust",
            conflict: "conflict",
            unsupported: "unsupported",
            "unsupported-feature": "unsupported",
            "unsupported-media-profile": "unsupported",
            media: "application",
            "media-failed": "application",
            "media-output-failed": "application",
            "media-input-failed": "application",
            "resync-required": "resync-required",
        };
        const kind: CanonicalErrorKind =
            error.code === "refused"
                ? error.problem?.code === 401 || error.problem?.code === 403
                    ? "authentication"
                    : "application"
                : (kinds[error.code] ?? "transport-uncertain");
        return new BrowserSdkError(
            kind,
            ["refused", "forbidden", "unauthorized"].includes(error.code)
                ? "refused"
                : certainty,
            key,
            error.problem,
        );
    }
    return new BrowserSdkError("protocol", certainty, key);
}
interface Snapshot {
    conversation: BrowserConversationSnapshot;
    sessions: Map<string, BrowserSessionSnapshot>;
    appearances: Map<string, JsonObject>;
    connections: Map<string, BrowserConnectionSnapshot>;
    streams: Map<string, BrowserStreamSnapshot>;
}
interface Hint {
    frame: Envelope;
    epoch: number;
}
class State {
    mediaPeer?: BrowserMediaPeer;
    local = new Map<
        string,
        {
            cid: ConversationId;
            sid: SessionId;
            epoch: number;
            ready: Promise<BrowserMediaConnection | BrowserRtcConnection>;
            rtc?: BrowserRtcConnection;
            control?: Promise<void>;
            revision?: number;
            offerOperation: string;
            generation: number;
            audio?: string;
            video?: string;
            screen?: string;
        }
    >();
    renderers = new Map<string, BrowserMediaRenderer | BrowserRenderHandle>();
    pending = new Map<string, object>();
    captures = new Map<string, object>();
    nativePeer(): BrowserMediaPeer {
        if (this.peer.controlTransport !== "webtransport")
            throw new BrowserSdkError("unsupported", "not-submitted");
        return (this.mediaPeer ??= new BrowserMediaPeer(this.peer, {
            audio: (binding, frame) => {
                const r = this.renderers.get(binding.connectionId);
                if (r instanceof BrowserMediaRenderer) r.audio(binding, frame);
            },
            video: (binding, frame) => {
                const r = this.renderers.get(binding.connectionId);
                if (r instanceof BrowserMediaRenderer) r.video(binding, frame);
            },
        }));
    }
    hints: Hint[] = [];
    hintBytes = 0;
    wake?: () => void;
    reader = false;
    stopEvents?: () => void;
    stopClose?: () => void;
    constructor(
        readonly owner: BrowserTransportClient,
        readonly tenant: string,
        readonly actor: string,
        readonly rtcConfiguration: BrowserRtcConfiguration = {},
    ) {}
    get peer(): BrowserClient {
        try {
            return this.owner.client;
        } catch (e) {
            throw translated(e, "read-only");
        }
    }
    malformed(
        key?: BrowserOperationReference,
        certainty: Certainty = "read-only",
    ): never {
        this.peer.close(
            new ConnectError("protocol", "Invalid typed browser response"),
        );
        throw new BrowserSdkError("protocol", certainty, key);
    }
    observe(peer: BrowserClient, epoch: number) {
        if (
            peer.closed ||
            this.owner.status.physicalEpoch !== epoch ||
            this.peer !== peer
        )
            throw new BrowserSdkError("cancelled", "read-only");
    }
    watch() {
        this.mediaPeer?.close();
        this.mediaPeer = undefined;
        for (const local of this.local.values()) local.rtc?.close();
        this.local.clear();
        this.pending.clear();
        for (const renderer of this.renderers.values()) renderer.close();
        this.renderers.clear();
        this.captures.clear();
        this.stopEvents?.();
        this.stopClose?.();
        this.hints = [];
        this.hintBytes = 0;
        const peer = this.peer,
            epoch = this.owner.status.physicalEpoch;
        this.stopClose = peer.onClose(() => {
            this.hints = [];
            this.hintBytes = 0;
            this.wake?.();
        });
        this.stopEvents = peer.onEvent((frame) => {
            if (frame.type !== "session.invite") return;
            const bytes = new TextEncoder().encode(canonicalJson(frame)).length;
            if (this.hints.length >= 64 || this.hintBytes + bytes > 1048576)
                this.malformed();
            this.hints.push({ frame, epoch });
            this.hintBytes += bytes;
            this.wake?.();
        });
    }
    async request(
        type: string,
        payload: JsonObject,
        scope: Scope,
        options: RequestOptions,
        key?: BrowserOperationReference,
    ): Promise<Envelope[]> {
        const peer = this.peer,
            epoch = this.owner.status.physicalEpoch;
        try {
            const frames = await peer.request(type, payload, scope, options);
            this.observe(peer, epoch);
            return frames;
        } catch (e) {
            throw translated(e, key ? "uncertain" : "read-only", key);
        }
    }
    async snapshot(
        cid: ConversationId,
        options: BrowserSdkRequestOptions,
    ): Promise<Snapshot> {
        const budget = new Budget(options),
            peer = this.peer,
            epoch = this.owner.status.physicalEpoch;
        let page: string | undefined,
            cursor: string | undefined,
            revision: number | undefined,
            bytes = 0;
        const facts: JsonObject[] = [],
            pages = new Set<string>();
        for (let i = 0; i < 256; i++) {
            let batch;
            try {
                batch = await this.owner.recoverConversation(
                    { conversationId: cid, page, limit: 63 },
                    budget.options(),
                );
            } catch (e) {
                throw translated(e, "read-only");
            }
            this.observe(peer, epoch);
            if (batch.kind !== "snapshot" || batch.frames.length !== 1)
                this.malformed();
            const body = batch.frames[0].payload;
            if (
                revision !== undefined &&
                (revision !== body.revision || cursor !== batch.cursor)
            )
                throw new BrowserSdkError("resync-required", "read-only");
            revision = Number(body.revision);
            cursor = batch.cursor;
            for (const item of body.items as Json[]) {
                if (
                    !object(item) ||
                    typeof item.kind !== "string" ||
                    !object(item.data)
                )
                    this.malformed();
                bytes += new TextEncoder().encode(canonicalJson(item)).length;
                if (bytes > 2 * 1048576)
                    throw new BrowserSdkError("resync-required", "read-only");
                facts.push(item);
            }
            if (!batch.hasMore) break;
            if (i === 255 || !batch.nextPage || pages.has(batch.nextPage))
                this.malformed();
            page = batch.nextPage;
            pages.add(page);
        }
        const participants: BrowserParticipantSnapshot[] = [],
            sessions = new Map<string, BrowserSessionSnapshot>(),
            appearances = new Map<string, JsonObject>(),
            connections = new Map<string, BrowserConnectionSnapshot>(),
            streams = new Map<string, BrowserStreamSnapshot>();
        let conversation: JsonObject | undefined;
        const memberIds = new Set();
        for (const item of facts) {
            const d = item.data as JsonObject;
            if (item.kind === "conversation") {
                if (
                    conversation ||
                    d.conversation_id !== cid ||
                    d.revision !== revision ||
                    typeof d.closed !== "boolean"
                )
                    this.malformed();
                conversation = d;
            } else if (item.kind === "participant") {
                if (
                    d.conversation_id !== cid ||
                    !identifier(d.participant_id) ||
                    !identifier(d.identity_id) ||
                    memberIds.has(d.participant_id) ||
                    !["human", "ai", "system", "external"].includes(
                        String(d.kind),
                    ) ||
                    !boundedText(d.role, 128) ||
                    !numeric(d.membership_epoch) ||
                    d.membership_epoch < 1 ||
                    typeof d.active !== "boolean" ||
                    !(
                        d.removal_reason === null ||
                        boundedText(d.removal_reason)
                    )
                )
                    this.malformed();
                memberIds.add(d.participant_id);
                participants.push(
                    freeze({
                        id: d.participant_id as ParticipantId,
                        conversationId: cid,
                        identityId: d.identity_id as IdentityId,
                        kind: d.kind as string,
                        role: d.role,
                        membershipEpoch: d.membership_epoch,
                        active: d.active,
                        removalReason: d.removal_reason as string | null,
                    }),
                );
            } else if (item.kind === "session") {
                const s = sessionView(d, cid, () => this.malformed());
                if (sessions.has(s.id)) this.malformed();
                sessions.set(s.id, s);
            } else if (item.kind === "connection") {
                const c = connectionView(d, cid, () => this.malformed());
                if (connections.has(c.id)) this.malformed();
                connections.set(c.id, c);
            } else if (item.kind === "stream") {
                const v = streamView(d, cid, () => this.malformed());
                const key = JSON.stringify([v.sessionId, v.connectionId, v.id]);
                if (streams.has(key)) this.malformed();
                streams.set(key, v);
            } else if (item.kind === "session_participant") {
                if (
                    !identifier(d.session_id) ||
                    !identifier(d.participant_id) ||
                    !numeric(d.membership_epoch) ||
                    !boundedText(d.role, 128) ||
                    ![
                        "invited",
                        "joining",
                        "present",
                        "left",
                        "removed",
                    ].includes(String(d.state))
                )
                    this.malformed();
                const id = d.session_id + ":" + d.participant_id;
                if (appearances.has(id)) this.malformed();
                appearances.set(id, freeze(copy(d)));
            }
        }
        if (!conversation || revision === undefined || cursor === undefined)
            this.malformed();
        budget.options();
        this.observe(peer, epoch);
        return {
            conversation: freeze({
                id: cid,
                revision: revision as SdkRevision<"conversation">,
                cursor,
                closed: conversation.closed as boolean,
                participants,
            }),
            sessions,
            appearances,
            connections,
            streams,
        };
    }
}
const clients = new WeakMap<BrowserSdkClient, State>();
function current(client: BrowserSdkClient): State {
    const s = clients.get(client);
    if (!s) invalid();
    return s;
}
export class BrowserOperation<T> {
    readonly reference: BrowserSdkOperationReference<T>;
    #record?: string;
    #revision = -1;
    /** @internal Instances are supplied by SDK-owned mutation kinds. */
    constructor(
        private readonly client: BrowserSdkClient,
        ref: BrowserOperationReference,
        private readonly kind: BrowserOperationKind<T>,
    ) {
        this.reference = reference(
            referenceJson(ref),
            kind,
        ) as BrowserSdkOperationReference<T>;
        Object.freeze(this);
    }
    referenceJson(): Json {
        return copy(referenceJson(this.reference));
    }
    private async observation(
        options: BrowserSdkRequestOptions,
    ): Promise<{ snapshot: BrowserOperationSnapshot; result?: Json }> {
        const s = current(this.client);
        let value;
        try {
            value = await s.owner.recoverOperation(
                this.reference,
                new Budget(options).options(),
            );
        } catch (e) {
            throw translated(e, "read-only", this.reference);
        }
        if (
            typeof value.record_id !== "string" ||
            !identifier(value.record_id) ||
            (this.#record !== undefined && value.record_id !== this.#record) ||
            Number(value.revision) < this.#revision
        )
            s.malformed(this.reference);
        this.#record = value.record_id as string;
        this.#revision = Number(value.revision);
        return {
            snapshot: freeze({
                key: this.reference,
                state: value.state as BrowserOperationState,
                revision: Number(value.revision),
                recordId: value.record_id as string,
                ...(value.problem === undefined
                    ? {}
                    : { problem: copy(value.problem) }),
            }),
            ...(value.result === undefined
                ? {}
                : { result: copy(value.result) }),
        };
    }
    async status(
        options: BrowserSdkRequestOptions,
    ): Promise<BrowserOperationSnapshot> {
        return (await this.observation(options)).snapshot;
    }
    async wait(
        options: BrowserSdkRequestOptions,
    ): Promise<BrowserCompletion<T>> {
        const budget = new Budget(options);
        for (;;) {
            const o = await this.observation(
                budget.options() as BrowserSdkRequestOptions,
            );
            if (o.snapshot.state === "completed") {
                if (o.result === undefined)
                    throw new BrowserSdkError(
                        "receipt-unavailable",
                        "recorded",
                        this.reference,
                    );
                const value = descriptor(this.kind).decode(
                    o.result,
                    this.client,
                    this.reference,
                );
                budget.options();
                return Object.freeze({ snapshot: o.snapshot, value });
            }
            if (["failed", "cancelled"].includes(o.snapshot.state))
                throw new BrowserSdkError(
                    "application",
                    "recorded",
                    this.reference,
                );
            await budget.delay();
        }
    }
    async waitUntilReady(
        options: BrowserSdkRequestOptions,
    ): Promise<BrowserOperationSnapshot> {
        const budget = new Budget(options);
        for (;;) {
            const s = await this.status(
                budget.options() as BrowserSdkRequestOptions,
            );
            if (["ready", "committing", "completed"].includes(s.state))
                return s;
            if (["failed", "cancelled"].includes(s.state))
                throw new BrowserSdkError(
                    "application",
                    "recorded",
                    this.reference,
                );
            await budget.delay();
        }
    }
}
const submit = Symbol("submit");
export class BrowserSdkClient {
    readonly role = "browser" as const;
    readonly operationKinds = browserOperationKinds;
    readonly conversations = {
        byId: (id: ConversationId) => {
            sdkId(id);
            return new BrowserConversation(this, id);
        },
        create: (
            input: {
                policy: "persistent" | "ephemeral";
                participants: readonly {
                    identityId: IdentityId;
                    role: string;
                }[];
                idleCloseSeconds?: number;
                metadata?: Json;
            },
            options: BrowserSdkMutationOptions,
        ) => {
            if (
                !input ||
                !["persistent", "ephemeral"].includes(input.policy) ||
                !Array.isArray(input.participants) ||
                input.participants.length > 128
            )
                invalid();
            const identities = new Set();
            const participants = input.participants.map((p) => {
                sdkId(p.identityId);
                if (identities.has(p.identityId) || !boundedText(p.role, 64))
                    invalid();
                identities.add(p.identityId);
                return { identity_id: p.identityId, role: p.role };
            });
            const idle =
                input.idleCloseSeconds ??
                (input.policy === "ephemeral" ? 60 : undefined);
            if (
                idle !== undefined &&
                (!Number.isInteger(idle) || idle < 1 || idle > 604800)
            )
                invalid();
            return this[submit](
                browserOperationKinds.conversationCreate,
                {
                    tenant_id: current(this).tenant,
                    policy: input.policy,
                    initial_participants: participants,
                    ...(idle === undefined ? {} : { idle_close_secs: idle }),
                    ...(input.metadata === undefined
                        ? {}
                        : { metadata: input.metadata }),
                },
                {},
                options,
            );
        },
    };
    readonly operations = {
        restore: <T>(saved: Json, k: BrowserOperationKind<T>) =>
            new BrowserOperation(this, reference(copy(saved), k), k),
        recover: <T>(ref: BrowserSdkOperationReference<T>) => {
            if (!ref) invalid();
            const k = Object.values(browserOperationKinds).find(
                (k) => k.command === ref.command,
            ) as BrowserOperationKind<T> | undefined;
            if (!k) invalid();
            return new BrowserOperation(this, ref, k);
        },
    };
    private constructor(s: State) {
        clients.set(this, s);
        s.watch();
    }
    static async connect(
        options: BrowserSdkClientOptions,
    ): Promise<BrowserSdkClient> {
        sdkId(options.tenantId);
        const tenant = options.tenantId,
            actor = options.identityId;
        let media: BrowserRtcConfiguration;
        try {
            media =
                browserIceConfiguration(
                    options.media?.iceServers,
                    options.media?.iceTransportPolicy,
                ) ?? {};
        } catch (error) {
            throw translated(error, "not-submitted");
        }
        try {
            return new BrowserSdkClient(
                new State(
                    await BrowserTransportClient.connect(options),
                    tenant,
                    actor,
                    media,
                ),
            );
        } catch (error) {
            throw translated(error, "not-submitted");
        }
    }

    get transport() {
        return current(this).owner.status;
    }
    async reconnect(
        options: Parameters<BrowserTransportClient["reconnect"]>[0] = {},
    ): Promise<void> {
        const s = current(this);
        try {
            await s.owner.reconnect(options);
            s.watch();
        } catch (error) {
            throw translated(error, "not-submitted");
        }
    }
    close(): void {
        const s = current(this);
        s.owner.close();
        s.stopEvents?.();
        s.stopClose?.();
        s.hints = [];
        s.hintBytes = 0;
        s.wake?.();
    }
    async [submit]<T>(
        k: BrowserOperationKind<T>,
        body: JsonObject,
        scope: Scope,
        options: BrowserSdkMutationOptions,
    ): Promise<BrowserOperation<T>> {
        const d = descriptor(k);
        if (!options || !identifier(options.operationId)) invalid();
        const key = reference(
            referenceJson({
                operationId: options.operationId,
                command: d.command,
                scope: {
                    ...(scope.cid === undefined
                        ? {}
                        : { conversationId: scope.cid }),
                    ...(scope.sid === undefined
                        ? {}
                        : { sessionId: scope.sid }),
                    ...(scope.connid === undefined
                        ? {}
                        : { connectionId: scope.connid }),
                },
            }),
            k,
        );
        const budget = new Budget(options);
        const captured = copy(body);
        let control: ReturnType<BrowserMediaPeer["beginControl"]> | undefined;
        try {
            const s = current(this),
                peer = s.peer,
                epoch = s.owner.status.physicalEpoch;
            const local =
                scope.connid === undefined
                    ? undefined
                    : s.local.get(scope.connid);
            let frames: Envelope[];
            if (
                peer.controlTransport === "webtransport" &&
                local &&
                [
                    "connection.update",
                    "stream.subscribe",
                    "stream.unsubscribe",
                ].includes(d.command)
            ) {
                if (
                    local.cid !== scope.cid ||
                    local.sid !== scope.sid ||
                    local.epoch !== epoch
                )
                    throw new BrowserSdkError(
                        "receipt-unavailable",
                        "not-submitted",
                        key,
                    );
                await boundedLocal(
                    local.ready,
                    budget.options() as BrowserSdkRequestOptions,
                );
                s.observe(peer, epoch);
                control = s
                    .nativePeer()
                    .beginControl(
                        scope.connid!,
                        key.operationId,
                        d.command as
                            | "connection.update"
                            | "stream.subscribe"
                            | "stream.unsubscribe",
                        captured,
                        scope.expected_revision!,
                        budget.options(),
                    );
                const pending = Promise.allSettled([
                    local.control ?? Promise.resolve(),
                    control.ready,
                ]).then(() => {});
                local.control = pending;
                void pending
                    .finally(() => {
                        if (local.control === pending)
                            local.control = undefined;
                    })
                    .catch(() => {});
                frames = await control.response;
                s.observe(peer, epoch);
            } else
                frames = await s.request(
                    d.command === "create" ? "conversation.create" : d.command,
                    captured,
                    { ...scope, operation_id: key.operationId },
                    budget.options(),
                    key,
                );
            const updates = frames.filter(
                (f) => f.type === "operation.updated",
            );
            if (d.scope !== "root") {
                if (
                    updates.length !== 1 ||
                    updates[0].cid !== scope.cid ||
                    updates[0].sid !== scope.sid ||
                    updates[0].connid !== scope.connid ||
                    updates[0].payload.operation_id !== key.operationId ||
                    !states.has(String(updates[0].payload.state)) ||
                    !numeric(updates[0].payload.revision)
                )
                    current(this).malformed(key, "uncertain");
            } else {
                const ack = frames.find((f) => f.type === "ack"),
                    details = ack?.payload.details;
                if (
                    !object(details) ||
                    !object(details.conversation) ||
                    !identifier(details.conversation.id) ||
                    !object(details.conversation.receipt) ||
                    details.conversation.receipt.operation_id !==
                        key.operationId
                )
                    current(this).malformed(key, "uncertain");
            }
            budget.options();
            return new BrowserOperation(this, key, k);
        } catch (error) {
            throw translated(
                error,
                control && !control.submissionAttempted
                    ? "not-submitted"
                    : "uncertain",
                key,
            );
        }
    }
    incomingInvitations(
        options: BrowserSdkRequestOptions,
    ): BrowserIncomingInvitations {
        return incoming(this, options);
    }
}
export class BrowserParticipant {
    constructor(
        private readonly client: BrowserSdkClient,
        readonly conversationId: ConversationId,
        readonly id: ParticipantId,
    ) {
        Object.freeze(this);
    }
    async refresh(
        options: BrowserSdkRequestOptions,
    ): Promise<BrowserParticipantSnapshot> {
        const snapshot = await current(this.client).snapshot(
                this.conversationId,
                options,
            ),
            p = snapshot.conversation.participants.find(
                (p) => p.id === this.id,
            );
        if (!p) throw new BrowserSdkError("receipt-unavailable", "read-only");
        return p;
    }
}
export class BrowserConversation {
    readonly participants = {
        byId: (id: ParticipantId) => {
            sdkId(id);
            return new BrowserParticipant(this.client, this.id, id);
        },
        byIdentity: async (
            id: IdentityId,
            options: BrowserSdkRequestOptions,
        ) => {
            sdkId(id);
            const s = await this.refresh(options),
                p = s.participants.find((p) => p.identityId === id && p.active);
            if (!p)
                throw new BrowserSdkError("receipt-unavailable", "read-only");
            return new BrowserParticipant(this.client, this.id, p.id);
        },
    };
    readonly sessions = {
        byId: (id: SessionId) => {
            sdkId(id);
            return new BrowserSession(this.client, this.id, id);
        },
        invite: (
            input: {
                sessionId: SessionId;
                to: readonly ParticipantId[];
                medium: string;
                intent: string;
                capabilities: Json;
            },
            options: BrowserSdkMutationOptions,
        ) => {
            if (
                !input ||
                !identifier(input.sessionId) ||
                !Array.isArray(input.to) ||
                input.to.length > 128 ||
                input.to.some((p) => !identifier(p)) ||
                new Set(input.to).size !== input.to.length ||
                ![
                    "voice",
                    "video",
                    "voice+video",
                    "screen-share",
                    "text-chat",
                    "mixed",
                ].includes(input.medium) ||
                !boundedText(input.intent) ||
                !object(input.capabilities) ||
                new TextEncoder().encode(canonicalJson(input.capabilities))
                    .length > 16384
            )
                invalid();
            return this.client[submit](
                browserOperationKinds.sessionInvite,
                {
                    to: [...input.to],
                    medium: input.medium,
                    intent: input.intent,
                    capabilities_offer: input.capabilities,
                },
                { cid: this.id, sid: input.sessionId },
                options,
            );
        },
    };
    constructor(
        private readonly client: BrowserSdkClient,
        readonly id: ConversationId,
    ) {
        Object.freeze(this);
    }
    async refresh(
        options: BrowserSdkRequestOptions,
    ): Promise<BrowserConversationSnapshot> {
        return (await current(this.client).snapshot(this.id, options))
            .conversation;
    }
}
export class BrowserSession {
    readonly connections = {
        byId: (id: ConnectionId) => {
            sdkId(id);
            return new BrowserConnection(
                this.client,
                this.conversationId,
                this.id,
                id,
            );
        },
        offer: async (
            input: {
                connectionId: ConnectionId;
                streams: readonly [
                    BrowserOfferedStream,
                    ...BrowserOfferedStream[],
                ];
            },
            options: BrowserSdkMutationOptions,
        ): Promise<BrowserOperation<BrowserConnection>> => {
            if (
                !input ||
                !identifier(input.connectionId) ||
                !Array.isArray(input.streams) ||
                !input.streams.length ||
                input.streams.length > 8 ||
                input.streams.some((v) => !v || typeof v !== "object") ||
                new Set(input.streams.map((v) => v.id)).size !==
                    input.streams.length ||
                !options ||
                !identifier(options.operationId)
            )
                invalid();
            const low: BrowserConnectionOptions = {
                conversationId: this.conversationId,
                sessionId: this.id,
                connectionId: input.connectionId,
                operationId: options.operationId,
            };
            const seen = new Set<string>();
            for (const stream of input.streams) {
                if (
                    !stream ||
                    !identifier(stream.id) ||
                    !["audio", "video", "screen", "data"].includes(
                        stream.kind,
                    ) ||
                    !["sendrecv", "sendonly", "recvonly", "inactive"].includes(
                        stream.direction,
                    ) ||
                    !Array.isArray(stream.codecs) ||
                    !stream.codecs.length
                )
                    invalid();
                // The installed native browser backend currently owns these exact
                // fixed codecs/directions. Unsupported choices never downgrade.
                if (
                    stream.direction !== "sendrecv" ||
                    stream.codecs.length !== 1 ||
                    (stream.kind === "audio"
                        ? stream.codecs[0] !== "opus"
                        : stream.kind === "data"
                          ? !["text", "json", "binary"].includes(
                                stream.codecs[0],
                            )
                          : stream.codecs[0] !== "vp8") ||
                    (stream.kind !== "data" && seen.has(stream.kind))
                )
                    throw new BrowserSdkError("unsupported", "not-submitted");
                seen.add(stream.kind);
                if (stream.kind === "data")
                    (low.data ??= []).push({
                        id: stream.id,
                        protocol: stream.codecs[0] as
                            "text" | "json" | "binary",
                    });
                else
                    low[stream.kind as "audio" | "video" | "screen"] =
                        stream.id;
            }
            const connectionId = low.connectionId;
            const budget = new Budget(options),
                s = current(this.client),
                peer = s.peer,
                epoch = s.owner.status.physicalEpoch;
            const key = reference(
                referenceJson({
                    operationId: options.operationId,
                    command: "connection.offer",
                    scope: {
                        conversationId: this.conversationId,
                        sessionId: this.id,
                        connectionId: input.connectionId,
                    },
                }),
                browserOperationKinds.connectionOffer,
            );
            if (
                s.local.has(connectionId) ||
                s.pending.has(connectionId) ||
                s.local.size + s.pending.size >= 8
            )
                throw new BrowserSdkError("conflict", "not-submitted", key);
            const reservation = {};
            s.pending.set(connectionId, reservation);
            let begun:
                | {
                      accepted: Promise<void>;
                      ready: Promise<
                          BrowserMediaConnection | BrowserRtcConnection
                      >;
                      readonly submissionAttempted: boolean;
                      owner?: BrowserRtcConnection;
                  }
                | undefined;
            try {
                if (peer.controlTransport === "websocket") {
                    if (
                        !low.audio ||
                        low.data?.length ||
                        (low.screen && low.video)
                    )
                        throw new BrowserSdkError(
                            "unsupported",
                            "not-submitted",
                            key,
                        );
                    const snapshot = await s.snapshot(
                        this.conversationId,
                        budget.options() as BrowserSdkRequestOptions,
                    );
                    s.observe(peer, epoch);
                    const participant = snapshot.conversation.participants.find(
                        (v) => v.identityId === s.actor && v.active,
                    );
                    const presence = participant
                        ? snapshot.appearances.get(
                              this.id + ":" + participant.id,
                          )
                        : undefined;
                    if (
                        !participant ||
                        !presence ||
                        presence.state !== "present" ||
                        presence.membership_epoch !==
                            participant.membershipEpoch
                    )
                        throw new BrowserSdkError(
                            "application",
                            "not-submitted",
                            key,
                        );
                    begun = BrowserRtcConnection.begin(
                        peer,
                        low,
                        participant.id,
                        s.rtcConfiguration,
                        budget.options(),
                    );
                } else
                    begun = s
                        .nativePeer()
                        .beginOfferConnection(low, budget.options());
                const local = {
                    cid: this.conversationId,
                    sid: this.id,
                    epoch,
                    ready: begun.ready,
                    rtc: begun.owner,
                    offerOperation: key.operationId,
                    generation: 1,
                    audio: low.audio,
                    video: low.video,
                    screen: low.screen,
                };
                s.local.set(connectionId, local);
                begun.owner?.onClose(() => {
                    if (s.local.get(connectionId) === local)
                        s.local.delete(connectionId);
                });
                void begun.ready.catch(() => {
                    if (s.local.get(connectionId) === local)
                        s.local.delete(connectionId);
                });
                await begun.accepted;
                s.observe(peer, epoch);
                budget.options();
                return new BrowserOperation(
                    this.client,
                    key,
                    browserOperationKinds.connectionOffer,
                );
            } catch (error) {
                throw translated(
                    error,
                    begun?.submissionAttempted ? "uncertain" : "not-submitted",
                    key,
                );
            } finally {
                if (s.pending.get(connectionId) === reservation)
                    s.pending.delete(connectionId);
            }
        },
    };

    constructor(
        private readonly client: BrowserSdkClient,
        readonly conversationId: ConversationId,
        readonly id: SessionId,
        readonly receiptSnapshot?: BrowserSessionSnapshot,
    ) {
        Object.freeze(this);
    }
    async refresh(
        options: BrowserSdkRequestOptions,
    ): Promise<BrowserSessionSnapshot> {
        const snapshot = await current(this.client).snapshot(
                this.conversationId,
                options,
            ),
            s = snapshot.sessions.get(this.id);
        if (!s) throw new BrowserSdkError("receipt-unavailable", "read-only");
        return s;
    }
    accept(
        input: {
            capabilities: Json;
            expectedTopologyRevision: SdkRevision<"topology">;
        },
        options: BrowserSdkMutationOptions,
    ): Promise<BrowserOperation<BrowserSession>> {
        if (
            !input ||
            !object(input.capabilities) ||
            new TextEncoder().encode(canonicalJson(input.capabilities)).length >
                16384
        )
            invalid();
        return this.client[submit](
            browserOperationKinds.sessionAccept,
            { capabilities_answer: input.capabilities },
            {
                cid: this.conversationId,
                sid: this.id,
                expected_revision: sdkRevision(input.expectedTopologyRevision),
            },
            options,
        );
    }
    private decide<T>(
        k: BrowserOperationKind<T>,
        input: {
            reasonCode: number;
            reason: string;
            expectedTopologyRevision: SdkRevision<"topology">;
        },
        options: BrowserSdkMutationOptions,
    ): Promise<BrowserOperation<T>> {
        if (
            !input ||
            !Number.isInteger(input.reasonCode) ||
            input.reasonCode < 0 ||
            input.reasonCode > 65535 ||
            !boundedText(input.reason, 256, true)
        )
            invalid();
        return this.client[submit](
            k,
            { reason_code: input.reasonCode, reason: input.reason },
            {
                cid: this.conversationId,
                sid: this.id,
                expected_revision: sdkRevision(input.expectedTopologyRevision),
            },
            options,
        );
    }
    reject(
        input: {
            reasonCode: number;
            reason: string;
            expectedTopologyRevision: SdkRevision<"topology">;
        },
        options: BrowserSdkMutationOptions,
    ) {
        return this.decide(browserOperationKinds.sessionReject, input, options);
    }
    cancelInvitation(
        input: {
            reasonCode: number;
            reason: string;
            expectedTopologyRevision: SdkRevision<"topology">;
        },
        options: BrowserSdkMutationOptions,
    ) {
        return this.decide(
            browserOperationKinds.sessionCancelInvitation,
            input,
            options,
        );
    }
    end(
        input: {
            reasonCode: number;
            reason: string;
            expectedTopologyRevision: SdkRevision<"topology">;
        },
        options: BrowserSdkMutationOptions,
    ) {
        return this.decide(browserOperationKinds.sessionEnd, input, options);
    }
}
export interface BrowserIncomingInvitation {
    readonly session: BrowserSession;
    readonly from: ParticipantId;
    readonly to: ParticipantId;
    /** Stable server invitation source, distinct from the caller mutation ID. */
    readonly operationId: OperationId;
    readonly source: {
        readonly kind: "session.invite";
        readonly correlationId: OperationId;
    };
    readonly medium: string;
    readonly intent: string;
    readonly capabilities: Json;
}
export interface BrowserIncomingInvitations extends AsyncIterableIterator<BrowserIncomingInvitation> {
    next(): Promise<IteratorResult<BrowserIncomingInvitation, undefined>>;
    close(): Promise<void>;
}
function incoming(
    client: BrowserSdkClient,
    options: BrowserSdkRequestOptions,
): BrowserIncomingInvitations {
    new Budget(options);
    const s = current(client);
    if (s.reader) throw new BrowserSdkError("conflict", "read-only");
    s.reader = true;
    const captured = { timeoutMs: options.timeoutMs, signal: options.signal },
        stop = new AbortController();
    let closed = false,
        busy = false;
    const reader: BrowserIncomingInvitations = {
        [Symbol.asyncIterator]() {
            return reader;
        },
        async next() {
            if (closed) return { done: true, value: undefined };
            if (busy) throw new BrowserSdkError("conflict", "read-only");
            const combined = AbortSignal.any([
                    stop.signal,
                    ...(captured.signal ? [captured.signal] : []),
                ]),
                budget = new Budget({ ...captured, signal: combined });
            busy = true;
            try {
                for (;;) {
                    budget.options();
                    const peer = s.peer,
                        epoch = s.owner.status.physicalEpoch;
                    const queued = s.hints.shift();
                    if (!queued) {
                        await new Promise<void>((resolve, reject) => {
                            let timer: ReturnType<typeof setTimeout>;
                            const finish = () => {
                                    clearTimeout(timer);
                                    combined.removeEventListener(
                                        "abort",
                                        cancel,
                                    );
                                    if (s.wake === finish) s.wake = undefined;
                                    resolve();
                                },
                                cancel = () => {
                                    clearTimeout(timer);
                                    combined.removeEventListener(
                                        "abort",
                                        cancel,
                                    );
                                    if (s.wake === finish) s.wake = undefined;
                                    reject(
                                        new BrowserSdkError(
                                            "cancelled",
                                            "read-only",
                                        ),
                                    );
                                };
                            s.wake = finish;
                            timer = setTimeout(
                                finish,
                                budget.options().timeoutMs,
                            );
                            combined.addEventListener("abort", cancel, {
                                once: true,
                            });
                            if (combined.aborted) cancel();
                        });
                        continue;
                    }
                    s.hintBytes -= new TextEncoder().encode(
                        canonicalJson(queued.frame),
                    ).length;
                    if (queued.epoch !== epoch) continue;
                    const f = queued.frame,
                        p = f.payload;
                    if (
                        f.connid !== undefined ||
                        f.in_reply_to !== undefined ||
                        f.expected_revision !== undefined ||
                        !identifier(f.cid) ||
                        !identifier(f.sid) ||
                        !identifier(f.operation_id) ||
                        !identifier(p.from) ||
                        !Array.isArray(p.to) ||
                        p.to.length !== 1 ||
                        !identifier(p.to[0]) ||
                        p.from === p.to[0] ||
                        ![
                            "voice",
                            "video",
                            "voice+video",
                            "screen-share",
                            "text-chat",
                            "mixed",
                        ].includes(String(p.medium)) ||
                        !boundedText(p.intent) ||
                        !object(p.capabilities_offer)
                    )
                        s.malformed();
                    const to = (p.to as string[])[0];
                    const snapshot = await s.snapshot(
                        f.cid as ConversationId,
                        budget.options() as BrowserSdkRequestOptions,
                    );
                    s.observe(peer, epoch);
                    const target = snapshot.conversation.participants.find(
                            (v) =>
                                v.id === to &&
                                v.identityId === s.actor &&
                                v.active,
                        ),
                        view = snapshot.sessions.get(f.sid as string),
                        appearance = snapshot.appearances.get(f.sid + ":" + to);
                    if (
                        snapshot.conversation.closed ||
                        !target ||
                        !view ||
                        ["ended", "rejected", "cancelled", "failed"].includes(
                            view.state,
                        ) ||
                        !appearance ||
                        appearance.membership_epoch !==
                            target.membershipEpoch ||
                        appearance.state !== "invited"
                    )
                        continue;
                    return {
                        done: false,
                        value: freeze({
                            session: new BrowserSession(
                                client,
                                f.cid as ConversationId,
                                f.sid as SessionId,
                            ),
                            from: p.from as ParticipantId,
                            to: to as ParticipantId,
                            operationId: f.operation_id as OperationId,
                            source: {
                                kind: "session.invite" as const,
                                correlationId: f.operation_id as OperationId,
                            },
                            medium: p.medium as string,
                            intent: p.intent as string,
                            capabilities: copy(p.capabilities_offer),
                        }),
                    };
                }
            } catch (e) {
                if (closed) return { done: true, value: undefined };
                throw translated(e, "read-only");
            } finally {
                busy = false;
            }
        },
        async close() {
            if (closed) return;
            closed = true;
            s.reader = false;
            stop.abort();
            s.wake?.();
        },
        async return() {
            await reader.close();
            return { done: true, value: undefined };
        },
    };
    return reader;
}

export interface BrowserCaptureHandle {
    readonly closed: boolean;
    pause(): void;
    resume(): Promise<void>;
    close(): void;
}
export interface BrowserRenderHandle {
    readonly finished: Promise<void>;
    readonly diagnostics: Readonly<{
        closed: boolean;
        paused: boolean;
        audio: number;
        video: number;
    }>;
    pause(): void;
    resume(): Promise<void>;
    close(): void;
}
export interface BrowserOfferedStream {
    readonly id: StreamId;
    readonly kind: "audio" | "video" | "screen" | "data";
    readonly direction: "sendonly" | "recvonly" | "sendrecv" | "inactive";
    readonly codecs: readonly [string, ...string[]];
}
export interface BrowserConnectionSnapshot {
    /** Current physical's last committed negotiated revision, when locally known. */
    readonly streamRevision?: SdkRevision<"stream">;
    readonly id: ConnectionId;
    readonly conversationId: ConversationId;
    readonly sessionId: SessionId;
    readonly participantId: ParticipantId;
    readonly generation: number;
    /** Store resource revision, independent of negotiated stream revision. */
    readonly resourceRevision: SdkRevision<"connection-resource">;
    readonly state: string;
}
export interface BrowserStreamSnapshot {
    readonly id: StreamId;
    readonly conversationId: ConversationId;
    readonly sessionId: SessionId;
    readonly connectionId: ConnectionId | null;
    readonly participantId: ParticipantId;
    readonly medium: "audio" | "video" | "screen" | "data";
    readonly state: string;
    readonly resourceRevision: SdkRevision<"stream-resource">;
    /** A current snapshot is an observation, never a local capture grant. */
    readonly ownership: "observation";
}
function connectionView(
    d: JsonObject,
    cid: ConversationId,
    bad: () => never,
): BrowserConnectionSnapshot {
    if (
        !identifier(d.connection_id) ||
        !identifier(d.session_id) ||
        !identifier(d.participant_id) ||
        !identifier(d.endpoint_id) ||
        !identifier(d.device_id) ||
        !numeric(d.membership_epoch) ||
        d.membership_epoch < 1 ||
        !numeric(d.generation) ||
        d.generation < 1 ||
        !numeric(d.revision) ||
        !boundedText(d.state, 64) ||
        !(d.replaces === null || identifier(d.replaces))
    )
        return bad();
    return freeze({
        id: d.connection_id as ConnectionId,
        conversationId: cid,
        sessionId: d.session_id as SessionId,
        participantId: d.participant_id as ParticipantId,
        generation: d.generation,
        resourceRevision: d.revision as SdkRevision<"connection-resource">,
        state: d.state,
    });
}
function streamView(
    d: JsonObject,
    cid: ConversationId,
    bad: () => never,
): BrowserStreamSnapshot {
    if (
        !identifier(d.stream_id) ||
        !identifier(d.session_id) ||
        !identifier(d.participant_id) ||
        !(d.connection_id === null || identifier(d.connection_id)) ||
        !["audio", "video", "screen", "data"].includes(String(d.medium)) ||
        !boundedText(d.state, 64) ||
        !numeric(d.revision)
    )
        return bad();
    return freeze({
        id: d.stream_id as StreamId,
        conversationId: cid,
        sessionId: d.session_id as SessionId,
        connectionId: d.connection_id as ConnectionId | null,
        participantId: d.participant_id as ParticipantId,
        medium: d.medium as BrowserStreamSnapshot["medium"],
        state: d.state,
        resourceRevision: d.revision as SdkRevision<"stream-resource">,
        ownership: "observation" as const,
    });
}
async function boundedLocal<T>(
    work: Promise<T>,
    options: BrowserSdkRequestOptions,
): Promise<T> {
    const budget = new Budget(options);
    return new Promise<T>((resolve, reject) => {
        let settled = false;
        const finish = (error: unknown, value?: T) => {
            if (settled) return;
            settled = true;
            clearTimeout(timer);
            options.signal?.removeEventListener("abort", abort);
            error ? reject(error) : resolve(value!);
        };
        const abort = () =>
            finish(new BrowserSdkError("cancelled", "read-only"));
        const timer = setTimeout(
            () => finish(new BrowserSdkError("deadline", "read-only")),
            budget.options().timeoutMs,
        );
        options.signal?.addEventListener("abort", abort, { once: true });
        if (options.signal?.aborted) abort();
        work.then(
            (value) => finish(undefined, value),
            (error) => finish(translated(error, "read-only")),
        );
    });
}
export class BrowserConnection {
    readonly streams = {
        subscribe: (
            input: {
                readonly selectors: BrowserNonEmpty<BrowserStreamSelector>;
                readonly expectedStreamRevision: SdkRevision<"stream">;
            },
            options: BrowserSdkMutationOptions,
        ): Promise<BrowserOperation<BrowserStreamSelectionReceipt>> => {
            if (!input) invalid();
            return this.client[submit](
                browserOperationKinds.streamSubscribe,
                subscribeBody(input.selectors, invalid),
                {
                    cid: this.conversationId,
                    sid: this.sessionId,
                    connid: this.id,
                    expected_revision: sdkRevision(
                        input.expectedStreamRevision,
                    ),
                },
                options,
            );
        },
        unsubscribe: (
            input: {
                readonly streams: BrowserNonEmpty<StreamId>;
                readonly expectedStreamRevision: SdkRevision<"stream">;
            },
            options: BrowserSdkMutationOptions,
        ): Promise<BrowserOperation<BrowserStreamSelectionReceipt>> => {
            if (!input) invalid();
            return this.client[submit](
                browserOperationKinds.streamUnsubscribe,
                unsubscribeBody(input.streams, invalid),
                {
                    cid: this.conversationId,
                    sid: this.sessionId,
                    connid: this.id,
                    expected_revision: sdkRevision(
                        input.expectedStreamRevision,
                    ),
                },
                options,
            );
        },
        byId: (id: StreamId) => {
            sdkId(id);
            return new BrowserStream(
                this.client,
                this.conversationId,
                this.sessionId,
                this.id,
                id,
            );
        },
        list: async (
            options: BrowserSdkRequestOptions,
        ): Promise<readonly BrowserStream[]> => {
            const snapshot = await current(this.client).snapshot(
                this.conversationId,
                options,
            );
            const connection = snapshot.connections.get(this.id);
            if (!connection || connection.sessionId !== this.sessionId)
                throw new BrowserSdkError("receipt-unavailable", "read-only");
            return Object.freeze(
                [...snapshot.streams.values()]
                    .filter(
                        (v) =>
                            v.connectionId === this.id &&
                            v.sessionId === this.sessionId,
                    )
                    .map((v) => this.streams.byId(v.id)),
            );
        },
    };
    constructor(
        private readonly client: BrowserSdkClient,
        readonly conversationId: ConversationId,
        readonly sessionId: SessionId,
        readonly id: ConnectionId,
        readonly receiptStreamRevision?: SdkRevision<"stream">,
    ) {
        Object.freeze(this);
    }
    async refresh(
        options: BrowserSdkRequestOptions,
    ): Promise<BrowserConnectionSnapshot> {
        const c = (
            await current(this.client).snapshot(this.conversationId, options)
        ).connections.get(this.id);
        if (!c || c.sessionId !== this.sessionId)
            throw new BrowserSdkError("receipt-unavailable", "read-only");
        const s = current(this.client),
            local = s.local.get(this.id);
        if (
            local &&
            local.cid === this.conversationId &&
            local.sid === this.sessionId &&
            local.epoch === s.owner.status.physicalEpoch &&
            local.generation === c.generation
        ) {
            let revision = local.revision ?? 0;
            if (s.mediaPeer)
                try {
                    revision = Math.max(
                        revision,
                        s.mediaPeer.revision(this.id),
                    );
                } catch {
                    /* Retired or not installed locally. */
                }
            if (revision > 0)
                return freeze({
                    ...c,
                    streamRevision: revision as SdkRevision<"stream">,
                });
        }
        return c;
    }
    update(
        input: BrowserConnectionUpdate,
        expectedStreamRevision: SdkRevision<"stream">,
        options: BrowserSdkMutationOptions,
    ): Promise<BrowserOperation<BrowserConnectionUpdateReceipt>> {
        return this.client[submit](
            browserOperationKinds.connectionUpdate,
            updateBody(input, invalid),
            {
                cid: this.conversationId,
                sid: this.sessionId,
                connid: this.id,
                expected_revision: sdkRevision(expectedStreamRevision),
            },
            options,
        );
    }
    end(
        input: {
            reasonCode: number;
            reason: string;
            expectedStreamRevision?: SdkRevision<"stream">;
        },
        options: BrowserSdkMutationOptions,
    ): Promise<BrowserOperation<BrowserConnection>> {
        if (
            !input ||
            !Number.isInteger(input.reasonCode) ||
            input.reasonCode < 0 ||
            input.reasonCode > 65535 ||
            !boundedText(input.reason, 256, true)
        )
            invalid();
        return this.client[submit](
            browserOperationKinds.connectionEnd,
            { reason_code: input.reasonCode, reason: input.reason },
            {
                cid: this.conversationId,
                sid: this.sessionId,
                connid: this.id,
                ...(input.expectedStreamRevision === undefined
                    ? {}
                    : {
                          expected_revision: sdkRevision(
                              input.expectedStreamRevision,
                          ),
                      }),
            },
            options,
        );
    }
    private async native(options: BrowserSdkRequestOptions) {
        const s = current(this.client),
            peer = s.peer,
            epoch = s.owner.status.physicalEpoch,
            local = s.local.get(this.id);
        if (
            !local ||
            local.cid !== this.conversationId ||
            local.sid !== this.sessionId ||
            local.epoch !== epoch
        )
            throw new BrowserSdkError("receipt-unavailable", "read-only");
        const budget = new Budget(options);
        const native = await boundedLocal(
            local.ready,
            budget.options() as BrowserSdkRequestOptions,
        );
        while (local.control)
            await boundedLocal(
                local.control,
                budget.options() as BrowserSdkRequestOptions,
            );
        s.observe(peer, epoch);
        if (s.local.get(this.id) !== local)
            throw new BrowserSdkError("receipt-unavailable", "read-only");
        return { native, local, s, peer, epoch };
    }
    /** Local capture is explicit and owns only this physical's negotiated senders. */
    async capture(
        input: {
            audio?: boolean;
            video?: "camera" | "screen";
            audioDeviceId?: string;
            videoDeviceId?: string;
            stream?: MediaStream;
            state?: BrowserMediaCaptureOptions["state"];
        },
        options: BrowserSdkRequestOptions,
    ): Promise<BrowserCaptureHandle> {
        if (
            !input ||
            (input.audio !== undefined && typeof input.audio !== "boolean") ||
            (input.video !== undefined &&
                !["camera", "screen"].includes(input.video)) ||
            (!input.audio && !input.video)
        )
            invalid();
        const captured = { ...input },
            budget = new Budget(options),
            { native, local, s, peer, epoch } = await this.native(
                budget.options() as BrowserSdkRequestOptions,
            );
        const audio = captured.audio ? local.audio : undefined,
            video =
                captured.video === undefined
                    ? undefined
                    : captured.video === "screen"
                      ? local.screen
                      : local.video;
        if ((captured.audio && !audio) || (captured.video && !video)) invalid();
        if (s.captures.has(this.id))
            throw new BrowserSdkError("conflict", "read-only");
        const reservation = {};
        s.captures.set(this.id, reservation);
        const release = () => {
            if (s.captures.get(this.id) === reservation)
                s.captures.delete(this.id);
        };
        let capture: BrowserMediaCapture | BrowserCaptureHandle | undefined;
        try {
            const state: BrowserMediaCaptureOptions["state"] = (
                state,
                error,
            ) => {
                if (state === "closed") release();
                captured.state?.(
                    state,
                    error ? translated(error, "read-only") : undefined,
                );
            };
            capture =
                native instanceof BrowserRtcConnection
                    ? await native.capture(
                          { ...captured, state },
                          budget.options(),
                      )
                    : await BrowserMediaCapture.open(native, {
                          audioStreamId: audio,
                          videoStreamId: video,
                          videoSource: captured.video,
                          audioDeviceId: captured.audioDeviceId,
                          videoDeviceId: captured.videoDeviceId,
                          stream: captured.stream,
                          state,
                          ...budget.options(),
                      });
            s.observe(peer, epoch);
            const owner = capture;
            return Object.freeze({
                get closed() {
                    return owner.closed;
                },
                pause: () => owner.pause(),
                resume: async () => {
                    try {
                        await owner.resume();
                    } catch (error) {
                        throw translated(error, "read-only");
                    }
                },
                close: () => owner.close(),
            });
        } catch (error) {
            release();
            capture?.close();
            throw translated(error, "read-only");
        }
    }
    /** Decoded frames route to the owned renderer without protocol callbacks in app code. */
    async render(
        input: {
            audio?: boolean;
            video?: readonly {
                stream: BrowserStream;
                canvas: HTMLCanvasElement;
            }[];
            volume?: number;
        },
        options: BrowserSdkRequestOptions,
    ): Promise<BrowserRenderHandle> {
        if (
            !input ||
            (input.audio !== undefined && typeof input.audio !== "boolean") ||
            (input.video !== undefined &&
                (!Array.isArray(input.video) || input.video.length > 8))
        )
            invalid();
        const video: Record<string, HTMLCanvasElement> = {};
        for (const v of input.video ?? []) {
            if (
                !v ||
                !(v.stream instanceof BrowserStream) ||
                !v.stream.belongs(
                    this.client,
                    this.conversationId,
                    this.sessionId,
                    this.id,
                ) ||
                Object.hasOwn(video, v.stream.id)
            )
                invalid();
            video[v.stream.id] = v.canvas;
        }
        const audio = input.audio,
            volume = input.volume,
            budget = new Budget(options),
            { native, s, peer, epoch } = await this.native(
                budget.options() as BrowserSdkRequestOptions,
            );
        if (s.renderers.has(this.id))
            throw new BrowserSdkError("conflict", "read-only");
        let renderer: BrowserMediaRenderer | BrowserRenderHandle | undefined;
        try {
            renderer =
                native instanceof BrowserRtcConnection
                    ? await native.render(
                          { audio, video, volume },
                          budget.options(),
                      )
                    : await BrowserMediaRenderer.open(native, {
                          audio,
                          video,
                          volume,
                          ...budget.options(),
                      });
            s.observe(peer, epoch);
            if (s.renderers.has(this.id))
                throw new BrowserSdkError("conflict", "read-only");
            s.renderers.set(this.id, renderer);
            const owner = renderer;
            void renderer.finished
                .finally(() => {
                    if (s.renderers.get(this.id) === owner)
                        s.renderers.delete(this.id);
                })
                .catch(() => {});
            const finished = renderer.finished.catch((error) => {
                throw translated(error, "read-only");
            });
            void finished.catch(() => {});
            return Object.freeze({
                finished,
                get diagnostics() {
                    const v = owner.diagnostics;
                    return Object.freeze({
                        closed: v.closed,
                        paused: v.paused,
                        audio: v.audio,
                        video: v.video,
                    });
                },
                pause: () => owner.pause(),
                resume: async () => {
                    try {
                        await owner.resume();
                    } catch (error) {
                        throw translated(error, "read-only");
                    }
                },
                close: () => owner.close(),
            });
        } catch (error) {
            renderer?.close();
            throw translated(error, "read-only");
        }
    }
}
export class BrowserStream {
    constructor(
        private readonly client: BrowserSdkClient,
        readonly conversationId: ConversationId,
        readonly sessionId: SessionId,
        readonly connectionId: ConnectionId,
        readonly id: StreamId,
    ) {
        Object.freeze(this);
    }
    /** @internal Only validates handle parentage, never current authority. */
    belongs(
        client: BrowserSdkClient,
        cid: ConversationId,
        sid: SessionId,
        connid: ConnectionId,
    ) {
        return (
            this.client === client &&
            this.conversationId === cid &&
            this.sessionId === sid &&
            this.connectionId === connid
        );
    }
    async refresh(
        options: BrowserSdkRequestOptions,
    ): Promise<BrowserStreamSnapshot> {
        const v = (
            await current(this.client).snapshot(this.conversationId, options)
        ).streams.get(
            JSON.stringify([this.sessionId, this.connectionId, this.id]),
        );
        if (
            !v ||
            v.sessionId !== this.sessionId ||
            v.connectionId !== this.connectionId
        )
            throw new BrowserSdkError("receipt-unavailable", "read-only");
        return v;
    }
}
