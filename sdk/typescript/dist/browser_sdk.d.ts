import { type BrowserMessageAcceptance } from "./browser_sdk_messages.js";
import { type BrowserCursor } from "./browser_sdk_events.js";
export type { BrowserMessageAcceptance } from "./browser_sdk_messages.js";
export type { BrowserCursor, BrowserJournalEvent, BrowserEventStream, } from "./browser_sdk_events.js";
export declare function browserCursor(value: string): BrowserCursor;
import { type BrowserSessionMedium, type BrowserSessionUpdate } from "./browser_sdk_sessions.js";
export type { BrowserSessionMedium, BrowserSessionUpdate, } from "./browser_sdk_sessions.js";
import { type BrowserNonEmpty, type BrowserMedium, type BrowserStreamSelector, type BrowserConnectionUpdate, type BrowserConnectionUpdateReceipt, type BrowserStreamSelectionReceipt } from "./browser_sdk_streams.js";
export type { BrowserNonEmpty, BrowserStreamSelector, BrowserConnectionUpdate, BrowserConnectionUpdateReceipt, BrowserStreamSelectionReceipt, BrowserCodecObservation, BrowserStreamGateObservation, BrowserMedium, BrowserDirection, } from "./browser_sdk_streams.js";
import { type BrowserRtcConfiguration } from "./browser_sdk_webrtc.js";
import { BrowserTransportClient, type BrowserTransportClientOptions, type BrowserOperationReference } from "./browser_transport_client.js";
import { CanonicalError, type CanonicalErrorKind, type Certainty } from "./canonical_error.js";
import { type BrowserMediaCaptureOptions } from "./browser_media_capture.js";
import { type Json, type JsonObject } from "./uctp_json.js";
import { type Scope } from "./uctp_wire.js";
import type { SdkId, SdkRevision, ConversationId, IdentityId, ParticipantId, OperationId, SessionId, ConnectionId, StreamId, MessageId } from "./sdk_identity.js";
export type { SdkId, SdkRevision, ConversationId, IdentityId, ParticipantId, OperationId, SessionId, ConnectionId, StreamId, MessageId, } from "./sdk_identity.js";
export declare class BrowserSdkError extends CanonicalError {
    constructor(kind: CanonicalErrorKind, certainty: Certainty, operation?: BrowserOperationReference, problem?: CanonicalError["problem"]);
}
export declare function sdkId<K extends string>(value: string): SdkId<K>;
export declare function sdkRevision<K extends string>(value: number): SdkRevision<K>;
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
export type BrowserOperationState = "accepted" | "preparing" | "ready" | "committing" | "completed" | "cancelled" | "failed" | "interrupted" | "unknown";
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
export interface BrowserSdkOperationReference<T> extends BrowserOperationReference {
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
export declare const browserOperationKinds: Readonly<{
    connectionUpdate: BrowserOperationKind<BrowserConnectionUpdateReceipt>;
    streamSubscribe: BrowserOperationKind<BrowserStreamSelectionReceipt>;
    streamUnsubscribe: BrowserOperationKind<BrowserStreamSelectionReceipt>;
    connectionOffer: BrowserOperationKind<BrowserConnection>;
    connectionEnd: BrowserOperationKind<BrowserConnection>;
    conversationCreate: BrowserOperationKind<BrowserConversation>;
    messageSend: BrowserOperationKind<BrowserMessageAcceptance>;
    sessionInvite: BrowserOperationKind<BrowserSession>;
    sessionAccept: BrowserOperationKind<BrowserSession>;
    sessionUpdate: BrowserOperationKind<BrowserSession>;
    sessionEnd: BrowserOperationKind<BrowserSession>;
    sessionReject: BrowserOperationKind<BrowserSessionDecision>;
    sessionCancelInvitation: BrowserOperationKind<BrowserSessionDecision>;
}>;
export declare class BrowserOperation<T> {
    #private;
    private readonly client;
    private readonly kind;
    readonly reference: BrowserSdkOperationReference<T>;
    /** @internal Instances are supplied by SDK-owned mutation kinds. */
    constructor(client: BrowserSdkClient, ref: BrowserOperationReference, kind: BrowserOperationKind<T>);
    referenceJson(): Json;
    private observation;
    status(options: BrowserSdkRequestOptions): Promise<BrowserOperationSnapshot>;
    wait(options: BrowserSdkRequestOptions): Promise<BrowserCompletion<T>>;
    waitUntilReady(options: BrowserSdkRequestOptions): Promise<BrowserOperationSnapshot>;
}
declare const submit: unique symbol;
export declare class BrowserSdkClient {
    readonly role: "browser";
    readonly operationKinds: Readonly<{
        connectionUpdate: BrowserOperationKind<BrowserConnectionUpdateReceipt>;
        streamSubscribe: BrowserOperationKind<BrowserStreamSelectionReceipt>;
        streamUnsubscribe: BrowserOperationKind<BrowserStreamSelectionReceipt>;
        connectionOffer: BrowserOperationKind<BrowserConnection>;
        connectionEnd: BrowserOperationKind<BrowserConnection>;
        conversationCreate: BrowserOperationKind<BrowserConversation>;
        messageSend: BrowserOperationKind<BrowserMessageAcceptance>;
        sessionInvite: BrowserOperationKind<BrowserSession>;
        sessionAccept: BrowserOperationKind<BrowserSession>;
        sessionUpdate: BrowserOperationKind<BrowserSession>;
        sessionEnd: BrowserOperationKind<BrowserSession>;
        sessionReject: BrowserOperationKind<BrowserSessionDecision>;
        sessionCancelInvitation: BrowserOperationKind<BrowserSessionDecision>;
    }>;
    readonly conversations: {
        byId: (id: ConversationId) => BrowserConversation;
        create: (input: {
            policy: "persistent" | "ephemeral";
            participants: readonly IdentityId[];
            idleCloseSeconds?: number;
            metadata?: Json;
        }, options: BrowserSdkMutationOptions) => Promise<BrowserOperation<BrowserConversation>>;
    };
    readonly operations: {
        restore: <T>(saved: Json, k: BrowserOperationKind<T>) => BrowserOperation<T>;
        recover: <T>(ref: BrowserSdkOperationReference<T>) => BrowserOperation<T>;
    };
    private constructor();
    static connect(options: BrowserSdkClientOptions): Promise<BrowserSdkClient>;
    get transport(): import("./browser_transport_client.js").BrowserTransportState;
    reconnect(options?: Parameters<BrowserTransportClient["reconnect"]>[0]): Promise<void>;
    close(): void;
    [submit]<T>(k: BrowserOperationKind<T>, body: JsonObject, scope: Scope, options: BrowserSdkMutationOptions): Promise<BrowserOperation<T>>;
    incomingInvitations(options: BrowserSdkRequestOptions): BrowserIncomingInvitations;
}
export declare class BrowserParticipant {
    private readonly client;
    readonly conversationId: ConversationId;
    readonly id: ParticipantId;
    constructor(client: BrowserSdkClient, conversationId: ConversationId, id: ParticipantId);
    refresh(options: BrowserSdkRequestOptions): Promise<BrowserParticipantSnapshot>;
}
export declare class BrowserConversation {
    private readonly client;
    readonly id: ConversationId;
    readonly participants: {
        byId: (id: ParticipantId) => BrowserParticipant;
        byIdentity: (id: IdentityId, options: BrowserSdkRequestOptions) => Promise<BrowserParticipant>;
    };
    readonly sessions: {
        byId: (id: SessionId) => BrowserSession;
        invite: (input: {
            sessionId: SessionId;
            to: readonly ParticipantId[];
            medium: BrowserSessionMedium;
            intent: string;
            capabilities: Json;
        }, options: BrowserSdkMutationOptions) => Promise<BrowserOperation<BrowserSession>>;
    };
    constructor(client: BrowserSdkClient, id: ConversationId);
    send(input: {
        readonly messageId: MessageId;
        readonly to: BrowserNonEmpty<ParticipantId>;
        readonly text: string;
    }, options: BrowserSdkMutationOptions): Promise<BrowserOperation<BrowserMessageAcceptance>>;
    events(input: {
        readonly after?: BrowserCursor;
    }, options: BrowserSdkRequestOptions): import("./browser_sdk_events.js").BrowserEventStream;
    refresh(options: BrowserSdkRequestOptions): Promise<BrowserConversationSnapshot>;
}
export declare class BrowserSession {
    private readonly client;
    readonly conversationId: ConversationId;
    readonly id: SessionId;
    readonly receiptSnapshot?: BrowserSessionSnapshot | undefined;
    readonly connections: {
        byId: (id: ConnectionId) => BrowserConnection;
        offer: (input: {
            connectionId: ConnectionId;
            streams: BrowserNonEmpty<BrowserOfferedStream>;
        }, options: BrowserSdkMutationOptions) => Promise<BrowserOperation<BrowserConnection>>;
    };
    constructor(client: BrowserSdkClient, conversationId: ConversationId, id: SessionId, receiptSnapshot?: BrowserSessionSnapshot | undefined);
    refresh(options: BrowserSdkRequestOptions): Promise<BrowserSessionSnapshot>;
    inviteParticipants(input: {
        readonly to: BrowserNonEmpty<ParticipantId>;
        readonly medium: BrowserSessionMedium;
        readonly intent: string;
        readonly capabilities: Json;
        readonly expectedTopologyRevision: SdkRevision<"topology">;
    }, options: BrowserSdkMutationOptions): Promise<BrowserOperation<BrowserSession>>;
    update(input: BrowserSessionUpdate, expectedTopologyRevision: SdkRevision<"topology">, options: BrowserSdkMutationOptions): Promise<BrowserOperation<BrowserSession>>;
    accept(input: {
        capabilities: Json;
        expectedTopologyRevision: SdkRevision<"topology">;
    }, options: BrowserSdkMutationOptions): Promise<BrowserOperation<BrowserSession>>;
    private decide;
    reject(input: {
        reasonCode: number;
        reason: string;
        expectedTopologyRevision: SdkRevision<"topology">;
    }, options: BrowserSdkMutationOptions): Promise<BrowserOperation<BrowserSessionDecision>>;
    cancelInvitation(input: {
        reasonCode: number;
        reason: string;
        expectedTopologyRevision: SdkRevision<"topology">;
    }, options: BrowserSdkMutationOptions): Promise<BrowserOperation<BrowserSessionDecision>>;
    end(input: {
        reasonCode: number;
        reason: string;
        expectedTopologyRevision: SdkRevision<"topology">;
    }, options: BrowserSdkMutationOptions): Promise<BrowserOperation<BrowserSession>>;
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
export declare class BrowserConnection {
    private readonly client;
    readonly conversationId: ConversationId;
    readonly sessionId: SessionId;
    readonly id: ConnectionId;
    readonly receiptStreamRevision?: SdkRevision<"stream"> | undefined;
    readonly streams: {
        subscribe: (input: {
            readonly selectors: BrowserNonEmpty<BrowserStreamSelector>;
            readonly expectedStreamRevision: SdkRevision<"stream">;
        }, options: BrowserSdkMutationOptions) => Promise<BrowserOperation<BrowserStreamSelectionReceipt>>;
        unsubscribe: (input: {
            readonly streams: BrowserNonEmpty<StreamId>;
            readonly expectedStreamRevision: SdkRevision<"stream">;
        }, options: BrowserSdkMutationOptions) => Promise<BrowserOperation<BrowserStreamSelectionReceipt>>;
        byId: (id: StreamId) => BrowserStream;
        list: (options: BrowserSdkRequestOptions) => Promise<readonly BrowserStream[]>;
    };
    constructor(client: BrowserSdkClient, conversationId: ConversationId, sessionId: SessionId, id: ConnectionId, receiptStreamRevision?: SdkRevision<"stream"> | undefined);
    refresh(options: BrowserSdkRequestOptions): Promise<BrowserConnectionSnapshot>;
    update(input: BrowserConnectionUpdate, expectedStreamRevision: SdkRevision<"stream">, options: BrowserSdkMutationOptions): Promise<BrowserOperation<BrowserConnectionUpdateReceipt>>;
    end(input: {
        reasonCode: number;
        reason: string;
        expectedStreamRevision?: SdkRevision<"stream">;
    }, options: BrowserSdkMutationOptions): Promise<BrowserOperation<BrowserConnection>>;
    private native;
    /** Local capture is explicit and owns only this physical's negotiated senders. */
    capture(input: {
        audio?: boolean;
        video?: "camera" | "screen";
        audioDeviceId?: string;
        videoDeviceId?: string;
        stream?: MediaStream;
        state?: BrowserMediaCaptureOptions["state"];
    }, options: BrowserSdkRequestOptions): Promise<BrowserCaptureHandle>;
    /** Decoded frames route to the owned renderer without protocol callbacks in app code. */
    render(input: {
        audio?: boolean;
        video?: readonly {
            stream: BrowserStream;
            canvas: HTMLCanvasElement;
        }[];
        volume?: number;
    }, options: BrowserSdkRequestOptions): Promise<BrowserRenderHandle>;
}
export declare class BrowserStream {
    private readonly client;
    readonly conversationId: ConversationId;
    readonly sessionId: SessionId;
    readonly connectionId: ConnectionId;
    readonly id: StreamId;
    constructor(client: BrowserSdkClient, conversationId: ConversationId, sessionId: SessionId, connectionId: ConnectionId, id: StreamId);
    /** @internal Only validates handle parentage, never current authority. */
    belongs(client: BrowserSdkClient, cid: ConversationId, sid: SessionId, connid: ConnectionId): boolean;
    refresh(options: BrowserSdkRequestOptions): Promise<BrowserStreamSnapshot>;
}
