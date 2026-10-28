import type { SdkConnections } from "./node_connections.js";
import type { ConversationId, ParticipantId, SdkId, SdkRevision, SdkMutationOptions, SdkOperation, SdkOperationKind, SdkConsultInput, SdkConsultationOperation, SdkConsultationResult } from "./node_sdk.js";
import type { Json, JsonObject } from "./uctp_json.js";
import type { CanonicalRequestOptions } from "./canonical_validation.js";
export type SessionId = SdkId<"session">;
export type SdkMedium = "audio" | "video" | "screen" | "data";
export type SdkSessionMedium = "voice" | "video" | "voice+video" | "screen-share" | "text-chat" | "mixed";
export interface SdkNonEmpty<T> {
    readonly first: T;
    readonly rest: readonly T[];
}
export interface SdkSessionInvite {
    readonly sessionId: SessionId;
    readonly to: readonly ParticipantId[];
    readonly medium: SdkSessionMedium;
    readonly intent: string;
    readonly capabilities: Json;
}
export type SdkSessionUpdate = {
    readonly action: "participant-leave";
    readonly participantId: ParticipantId;
} | {
    readonly action: "role-change";
    readonly participantId: ParticipantId;
    readonly role: string;
} | {
    readonly action: "medium-upgrade";
    readonly media: SdkNonEmpty<SdkMedium>;
} | {
    readonly action: "codec-renegotiate";
    readonly codecPreferences: SdkNonEmpty<string>;
};
export interface SdkSessionDecision {
    readonly session: SdkSession;
    readonly state: string;
    readonly topologyRevision: SdkRevision<"topology">;
}
export interface SdkSessionSnapshot {
    readonly id: SessionId;
    readonly conversationId: ConversationId;
    readonly state: string;
    readonly media: readonly SdkMedium[];
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
export interface SdkSessions {
    byId(id: SessionId): SdkSession;
    invite(input: SdkSessionInvite, options: SdkMutationOptions): Promise<SdkOperation<SdkSession>>;
}
interface Kinds {
    readonly session_consult: SdkOperationKind<SdkConsultationResult>;
    readonly session_invite: SdkOperationKind<SdkSession>;
    readonly session_accept: SdkOperationKind<SdkSession>;
    readonly session_update: SdkOperationKind<SdkSession>;
    readonly session_end: SdkOperationKind<SdkSession>;
    readonly session_reject: SdkOperationKind<SdkSessionDecision>;
    readonly session_cancel: SdkOperationKind<SdkSessionDecision>;
}
interface Context {
    connections(id: SessionId): SdkConnections;
    readonly cid: ConversationId;
    readonly kinds: Kinds;
    mutate<T>(sid: SessionId, action: JsonObject, kind: SdkOperationKind<T>, options: SdkMutationOptions, via?: import("./node_connections.js").ConnectionId): Promise<SdkOperation<T>>;
    refresh(sid: SessionId, options: CanonicalRequestOptions): Promise<SdkSessionSnapshot>;
}
export declare function createSessions(context: Context): SdkSessions;
/** Stable IDs and typed control; creation of a handle never acquires media. */
export declare class SdkSession {
    private readonly context;
    readonly id: SessionId;
    readonly connections: SdkConnections;
    readonly conversationId: ConversationId;
    constructor(context: Context, id: SessionId);
    refresh(options: CanonicalRequestOptions): Promise<SdkSessionSnapshot>;
    inviteParticipants(input: {
        readonly to: SdkNonEmpty<ParticipantId>;
        readonly medium: SdkSessionMedium;
        readonly intent: string;
        readonly capabilities: Json;
        readonly expectedTopologyRevision: SdkRevision<"topology">;
    }, options: SdkMutationOptions): Promise<SdkOperation<SdkSession>>;
    consult(input: SdkConsultInput, options: SdkMutationOptions): Promise<SdkConsultationOperation>;
    accept(input: {
        readonly capabilities: Json;
        readonly expectedTopologyRevision: SdkRevision<"topology">;
    }, options: SdkMutationOptions): Promise<SdkOperation<SdkSession>>;
    reject(input: DecisionInput, options: SdkMutationOptions): Promise<SdkOperation<SdkSessionDecision>>;
    cancelInvitation(input: DecisionInput, options: SdkMutationOptions): Promise<SdkOperation<SdkSessionDecision>>;
    end(input: DecisionInput, options: SdkMutationOptions): Promise<SdkOperation<SdkSession>>;
    private decide;
    update(input: SdkSessionUpdate, expectedTopologyRevision: SdkRevision<"topology">, options: SdkMutationOptions): Promise<SdkOperation<SdkSession>>;
}
interface DecisionInput {
    readonly reasonCode: number;
    readonly reason: string;
    readonly expectedTopologyRevision: SdkRevision<"topology">;
}
export declare function sessionSnapshot(value: Json, cid: ConversationId, sid: SessionId): SdkSessionSnapshot;
export {};
