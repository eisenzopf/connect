import { type BrowserMovementContinuation } from "./browser_sdk_movements.js";
import { type Json, type JsonObject } from "./uctp_json.js";
import type { BrowserOperationReference } from "./browser_transport_client.js";
import type { BrowserNonEmpty } from "./browser_sdk_streams.js";
import type { BrowserConnection, BrowserSession } from "./browser_sdk.js";
import type { ConnectionId, ParticipantId, SdkId, SdkRevision } from "./sdk_identity.js";
export type BrowserOperationRecordId = SdkId<"operation-record">;
/** Opaque persistence context; this is original intent, never an authority grant. */
export type BrowserTransitionContinuation = BrowserMovementContinuation | {
    readonly kind: "consultation";
    readonly retained_connection_id: ConnectionId;
    readonly record_id?: BrowserOperationRecordId;
} | {
    readonly kind: "consultation-return";
    readonly retained_connection_id: ConnectionId;
    readonly consultation_record_id: BrowserOperationRecordId;
};
export interface TransitionReference extends BrowserOperationReference {
    readonly continuation?: BrowserTransitionContinuation;
}
export interface BrowserConsultInput {
    readonly via: ConnectionId;
    readonly retain: ConnectionId;
    readonly with: BrowserNonEmpty<ParticipantId>;
    readonly hold: "silence" | "configured-media";
    readonly expectedTopologyRevision: SdkRevision<"topology">;
}
export interface BrowserTopologyChange {
    readonly retained: readonly BrowserConnection[];
    readonly admitted: readonly BrowserConnection[];
    readonly retired: readonly ConnectionId[];
    readonly topologyRevision: SdkRevision<"topology">;
    readonly conversationRevision?: SdkRevision<"conversation">;
}
export interface BrowserConsultationResult extends BrowserTopologyChange {
    readonly session: BrowserSession;
}
export interface BrowserConsultationReturnReceipt {
    readonly consultationRecordId: BrowserOperationRecordId;
    readonly topology: BrowserTopologyChange;
    readonly remoteContinuing: boolean;
}
export declare const transitionCommand: (command: string) => boolean;
export declare function transitionContinuation(value: Json | undefined, key: BrowserOperationReference, bad: () => never): BrowserTransitionContinuation | undefined;
export declare function consultBody(input: BrowserConsultInput, bad: () => never): JsonObject;
export declare function transitionObservation(value: JsonObject, key: TransitionReference, expected: number | undefined, bad: () => never): void;
export declare function consultationCompletion(value: Json, key: TransitionReference, bad: () => never): {
    retained: ConnectionId[];
    admitted: ConnectionId[];
    retired: ConnectionId[];
    topologyRevision: SdkRevision<"topology">;
    consultationRecordId?: BrowserOperationRecordId | undefined;
    remoteContinuing?: boolean | undefined;
};
