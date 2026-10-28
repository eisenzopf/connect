import { type Json, type JsonObject } from "./uctp_json.js";
import type { OperationKey as CanonicalOperationKey } from "./canonical_error.js";
import type { CanonicalRequestOptions } from "./canonical_validation.js";
declare const identity: unique symbol;
declare const resultType: unique symbol;
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
    readonly revision: number;
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
declare const runOperation: unique symbol;
declare const sendMessage: unique symbol;
declare const refreshConversation: unique symbol;
export declare function sdkId<K extends string>(value: string): SdkId<K>;
/** Typed Conversation/message/decision slice of G6-062. Other control kinds,
 * invitations, administration, events and remote HTTP remain under development.
 * One fixed SDK configuration, one Rust owner, no transparent restart/retry. */
export declare class NodeClient {
    readonly role: "node";
    readonly operationKinds: Readonly<{
        conversation_create: SdkOperationKind<SdkConversation>;
        message_send: SdkOperationKind<SdkMessageAcceptance>;
        operation_cancel: SdkOperationKind<SdkDecisionReceipt>;
        operation_reconcile: SdkOperationKind<SdkDecisionReceipt>;
    }>;
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
    [refreshConversation](id: ConversationId, options: CanonicalRequestOptions): Promise<SdkConversationSnapshot>;
    close(options: CanonicalRequestOptions): Promise<void>;
}
export declare class SdkConversation {
    private readonly client;
    readonly id: ConversationId;
    constructor(client: NodeClient, id: ConversationId);
    refresh(options: CanonicalRequestOptions): Promise<SdkConversationSnapshot>;
    send(input: Parameters<NodeClient[typeof sendMessage]>[1], options: SdkMutationOptions): Promise<SdkOperation<SdkMessageAcceptance>>;
}
export declare class SdkOperation<T> {
    #private;
    private readonly client;
    readonly reference: SdkOperationReference<T>;
    private readonly kind;
    private readonly saved;
    private observe;
    constructor(client: NodeClient, reference: SdkOperationReference<T>, kind: SdkOperationKind<T>, saved: Json);
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
export {};
