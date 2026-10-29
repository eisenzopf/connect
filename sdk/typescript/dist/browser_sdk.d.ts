import { BrowserTransportClient, type BrowserTransportClientOptions, type BrowserOperationReference } from "./browser_transport_client.js";
import { CanonicalError, type CanonicalErrorKind, type Certainty } from "./canonical_error.js";
import { type Json, type JsonObject } from "./uctp_json.js";
import { type Scope } from "./uctp_wire.js";
import type { SdkId, SdkRevision, ConversationId, IdentityId, ParticipantId, OperationId, SessionId } from "./sdk_identity.js";
export type { SdkId, SdkRevision, ConversationId, IdentityId, ParticipantId, OperationId, SessionId, } from "./sdk_identity.js";
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
        readonly connectionId?: never;
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
export declare const browserOperationKinds: Readonly<{
    conversationCreate: BrowserOperationKind<BrowserConversation>;
    sessionInvite: BrowserOperationKind<BrowserSession>;
    sessionAccept: BrowserOperationKind<BrowserSession>;
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
    readonly operationKinds: Readonly<{
        conversationCreate: BrowserOperationKind<BrowserConversation>;
        sessionInvite: BrowserOperationKind<BrowserSession>;
        sessionAccept: BrowserOperationKind<BrowserSession>;
        sessionEnd: BrowserOperationKind<BrowserSession>;
        sessionReject: BrowserOperationKind<BrowserSessionDecision>;
        sessionCancelInvitation: BrowserOperationKind<BrowserSessionDecision>;
    }>;
    readonly conversations: {
        byId: (id: ConversationId) => BrowserConversation;
        create: (input: {
            policy: "persistent" | "ephemeral";
            participants: readonly {
                identityId: IdentityId;
                role: string;
            }[];
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
            medium: string;
            intent: string;
            capabilities: Json;
        }, options: BrowserSdkMutationOptions) => Promise<BrowserOperation<BrowserSession>>;
    };
    constructor(client: BrowserSdkClient, id: ConversationId);
    refresh(options: BrowserSdkRequestOptions): Promise<BrowserConversationSnapshot>;
}
export declare class BrowserSession {
    private readonly client;
    readonly conversationId: ConversationId;
    readonly id: SessionId;
    readonly receiptSnapshot?: BrowserSessionSnapshot | undefined;
    constructor(client: BrowserSdkClient, conversationId: ConversationId, id: SessionId, receiptSnapshot?: BrowserSessionSnapshot | undefined);
    refresh(options: BrowserSdkRequestOptions): Promise<BrowserSessionSnapshot>;
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
    readonly medium: string;
    readonly intent: string;
    readonly capabilities: Json;
}
export interface BrowserIncomingInvitations extends AsyncIterableIterator<BrowserIncomingInvitation> {
    next(): Promise<IteratorResult<BrowserIncomingInvitation, undefined>>;
    close(): Promise<void>;
}
