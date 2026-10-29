import type { BrowserConnection, SdkRevision, StreamId, ParticipantId } from "./browser_sdk.js";
import type { BrowserOperationReference } from "./browser_transport_client.js";
import { type Json, type JsonObject } from "./uctp_json.js";
export type BrowserMedium = "audio" | "video" | "screen" | "data";
export type BrowserDirection = "sendonly" | "recvonly" | "sendrecv" | "inactive";
export interface BrowserNonEmpty<T> {
    readonly first: T;
    readonly rest: readonly T[];
}
export type BrowserStreamSelector = {
    readonly stream: StreamId;
} | {
    readonly participant: ParticipantId;
    readonly kinds?: BrowserNonEmpty<BrowserMedium>;
};
export type BrowserConnectionUpdate = {
    readonly action: "mute" | "unmute";
    readonly streams: BrowserNonEmpty<StreamId>;
} | {
    readonly action: "hold";
    readonly streams?: BrowserNonEmpty<StreamId>;
    readonly direction: "sendonly" | "recvonly" | "inactive";
} | {
    readonly action: "resume";
    readonly streams?: BrowserNonEmpty<StreamId>;
} | {
    readonly action: "quality";
    readonly intervalMs: number;
} | {
    readonly action: "codec-renegotiate";
    readonly streams?: BrowserNonEmpty<StreamId>;
    readonly codecPreferences: BrowserNonEmpty<string>;
};
export interface BrowserCodecObservation {
    readonly parameters: {
        readonly id: StreamId;
        readonly medium: BrowserMedium;
        readonly direction: BrowserDirection;
        readonly codec: Json;
        readonly streamLocalId: number;
    };
    readonly streamGeneration: number;
    readonly connectionGeneration: number;
}
export interface BrowserStreamGateObservation extends BrowserCodecObservation {
    readonly state: "active" | "paused";
    readonly sourceMuted: boolean;
    readonly heldDirection?: BrowserDirection;
}
export interface BrowserConnectionUpdateReceipt {
    readonly connection: BrowserConnection;
    readonly connectionGeneration: number;
    readonly streamRevision: SdkRevision<"stream">;
    readonly outcome: {
        readonly kind: "codecs";
        readonly resourcesChanged: boolean;
        readonly streams: readonly BrowserCodecObservation[];
    } | {
        readonly kind: "gates";
        readonly action: "hold" | "resume" | "mute" | "unmute";
        readonly streams: readonly BrowserStreamGateObservation[];
    } | {
        readonly kind: "quality";
        readonly intervalMs: number;
    };
}
export interface BrowserStreamSelectionReceipt {
    readonly connection: BrowserConnection;
    readonly connectionGeneration: number;
    readonly streamRevision: SdkRevision<"stream">;
    readonly resourcesChanged: boolean;
    readonly bindings: readonly BrowserCodecObservation[] | null;
}
export declare function subscribeBody(selectors: BrowserNonEmpty<BrowserStreamSelector>, bad: () => never): JsonObject;
export declare function unsubscribeBody(streams: BrowserNonEmpty<StreamId>, bad: () => never): JsonObject;
export declare function updateBody(input: BrowserConnectionUpdate, bad: () => never): JsonObject;
export declare function selectionResult(value: Json, key: BrowserOperationReference, connection: BrowserConnection, bad: () => never): BrowserStreamSelectionReceipt;
export declare function updateResult(value: Json, key: BrowserOperationReference, connection: BrowserConnection, bad: () => never): BrowserConnectionUpdateReceipt;
