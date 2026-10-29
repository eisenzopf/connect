import { type Json, type JsonObject } from "./uctp_json.js";
import type { BrowserOperationReference } from "./browser_transport_client.js";
import type { BrowserConnection, BrowserOperation } from "./browser_sdk.js";
import type { BrowserConsultationResult, BrowserTopologyChange, TransitionReference } from "./browser_sdk_consultations.js";
import type { ConnectionId, ParticipantId, SdkId, SdkRevision } from "./sdk_identity.js";
export type EndpointId = SdkId<"endpoint">;
export type DeviceId = SdkId<"device">;
export type BrowserHandoffRole = {
    readonly scope: "session";
} | {
    readonly scope: "conversation";
    readonly expectedConversationRevision: SdkRevision<"conversation">;
};
export interface BrowserHandoffInput {
    readonly via: ConnectionId;
    readonly from: ParticipantId;
    readonly to: ParticipantId;
    readonly retain: ConnectionId;
    readonly candidate?: ConnectionId;
    readonly outgoing: "leave" | "observe";
    readonly role: BrowserHandoffRole;
    readonly expectedTopologyRevision: SdkRevision<"topology">;
    readonly consultation?: BrowserOperation<BrowserConsultationResult>;
}
export type BrowserMoveTarget = {
    readonly endpoint: EndpointId;
    readonly candidate?: never;
} | {
    readonly candidate: ConnectionId;
    readonly endpoint?: never;
};
export interface BrowserMoveInput {
    readonly retain: ConnectionId;
    readonly target: BrowserMoveTarget;
    readonly expectedTopologyRevision: SdkRevision<"topology">;
}
export interface BrowserHandoffRoleChange {
    readonly participantRevision: number;
    readonly roleScope: "session" | "conversation";
    readonly outgoing: "leave" | "observe";
    readonly sourceParticipant: ParticipantId;
    readonly targetParticipant: ParticipantId;
    readonly cleanup: "pending" | "not_requested";
}
export interface BrowserHandoffResult extends BrowserTopologyChange {
    readonly roleChange?: BrowserHandoffRoleChange;
}
export interface BrowserMoveBindingObservation {
    readonly generation: number;
    readonly participantId: ParticipantId;
    readonly membershipEpoch: number;
    readonly endpointId: EndpointId;
    readonly deviceId: DeviceId;
}
export interface BrowserMoveResult extends BrowserTopologyChange {
    readonly connection: BrowserConnection;
    readonly binding: BrowserMoveBindingObservation;
}
type SavedRole = {
    readonly scope: "session";
} | {
    readonly scope: "conversation";
    readonly expected_conversation_revision: SdkRevision<"conversation">;
};
type SavedTarget = {
    readonly kind: "endpoint";
    readonly id: EndpointId;
} | {
    readonly kind: "candidate";
    readonly id: ConnectionId;
};
export type BrowserMovementContinuation = {
    readonly kind: "handoff";
    readonly retained_connection_id: ConnectionId;
    readonly source_participant: ParticipantId;
    readonly target_participant: ParticipantId;
    readonly candidate?: ConnectionId | null;
    readonly role: SavedRole;
    readonly outgoing: "leave" | "observe";
} | {
    readonly kind: "replacement";
    readonly retained_connection_id: ConnectionId;
    readonly target: SavedTarget;
};
export declare const movementCommand: (value: string) => boolean;
export declare function movementContinuation(value: Json | undefined, key: BrowserOperationReference, bad: () => never): BrowserMovementContinuation;
export declare function handoffIntent(input: BrowserHandoffInput, bad: () => never): {
    body: JsonObject;
    context: BrowserMovementContinuation;
    via: ConnectionId;
    expected: SdkRevision<"topology">;
};
export declare function moveIntent(input: BrowserMoveInput, via: ConnectionId, bad: () => never): {
    body: JsonObject;
    context: BrowserMovementContinuation;
    expected: SdkRevision<"topology">;
};
export declare function movementCompletion(value: Json, key: TransitionReference, bad: () => never): {
    retained: ConnectionId[];
    admitted: ConnectionId[];
    retired: ConnectionId[];
    topologyRevision: SdkRevision<"topology">;
    conversationRevision?: SdkRevision<"conversation"> | undefined;
    kind: "handoff";
    roleChange?: BrowserHandoffRoleChange | undefined;
} | {
    retained: ConnectionId[];
    admitted: ConnectionId[];
    retired: ConnectionId[];
    topologyRevision: SdkRevision<"topology">;
    conversationRevision?: SdkRevision<"conversation"> | undefined;
    kind: "replacement";
    connectionId: ConnectionId;
    binding: {
        generation: number;
        participantId: ParticipantId;
        membershipEpoch: number;
        endpointId: EndpointId;
        deviceId: DeviceId;
    };
};
export {};
