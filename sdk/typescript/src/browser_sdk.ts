import {
    BrowserApplicationReads,
    type BrowserApplicationReadResolver,
    type BrowserDeliveryObservation,
} from "./browser_application_reads.js";
export type {
    BrowserApplicationReadOptions,
    BrowserApplicationReadContext,
    BrowserApplicationReadResolver,
    BrowserDeliveryObservation,
} from "./browser_application_reads.js";
import {
    identityKind,
    membershipRole,
    membershipRevision,
    participantView,
    membershipResult,
    type BrowserIdentityKind,
} from "./browser_sdk_membership.js";
export type { BrowserIdentityKind } from "./browser_sdk_membership.js";
import {
    messageBody,
    messageResult,
    type BrowserMessageAcceptance,
} from "./browser_sdk_messages.js";
import {
    createBrowserEvents,
    isBrowserCursor,
    type BrowserCursor,
} from "./browser_sdk_events.js";
export type { BrowserMessageAcceptance } from "./browser_sdk_messages.js";
export type {
    BrowserCursor,
    BrowserJournalEvent,
    BrowserEventStream,
} from "./browser_sdk_events.js";
export function browserCursor(value: string): BrowserCursor {
    if (!isBrowserCursor(value)) invalid();
    return value;
}
import { BrowserInvitationInbox } from "./browser_sdk_inbox.js";
import {
    fields,
    nonEmpty,
    sessionInviteBody,
    sessionUpdateBody,
    sessionCapabilities,
    offeredStreams,
    type BrowserSessionMedium,
    type BrowserSessionUpdate,
} from "./browser_sdk_sessions.js";
export type {
    BrowserSessionMedium,
    BrowserSessionUpdate,
} from "./browser_sdk_sessions.js";
import {
    subscribeBody,
    unsubscribeBody,
    updateBody,
    selectionResult,
    updateResult,
    type BrowserNonEmpty,
    type BrowserMedium,
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
    MessageId,
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
    MessageId,
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
    readonly resolveApplicationRead?: BrowserApplicationReadResolver;
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
    scope: "root" | "conversation" | "session" | "connection";
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
    readonly kind: BrowserIdentityKind;
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
    readonly media: readonly BrowserMedium[];
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
        media: v.media as BrowserMedium[],
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
        (key.command === "session.update"
            ? ![
                  "participant-leave",
                  "role-change",
                  "medium-upgrade",
                  "codec-renegotiate",
              ].includes(String(d.action))
            : d.action !== key.command) ||
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
function participantResult(
    v: Json,
    c: BrowserSdkClient,
    key: BrowserOperationReference,
): BrowserParticipant {
    const result = membershipResult(v, key, () =>
        current(c).malformed(key, "recorded"),
    );
    return new BrowserParticipant(
        c,
        key.scope.conversationId as ConversationId,
        result.snapshot.id,
        result.snapshot,
        result.revision,
    );
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
    conversationCreate: kind("conversation.create", "root", (v, c, k) => {
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
    participantAdd: kind(
        "conversation.participant.add",
        "conversation",
        participantResult,
    ),
    participantUpdate: kind(
        "conversation.participant.update",
        "conversation",
        participantResult,
    ),
    participantRemove: kind(
        "conversation.participant.remove",
        "conversation",
        participantResult,
    ),
    messageSend: kind("message.send", "conversation", (v, c, k) =>
        messageResult(v, k, () => current(c).malformed(k, "recorded")),
    ),
    sessionInvite: kind("session.invite", "session", sessionResult),
    sessionAccept: kind("session.accept", "session", sessionResult),
    sessionUpdate: kind("session.update", "session", sessionResult),
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
              (d.scope === "conversation"
                  ? s.session_id !== undefined
                  : !identifier(s.session_id)) ||
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
    readonly eventReaders = new Set<() => void>();
    readonly inbox = new BrowserInvitationInbox();
    invitationRead?: Promise<Snapshot>;
    invitationFailure?: BrowserSdkError;
    wake?: () => void;
    reader = false;
    stopEvents?: () => void;
    stopClose?: () => void;
    constructor(
        readonly owner: BrowserTransportClient,
        readonly tenant: string,
        readonly actor: string,
        readonly rtcConfiguration: BrowserRtcConfiguration,
        readonly applicationReads: BrowserApplicationReads,
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
        this.applicationReads.retire();
        for (const retire of this.eventReaders) retire();
        this.eventReaders.clear();
        this.inbox.clear();
        this.invitationRead = undefined;
        this.invitationFailure = undefined;
        const peer = this.peer,
            epoch = this.owner.status.physicalEpoch;
        this.stopClose = peer.onClose(() => {
            this.applicationReads.retire();
            for (const retire of this.eventReaders) retire();
            this.inbox.clear();
            this.invitationRead = undefined;
            this.wake?.();
        });
        this.stopEvents = peer.onEvent((frame) => {
            if (this.owner.status.physicalEpoch !== epoch || peer.closed)
                return;
            try {
                if (this.inbox.record(frame, epoch)) this.wake?.();
            } catch (error) {
                this.invitationFailure = translated(error, "read-only");
                peer.close(error);
                this.wake?.();
            }
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
                const view = participantView(d, cid, () => this.malformed());
                if (memberIds.has(view.id)) this.malformed();
                memberIds.add(view.id);
                participants.push(view);
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
                {
                    ...this.reference,
                    command:
                        this.reference.command === "conversation.create"
                            ? "create"
                            : this.reference.command === "message.send"
                              ? "send"
                              : this.reference.command,
                },
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
                participants: readonly IdentityId[];
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
                sdkId(p);
                if (identities.has(p)) invalid();
                identities.add(p);
                return { identity_id: p, role: "participant" };
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
        if (
            options.resolveApplicationRead !== undefined &&
            typeof options.resolveApplicationRead !== "function"
        )
            invalid();
        const applicationReads = new BrowserApplicationReads(
            options.resolveApplicationRead,
            options.identityId,
            options.deviceId,
        );
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
                    applicationReads,
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
        s.applicationReads.retire();
        s.stopEvents?.();
        s.stopClose?.();
        s.inbox.clear();
        s.invitationRead = undefined;
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
                    d.command,
                    captured,
                    { ...scope, operation_id: key.operationId },
                    budget.options(),
                    key,
                );
            const updates = frames.filter(
                (f) => f.type === "operation.updated",
            );
            if (d.command === "message.send") {
                const ack = frames.find((f) => f.type === "ack");
                if (
                    !ack ||
                    ack.cid !== scope.cid ||
                    ack.sid !== undefined ||
                    ack.connid !== undefined ||
                    !object(ack.payload.details)
                )
                    current(this).malformed(key, "uncertain");
                const accepted = messageResult(ack!.payload.details, key, () =>
                    current(this).malformed(key, "uncertain"),
                );
                if (accepted.messageId !== captured.msg_id)
                    current(this).malformed(key, "uncertain");
            } else if (d.scope !== "root") {
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
            if (d.command.startsWith("conversation.participant.")) {
                const bad = () => current(this).malformed(key, "uncertain");
                if (updates[0]?.payload.state !== "completed") bad();
                const result = membershipResult(
                    updates[0].payload.result,
                    key,
                    bad,
                );
                const notices = frames.filter(
                    (f) => f.type === "conversation.participant.changed",
                );
                const action =
                    d.command === "conversation.participant.add"
                        ? "added"
                        : d.command === "conversation.participant.update"
                          ? "updated"
                          : "removed";
                if (
                    (captured.participant_id !== undefined &&
                        result.snapshot.id !== captured.participant_id) ||
                    (captured.identity_id !== undefined &&
                        result.snapshot.identityId !== captured.identity_id) ||
                    notices.length !== 1 ||
                    notices[0].cid !== scope.cid ||
                    notices[0].sid !== undefined ||
                    notices[0].connid !== undefined ||
                    notices[0].operation_id !== key.operationId ||
                    notices[0].expected_revision !== undefined ||
                    notices[0].payload.kind !== action ||
                    notices[0].payload.participant_id !== result.snapshot.id ||
                    notices[0].payload.membership_epoch !==
                        result.snapshot.membershipEpoch
                )
                    bad();
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
        readonly receiptSnapshot?: BrowserParticipantSnapshot,
        readonly receiptConversationRevision?: SdkRevision<"conversation">,
    ) {
        Object.freeze(this);
    }
    update(
        input: {
            readonly role: string;
            readonly expectedConversationRevision: SdkRevision<"conversation">;
        },
        options: BrowserSdkMutationOptions,
    ): Promise<BrowserOperation<BrowserParticipant>> {
        fields(input, ["role", "expectedConversationRevision"], invalid);
        return this.client[submit](
            browserOperationKinds.participantUpdate,
            {
                participant_id: this.id,
                role: membershipRole(input.role, invalid),
            },
            {
                cid: this.conversationId,
                expected_revision: membershipRevision(
                    input.expectedConversationRevision,
                    invalid,
                ),
            },
            options,
        );
    }
    remove(
        input: {
            readonly expectedConversationRevision: SdkRevision<"conversation">;
        },
        options: BrowserSdkMutationOptions,
    ): Promise<BrowserOperation<BrowserParticipant>> {
        fields(input, ["expectedConversationRevision"], invalid);
        return this.client[submit](
            browserOperationKinds.participantRemove,
            { participant_id: this.id },
            {
                cid: this.conversationId,
                expected_revision: membershipRevision(
                    input.expectedConversationRevision,
                    invalid,
                ),
            },
            options,
        );
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
        add: (
            input: {
                readonly identityId: IdentityId;
                readonly kind: BrowserIdentityKind;
                readonly role: string;
                readonly expectedRevision: SdkRevision<"conversation">;
            },
            options: BrowserSdkMutationOptions,
        ): Promise<BrowserOperation<BrowserParticipant>> => {
            fields(
                input,
                ["identityId", "kind", "role", "expectedRevision"],
                invalid,
            );
            if (!identifier(input.identityId) || !identityKind(input.kind))
                invalid();
            return this.client[submit](
                browserOperationKinds.participantAdd,
                {
                    identity_id: input.identityId,
                    kind: input.kind,
                    role: membershipRole(input.role, invalid),
                },
                {
                    cid: this.id,
                    expected_revision: membershipRevision(
                        input.expectedRevision,
                        invalid,
                    ),
                },
                options,
            );
        },
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
                medium: BrowserSessionMedium;
                intent: string;
                capabilities: Json;
            },
            options: BrowserSdkMutationOptions,
        ) => {
            fields(
                input,
                ["sessionId", "to", "medium", "intent", "capabilities"],
                invalid,
            );
            if (!identifier(input.sessionId)) invalid();
            const body = sessionInviteBody(input, invalid);
            return this.client[submit](
                browserOperationKinds.sessionInvite,
                body,
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
    send(
        input: {
            readonly messageId: MessageId;
            readonly to: BrowserNonEmpty<ParticipantId>;
            readonly text: string;
        },
        options: BrowserSdkMutationOptions,
    ): Promise<BrowserOperation<BrowserMessageAcceptance>> {
        return this.client[submit](
            browserOperationKinds.messageSend,
            messageBody(input, invalid),
            { cid: this.id },
            options,
        );
    }
    async receipts(
        messageId: MessageId,
        options: BrowserSdkRequestOptions,
    ): Promise<readonly BrowserDeliveryObservation[]> {
        const budget = new Budget(options),
            s = current(this.client),
            peer = s.peer,
            epoch = s.owner.status.physicalEpoch;
        try {
            return await s.applicationReads.receipts(
                this.id,
                messageId,
                { ...budget.options(), timeoutMs: budget.options().timeoutMs! },
                epoch,
                () => s.observe(peer, epoch),
            );
        } catch (error) {
            throw translated(error, "read-only");
        }
    }
    events(
        input: { readonly after?: BrowserCursor },
        options: BrowserSdkRequestOptions,
    ) {
        new Budget(options);
        const s = current(this.client),
            peer = s.peer,
            epoch = s.owner.status.physicalEpoch;
        return createBrowserEvents(this.id, input, options, {
            check: () => s.observe(peer, epoch),
            read: (after) =>
                s.owner.recoverConversation(
                    {
                        conversationId: this.id,
                        ...(after === undefined ? {} : { cursor: after }),
                        limit: 1,
                    },
                    { timeoutMs: 2000 },
                ),
            reserve: (retire) => {
                s.observe(peer, epoch);
                if (s.eventReaders.size >= 8)
                    throw new BrowserSdkError("conflict", "read-only");
                s.eventReaders.add(retire);
                return () => s.eventReaders.delete(retire);
            },
            error: (kind) => new BrowserSdkError(kind, "read-only"),
            translated: (error) => translated(error, "read-only"),
            bad: () => s.malformed(),
        });
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
                streams: BrowserNonEmpty<BrowserOfferedStream>;
            },
            options: BrowserSdkMutationOptions,
        ): Promise<BrowserOperation<BrowserConnection>> => {
            if (
                !input ||
                !identifier(input.connectionId) ||
                !options ||
                !identifier(options.operationId)
            )
                invalid();
            fields(input, ["connectionId", "streams"], invalid);
            const streams = offeredStreams(input.streams, invalid);
            const low: BrowserConnectionOptions = {
                conversationId: this.conversationId,
                sessionId: this.id,
                connectionId: input.connectionId,
                operationId: options.operationId,
            };
            const seen = new Set<string>();
            for (const stream of streams) {
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
    inviteParticipants(
        input: {
            readonly to: BrowserNonEmpty<ParticipantId>;
            readonly medium: BrowserSessionMedium;
            readonly intent: string;
            readonly capabilities: Json;
            readonly expectedTopologyRevision: SdkRevision<"topology">;
        },
        options: BrowserSdkMutationOptions,
    ): Promise<BrowserOperation<BrowserSession>> {
        fields(
            input,
            [
                "to",
                "medium",
                "intent",
                "capabilities",
                "expectedTopologyRevision",
            ],
            invalid,
        );
        const body = sessionInviteBody(
            { ...input, to: nonEmpty(input.to, identifier, 256, invalid) },
            invalid,
        );
        return this.client[submit](
            browserOperationKinds.sessionInvite,
            body,
            {
                cid: this.conversationId,
                sid: this.id,
                expected_revision: sdkRevision(input.expectedTopologyRevision),
            },
            options,
        );
    }
    update(
        input: BrowserSessionUpdate,
        expectedTopologyRevision: SdkRevision<"topology">,
        options: BrowserSdkMutationOptions,
    ): Promise<BrowserOperation<BrowserSession>> {
        return this.client[submit](
            browserOperationKinds.sessionUpdate,
            sessionUpdateBody(input, invalid),
            {
                cid: this.conversationId,
                sid: this.id,
                expected_revision: sdkRevision(expectedTopologyRevision),
            },
            options,
        );
    }
    accept(
        input: {
            capabilities: Json;
            expectedTopologyRevision: SdkRevision<"topology">;
        },
        options: BrowserSdkMutationOptions,
    ): Promise<BrowserOperation<BrowserSession>> {
        fields(input, ["capabilities", "expectedTopologyRevision"], invalid);
        sessionCapabilities(input.capabilities, invalid);
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
    readonly medium: BrowserSessionMedium;
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
                    if (s.invitationFailure) throw s.invitationFailure;
                    const peer = s.peer,
                        epoch = s.owner.status.physicalEpoch;
                    const queued = s.inbox.peek();
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
                    if (queued.epoch !== epoch) {
                        s.inbox.consume(queued);
                        continue;
                    }
                    const f = queued.frame,
                        p = f.payload;
                    const to = (p.to as string[])[0];
                    // A local wait can detach without cancelling the physical's
                    // finite read. A later wait drains that original work, then
                    // performs its own fresh authorized read; no cached delivery.
                    const previous = s.invitationRead;
                    if (previous) {
                        await boundedLocal(
                            previous.then(
                                () => {},
                                () => {},
                            ),
                            budget.options() as BrowserSdkRequestOptions,
                        );
                        s.observe(peer, epoch);
                        if (s.invitationFailure) throw s.invitationFailure;
                        if (s.inbox.peek() !== queued) continue;
                    }
                    const work = s.snapshot(f.cid as ConversationId, {
                        timeoutMs: 2000,
                    });
                    s.invitationRead = work;
                    const release = () => {
                        if (s.invitationRead === work)
                            s.invitationRead = undefined;
                    };
                    void work.then(release, release);
                    const snapshot = await boundedLocal(
                        work,
                        budget.options() as BrowserSdkRequestOptions,
                    );
                    s.observe(peer, epoch);
                    budget.options();
                    if (s.invitationFailure) throw s.invitationFailure;
                    if (!s.inbox.consume(queued)) continue;
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
                            medium: p.medium as BrowserSessionMedium,
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
    readonly codecs: BrowserNonEmpty<string>;
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
