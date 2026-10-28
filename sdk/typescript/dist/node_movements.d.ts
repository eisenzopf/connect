import type { SdkId, SdkRevision, ParticipantId, SdkConsultationOperation, SdkOperationKey } from "./node_sdk.js";
import type { ConnectionId, SdkConnection } from "./node_connections.js";
import type { Json, JsonObject } from "./uctp_json.js";
export type EndpointId = SdkId<"endpoint">;
export type DeviceId = SdkId<"device">;
export interface SdkTopologyChange {
    readonly retained: readonly SdkConnection[];
    readonly admitted: readonly SdkConnection[];
    readonly retired: readonly ConnectionId[];
    readonly topologyRevision: SdkRevision<"topology">;
    readonly conversationRevision?: SdkRevision<"conversation">;
}
export type SdkHandoffRole = {
    readonly scope: "session";
} | {
    readonly scope: "conversation";
    readonly expectedConversationRevision: SdkRevision<"conversation">;
};
export interface SdkHandoffInput {
    readonly via: ConnectionId;
    readonly from: ParticipantId;
    readonly to: ParticipantId;
    readonly retain: ConnectionId;
    readonly candidate?: ConnectionId;
    readonly outgoing: "leave" | "observe";
    readonly role: SdkHandoffRole;
    readonly expectedTopologyRevision: SdkRevision<"topology">;
    readonly consultation?: SdkConsultationOperation;
}
export interface SdkHandoffResult extends SdkTopologyChange {
    /** Captured host result, not current role authority or native custody. */
    readonly roleChange?: {
        readonly participantRevision: number;
        readonly roleScope: "session" | "conversation";
        readonly outgoing: "leave" | "observe";
        readonly sourceParticipant: ParticipantId;
        readonly targetParticipant: ParticipantId;
        readonly cleanup: "pending" | "not_requested";
    };
}
export type SdkMoveTarget = {
    readonly endpoint: EndpointId;
    readonly candidate?: never;
} | {
    readonly candidate: ConnectionId;
    readonly endpoint?: never;
};
export interface SdkMoveInput {
    readonly retain: ConnectionId;
    readonly target: SdkMoveTarget;
    readonly expectedTopologyRevision: SdkRevision<"topology">;
}
export interface SdkMoveResult extends SdkTopologyChange {
    readonly connection: SdkConnection;
    /** Captured binding observation; metadata handles acquire no media ownership. */
    readonly binding: {
        readonly generation: number;
        readonly participantId: ParticipantId;
        readonly membershipEpoch: number;
        readonly endpointId: EndpointId;
        readonly deviceId: DeviceId;
    };
}
export declare function handoffInput(v: SdkHandoffInput): JsonObject;
export declare function moveInput(v: SdkMoveInput, via: ConnectionId): JsonObject;
export declare function movementContext(v: Json, command: string, via: Json): boolean;
interface Context {
    readonly key: SdkOperationKey;
    connection(id: ConnectionId): SdkConnection;
}
export declare function handoffResult(value: Json, ctx: Context, saved: Json): SdkHandoffResult;
export declare function moveResult(value: Json, ctx: Context, saved: Json): SdkMoveResult;
export {};
