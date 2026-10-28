import { type SdkRawCanonical } from "./node_raw.js";
import { type SdkDeliveryObservation } from "./node_receipts.js";
import { type SdkHandoffInput, type SdkHandoffResult, type SdkMoveInput, type SdkMoveResult } from "./node_movements.js";
import { type ConnectionContext, type ConnectionId, type SdkConnection, type SdkConnectionUpdateReceipt, type SdkStreamSelectionReceipt } from "./node_connections.js";
import { type SdkSession, type SdkSessions, type SessionId, type SdkSessionDecision, type SdkSessionSnapshot } from "./node_sessions.js";
import { type Json, type JsonObject } from "./uctp_json.js";
import { SdkHelper } from "./sdk_helper.js";
import type { OperationKey as CanonicalOperationKey } from "./canonical_error.js";
import type { CanonicalRequestOptions } from "./canonical_validation.js";
declare const identity: unique symbol;
declare const resultType: unique symbol;
declare const revisionFamily: unique symbol;
export type SdkRevision<K extends string> = number & {
    readonly [revisionFamily]: K;
};
export declare function sdkRevision<K extends string>(value: number): SdkRevision<K>;
export type SdkId<K extends string> = string & {
    readonly [identity]: K;
};
export type ConversationId = SdkId<"conversation">;
export type IdentityId = SdkId<"identity">;
export type ParticipantId = SdkId<"participant">;
export type MessageId = SdkId<"message">;
export type OperationId = SdkId<"operation">;
export interface SdkOperationKey extends CanonicalOperationKey {
    readonly operationId: OperationId;
    readonly scope: {
        readonly conversationId?: ConversationId;
        readonly sessionId?: SdkId<"session">;
        readonly connectionId?: SdkId<"connection">;
    };
}
type OperationKey = SdkOperationKey;
export interface SdkMutationOptions extends CanonicalRequestOptions {
    readonly operationId: OperationId;
    readonly extensions?: Readonly<Record<string, Json>>;
}
export type OperationState = "accepted" | "preparing" | "ready" | "committing" | "completed" | "cancelled" | "failed" | "interrupted" | "unknown";
export interface SdkOperationSnapshot {
    readonly key: OperationKey;
    readonly state: OperationState;
    readonly revision: number;
    readonly recordId?: string;
    readonly cancelRequested?: boolean;
    readonly expiresAt?: number;
    readonly problem?: Json;
}
export interface SdkCompletion<T> {
    readonly snapshot: SdkOperationSnapshot;
    readonly value: T;
}
export interface SdkOperationKind<T> {
    readonly command: string;
    readonly [resultType]: (value: T) => T;
}
export interface SdkOperationReference<T> extends OperationKey {
    readonly [resultType]: (value: T) => T;
}
/** Interface support on the selected Node facade; not permission or readiness. */
export interface SdkCapabilitySnapshot {
    readonly profile: "connect/2";
    readonly runtimeRole: "node";
    readonly selectedTransport: "quic";
    readonly features: ReadonlySet<string>;
    readonly fallbackReason?: string;
}
export interface SdkCapabilities {
    refresh(options: CanonicalRequestOptions): Promise<SdkCapabilitySnapshot>;
    require(feature: string, options: CanonicalRequestOptions): Promise<void>;
}
export interface SdkConsultationResult {
    readonly session: SdkSession;
    readonly retained: readonly SdkConnection[];
    readonly admitted: readonly SdkConnection[];
    readonly retired: readonly ConnectionId[];
    readonly topologyRevision: SdkRevision<"topology">;
    readonly conversationRevision?: SdkRevision<"conversation">;
}
export type SdkConsultationOperation = SdkOperation<SdkConsultationResult>;
export interface SdkConsultInput {
    readonly via: ConnectionId;
    readonly retain: ConnectionId;
    readonly with: {
        readonly first: ParticipantId;
        readonly rest: readonly ParticipantId[];
    };
    readonly hold: "silence" | "configured-media";
    readonly expectedTopologyRevision: SdkRevision<"topology">;
}
export interface SdkMessageAcceptance {
    readonly messageId: MessageId;
    readonly operation: OperationKey;
    readonly boundary: "local-mutation";
}
export interface SdkDecisionReceipt {
    readonly decision: OperationKey;
    readonly target: OperationId;
    readonly observedState: OperationState;
    readonly disposition: string;
    readonly evidenceId?: string;
}
export interface SdkConversationSnapshot {
    readonly id: ConversationId;
    readonly revision: SdkRevision<"conversation">;
    readonly closed: boolean;
    readonly participants: readonly {
        readonly id: ParticipantId;
        readonly conversationId: ConversationId;
        readonly identityId: IdentityId;
        readonly kind: "human" | "ai" | "system" | "external";
        readonly role: string;
        readonly membershipEpoch: number;
        readonly active: boolean;
        readonly removalReason: string | null;
    }[];
}
export type SdkParticipantSnapshot = SdkConversationSnapshot["participants"][number];
declare const runOperation: unique symbol;
declare const sendMessage: unique symbol;
declare const refreshConversation: unique symbol;
declare const readReceipts: unique symbol;
declare const mutateParticipant: unique symbol;
declare const readParticipant: unique symbol;
declare const mutateSession: unique symbol;
declare const readSession: unique symbol;
declare const connectionContext: unique symbol;
declare const returnConsultation: unique symbol;
declare const handoffSession: unique symbol;
declare const moveConnection: unique symbol;
declare const submitMovement: unique symbol;
export declare function sdkId<K extends string>(value: string): SdkId<K>;
declare const operationKinds: Readonly<{
    session_handoff: SdkOperationKind<SdkHandoffResult>;
    connection_replace: SdkOperationKind<SdkMoveResult>;
    session_consult: SdkOperationKind<SdkConsultationResult>;
    session_consult_return: SdkOperationKind<SdkSession>;
    connection_offer: SdkOperationKind<SdkConnection>;
    connection_update: SdkOperationKind<SdkConnectionUpdateReceipt>;
    connection_end: SdkOperationKind<SdkConnection>;
    stream_subscribe: SdkOperationKind<SdkStreamSelectionReceipt>;
    stream_unsubscribe: SdkOperationKind<SdkStreamSelectionReceipt>;
    session_invite: SdkOperationKind<SdkSession>;
    session_accept: SdkOperationKind<SdkSession>;
    session_update: SdkOperationKind<SdkSession>;
    session_end: SdkOperationKind<SdkSession>;
    session_reject: SdkOperationKind<SdkSessionDecision>;
    session_cancel: SdkOperationKind<SdkSessionDecision>;
    conversation_participant_add: SdkOperationKind<SdkParticipant>;
    conversation_participant_update: SdkOperationKind<SdkParticipant>;
    conversation_participant_remove: SdkOperationKind<SdkParticipant>;
    conversation_create: SdkOperationKind<SdkConversation>;
    message_send: SdkOperationKind<SdkMessageAcceptance>;
    operation_cancel: SdkOperationKind<SdkDecisionReceipt>;
    operation_reconcile: SdkOperationKind<SdkDecisionReceipt>;
}>;
/** Typed Conversation/message/decision slice of G6-062. Other control kinds,
 * invitations, administration, events and remote HTTP remain under development.
 * One fixed SDK configuration, one Rust owner, no transparent restart/retry. */
export declare class NodeClient {
    readonly role: "node";
    readonly operationKinds: Readonly<{
        session_handoff: SdkOperationKind<SdkHandoffResult>;
        connection_replace: SdkOperationKind<SdkMoveResult>;
        session_consult: SdkOperationKind<SdkConsultationResult>;
        session_consult_return: SdkOperationKind<SdkSession>;
        connection_offer: SdkOperationKind<SdkConnection>;
        connection_update: SdkOperationKind<SdkConnectionUpdateReceipt>;
        connection_end: SdkOperationKind<SdkConnection>;
        stream_subscribe: SdkOperationKind<SdkStreamSelectionReceipt>;
        stream_unsubscribe: SdkOperationKind<SdkStreamSelectionReceipt>;
        session_invite: SdkOperationKind<SdkSession>;
        session_accept: SdkOperationKind<SdkSession>;
        session_update: SdkOperationKind<SdkSession>;
        session_end: SdkOperationKind<SdkSession>;
        session_reject: SdkOperationKind<SdkSessionDecision>;
        session_cancel: SdkOperationKind<SdkSessionDecision>;
        conversation_participant_add: SdkOperationKind<SdkParticipant>;
        conversation_participant_update: SdkOperationKind<SdkParticipant>;
        conversation_participant_remove: SdkOperationKind<SdkParticipant>;
        conversation_create: SdkOperationKind<SdkConversation>;
        message_send: SdkOperationKind<SdkMessageAcceptance>;
        operation_cancel: SdkOperationKind<SdkDecisionReceipt>;
        operation_reconcile: SdkOperationKind<SdkDecisionReceipt>;
    }>;
    readonly capabilities: SdkCapabilities;
    readonly raw: SdkRawCanonical;
    readonly conversations: Readonly<{
        byId: (id: ConversationId) => SdkConversation;
        create: (input: {
            readonly participants: readonly IdentityId[];
            readonly policy: "persistent" | "ephemeral";
        }, options: SdkMutationOptions) => Promise<SdkOperation<SdkConversation>>;
    }>;
    readonly operations: Readonly<{
        restore: <T>(saved: Json, expected: SdkOperationKind<T>) => SdkOperation<T>;
        recover: <T>(reference: SdkOperationReference<T>) => SdkOperation<T>;
    }>;
    private readonly helper;
    constructor(config: {
        readonly config: string;
        readonly executable?: string;
    });
    private restore;
    private fromReply;
    /** Internal to owned handles; public behavior remains typed, not raw JSON. */
    [runOperation]<T, R>(saved: Json, expected: SdkOperationKind<T>, action: JsonObject, options: CanonicalRequestOptions, decode: (v: Json) => R, key: OperationKey, mutating?: boolean): Promise<R>;
    [sendMessage](id: ConversationId, input: {
        readonly messageId: MessageId;
        readonly to: {
            readonly first: ParticipantId;
            readonly rest: readonly ParticipantId[];
        };
        readonly text: string;
    }, options: SdkMutationOptions): Promise<SdkOperation<SdkMessageAcceptance>>;
    [readReceipts](cid: ConversationId, mid: MessageId, options: CanonicalRequestOptions): Promise<readonly SdkDeliveryObservation[]>;
    [refreshConversation](id: ConversationId, options: CanonicalRequestOptions): Promise<SdkConversationSnapshot>;
    [mutateParticipant](cid: ConversationId, action: JsonObject, kind: SdkOperationKind<SdkParticipant>, options: SdkMutationOptions): Promise<SdkOperation<SdkParticipant>>;
    [readParticipant](cid: ConversationId, pid: ParticipantId, options: CanonicalRequestOptions): Promise<SdkParticipantSnapshot>;
    [mutateSession]<T>(cid: ConversationId, sid: SessionId, action: JsonObject, kind: SdkOperationKind<T>, options: SdkMutationOptions, via?: ConnectionId): Promise<SdkOperation<T>>;
    [returnConsultation](saved: Json, original: OperationKey, expected: SdkRevision<"topology">, options: SdkMutationOptions): Promise<SdkOperation<SdkSession>>;
    [submitMovement]<T>(type: "session_handoff" | "connection_move", cid: ConversationId, sid: SessionId, via: ConnectionId, input: JsonObject, kind: SdkOperationKind<T>, context: JsonObject, options: SdkMutationOptions): Promise<SdkOperation<T>>;
    [handoffSession](cid: ConversationId, sid: SessionId, input: SdkHandoffInput, options: SdkMutationOptions): Promise<SdkOperation<SdkHandoffResult>>;
    [moveConnection](cid: ConversationId, sid: SessionId, via: ConnectionId, input: SdkMoveInput, options: SdkMutationOptions): Promise<SdkOperation<SdkMoveResult>>;
    [readSession](cid: ConversationId, sid: SessionId, options: CanonicalRequestOptions): Promise<SdkSessionSnapshot>;
    [connectionContext](cid: ConversationId, sid: SessionId): ConnectionContext;
    close(options: CanonicalRequestOptions): Promise<void>;
}
export declare class SdkConversation {
    private readonly client;
    readonly id: ConversationId;
    readonly sessions: SdkSessions;
    readonly participants: Readonly<{
        byId: (id: ParticipantId) => SdkParticipant;
        add: (input: {
            readonly identityId: IdentityId;
            readonly kind: "human" | "ai" | "system" | "external";
            readonly role: string;
            readonly expectedRevision: SdkRevision<"conversation">;
        }, options: SdkMutationOptions) => Promise<SdkOperation<SdkParticipant>>;
    }>;
    constructor(client: NodeClient, id: ConversationId);
    refresh(options: CanonicalRequestOptions): Promise<SdkConversationSnapshot>;
    receipts(messageId: MessageId, options: CanonicalRequestOptions): Promise<readonly SdkDeliveryObservation[]>;
    send(input: Parameters<NodeClient[typeof sendMessage]>[1], options: SdkMutationOptions): Promise<SdkOperation<SdkMessageAcceptance>>;
}
export declare class SdkParticipant {
    private readonly client;
    readonly conversationId: ConversationId;
    readonly id: ParticipantId;
    constructor(client: NodeClient, conversationId: ConversationId, id: ParticipantId);
    refresh(options: CanonicalRequestOptions): Promise<SdkParticipantSnapshot>;
    update(input: {
        readonly role: string;
        readonly expectedConversationRevision: SdkRevision<"conversation">;
    }, options: SdkMutationOptions): Promise<SdkOperation<SdkParticipant>>;
    remove(input: {
        readonly expectedConversationRevision: SdkRevision<"conversation">;
    }, options: SdkMutationOptions): Promise<SdkOperation<SdkParticipant>>;
}
type OperationOwner = {
    readonly operationKinds: Pick<typeof operationKinds, "operation_cancel" | "operation_reconcile">;
    readonly operations: {
        restore<T>(saved: Json, kind: SdkOperationKind<T>): SdkOperation<T>;
    };
    [runOperation]: NodeClient[typeof runOperation];
    [returnConsultation]: NodeClient[typeof returnConsultation];
};
export declare class SdkOperation<T> {
    #private;
    private readonly client;
    readonly reference: SdkOperationReference<T>;
    private readonly kind;
    private readonly saved;
    private readonly decode;
    private observe;
    constructor(client: OperationOwner, reference: SdkOperationReference<T>, kind: SdkOperationKind<T>, saved: Json, decode: (value: Json) => T);
    returnToRetained(this: SdkOperation<SdkConsultationResult>, input: {
        readonly expectedTopologyRevision: SdkRevision<"topology">;
    }, options: SdkMutationOptions): Promise<SdkOperation<SdkSession>>;
    referenceJson(): Json;
    status(options: CanonicalRequestOptions): Promise<SdkOperationSnapshot>;
    waitUntilReady(options: CanonicalRequestOptions): Promise<SdkOperationSnapshot>;
    wait(options: CanonicalRequestOptions): Promise<SdkCompletion<T>>;
    cancel(options: SdkMutationOptions): Promise<SdkOperation<SdkDecisionReceipt>>;
    reconcile(input: {
        readonly outcome: "completed" | "cancelled" | "failed";
        readonly evidenceId: string;
    }, options: SdkMutationOptions): Promise<SdkOperation<SdkDecisionReceipt>>;
    private decide;
}
/** Internal trusted-backend operation owner; no provisioning on ParticipantClient. */
export declare function createAdministrativeOperations(helper: SdkHelper, validateReference: (value: Json) => {
    reference: JsonObject;
    kind: string;
}): {
    restore: (saved: Json) => SdkOperation<Json>;
    recover<T>(reference: SdkOperationReference<T>): SdkOperation<T>;
    fromReply(value: Json, key: OperationKey): SdkOperation<Json>;
};
export {};
