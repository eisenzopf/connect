import type { SdkMoveInput, SdkMoveResult } from "./node_movements.js";
import type { ConversationId, ParticipantId, SdkId, SdkRevision, SdkMutationOptions, SdkOperation, SdkOperationKind } from "./node_sdk.js";
import type { SessionId, SdkMedium, SdkNonEmpty } from "./node_sessions.js";
import type { Json, JsonObject } from "./uctp_json.js";
import type { CanonicalRequestOptions } from "./canonical_validation.js";
export type ConnectionId = SdkId<"connection">;
export type StreamId = SdkId<"stream">;
export type SdkDirection = "sendonly" | "recvonly" | "sendrecv" | "inactive";
export interface SdkOfferedStream {
    readonly id: StreamId;
    readonly kind: SdkMedium;
    readonly direction: SdkDirection;
    readonly codecs: SdkNonEmpty<string>;
}
export interface SdkConnectionSnapshot {
    readonly id: ConnectionId;
    readonly conversationId: ConversationId;
    readonly sessionId: SessionId;
    readonly generation: number;
    readonly streamRevision?: SdkRevision<"stream">;
    readonly state: string;
}
export interface SdkNegotiatedStream {
    readonly id: StreamId;
    readonly medium: SdkMedium;
    readonly direction: SdkDirection;
    readonly codec: Json;
    readonly streamLocalId?: number;
}
export interface SdkCodecObservation {
    readonly parameters: SdkNegotiatedStream;
    readonly streamGeneration: number;
    readonly connectionGeneration: number;
}
export interface SdkStreamGateObservation extends SdkCodecObservation {
    readonly state: "active" | "paused";
    readonly sourceMuted: boolean;
    readonly heldDirection?: SdkDirection;
}
export type SdkConnectionUpdateOutcome = {
    readonly kind: "codecs";
    readonly resourcesChanged: boolean;
    readonly streams: readonly SdkCodecObservation[];
} | {
    readonly kind: "gates";
    readonly action: "hold" | "resume" | "mute" | "unmute";
    readonly streams: readonly SdkStreamGateObservation[];
} | {
    readonly kind: "quality";
    readonly intervalMs: number;
};
export interface SdkConnectionUpdateReceipt {
    readonly connection: SdkConnection;
    readonly connectionGeneration: number;
    readonly streamRevision: SdkRevision<"stream">;
    readonly outcome: SdkConnectionUpdateOutcome;
}
export interface SdkStreamSelectionReceipt {
    readonly connection: SdkConnection;
    readonly connectionGeneration: number;
    readonly streamRevision: SdkRevision<"stream">;
    readonly resourcesChanged: boolean;
    readonly bindings: readonly SdkCodecObservation[] | null;
}
export type SdkStreamSelector = {
    readonly stream: StreamId;
} | {
    readonly participant: ParticipantId;
    readonly kinds?: SdkNonEmpty<SdkMedium>;
};
export type SdkConnectionUpdate = {
    readonly action: "mute" | "unmute";
    readonly streams: SdkNonEmpty<StreamId>;
} | {
    readonly action: "hold";
    readonly streams?: SdkNonEmpty<StreamId>;
    readonly direction: "sendonly" | "recvonly" | "inactive";
} | {
    readonly action: "resume";
    readonly streams?: SdkNonEmpty<StreamId>;
} | {
    readonly action: "quality";
    readonly intervalMs: number;
} | {
    readonly action: "codec-renegotiate";
    readonly streams?: SdkNonEmpty<StreamId>;
    readonly codecPreferences: SdkNonEmpty<string>;
};
export interface SdkStreamSnapshot {
    readonly id: StreamId;
    readonly conversationId: ConversationId;
    readonly sessionId: SessionId;
    readonly connectionId: ConnectionId;
    readonly medium: SdkMedium;
    readonly state: string;
    readonly resourceRevision: SdkRevision<"stream-resource">;
    readonly direction?: SdkDirection;
    readonly format?: {
        readonly codec: string;
        readonly clockRate: number;
        readonly channels?: number;
        readonly width?: number;
        readonly height?: number;
    };
    readonly clock?: {
        readonly sourceId: string;
        readonly generation: number;
        readonly unitsPerSecond: bigint;
        readonly synchronized: boolean;
    };
    readonly ownership: "observation" | "local-source" | "local-sink" | "local-duplex";
}
export interface SdkConnections {
    byId(id: ConnectionId): SdkConnection;
    offer(input: {
        readonly connectionId: ConnectionId;
        readonly streams: SdkNonEmpty<SdkOfferedStream>;
    }, options: SdkMutationOptions): Promise<SdkOperation<SdkConnection>>;
}
interface Kinds {
    readonly connection_offer: SdkOperationKind<SdkConnection>;
    readonly connection_update: SdkOperationKind<SdkConnectionUpdateReceipt>;
    readonly connection_end: SdkOperationKind<SdkConnection>;
    readonly stream_subscribe: SdkOperationKind<SdkStreamSelectionReceipt>;
    readonly stream_unsubscribe: SdkOperationKind<SdkStreamSelectionReceipt>;
}
export interface ConnectionContext {
    move(id: ConnectionId, input: SdkMoveInput, options: SdkMutationOptions): Promise<SdkOperation<SdkMoveResult>>;
    readonly cid: ConversationId;
    readonly sid: SessionId;
    readonly kinds: Kinds;
    mutate<T>(id: ConnectionId, action: JsonObject, kind: SdkOperationKind<T>, options: SdkMutationOptions): Promise<SdkOperation<T>>;
    read<T>(id: ConnectionId, action: JsonObject, options: CanonicalRequestOptions, decode: (value: Json) => T): Promise<T>;
}
export declare function createConnections(ctx: ConnectionContext): SdkConnections;
/** Captured receipt revision is historical; byId construction supplies none. */
export declare class SdkConnection {
    private readonly ctx;
    readonly id: ConnectionId;
    readonly receiptStreamRevision?: SdkRevision<"stream"> | undefined;
    readonly conversationId: ConversationId;
    readonly sessionId: SessionId;
    readonly streams: Readonly<{
        byId: (id: StreamId) => SdkStream;
        subscribe: (input: {
            readonly selectors: SdkNonEmpty<SdkStreamSelector>;
            readonly expectedStreamRevision: SdkRevision<"stream">;
        }, options: SdkMutationOptions) => Promise<SdkOperation<SdkStreamSelectionReceipt>>;
        unsubscribe: (input: {
            readonly streams: SdkNonEmpty<StreamId>;
            readonly expectedStreamRevision: SdkRevision<"stream">;
        }, options: SdkMutationOptions) => Promise<SdkOperation<SdkStreamSelectionReceipt>>;
    }>;
    constructor(ctx: ConnectionContext, id: ConnectionId, receiptStreamRevision?: SdkRevision<"stream"> | undefined);
    refresh(options: CanonicalRequestOptions): Promise<SdkConnectionSnapshot>;
    move(input: SdkMoveInput, options: SdkMutationOptions): Promise<SdkOperation<SdkMoveResult>>;
    update(input: SdkConnectionUpdate, expectedStreamRevision: SdkRevision<"stream">, options: SdkMutationOptions): Promise<SdkOperation<SdkConnectionUpdateReceipt>>;
    end(input: {
        readonly reasonCode: number;
        readonly reason: string;
        readonly expectedStreamRevision?: SdkRevision<"stream">;
    }, options: SdkMutationOptions): Promise<SdkOperation<SdkConnection>>;
}
export declare class SdkStream {
    private readonly ctx;
    readonly connectionId: ConnectionId;
    readonly id: StreamId;
    readonly conversationId: ConversationId;
    readonly sessionId: SessionId;
    constructor(ctx: ConnectionContext, connectionId: ConnectionId, id: StreamId);
    refresh(options: CanonicalRequestOptions): Promise<SdkStreamSnapshot>;
}
export declare function connectionHandle(v: Json, ctx: ConnectionContext, expected: ConnectionId): SdkConnection;
export declare function connectionUpdate(v: Json, ctx: ConnectionContext, id: ConnectionId): SdkConnectionUpdateReceipt;
export declare function streamSelection(v: Json, ctx: ConnectionContext, id: ConnectionId): SdkStreamSelectionReceipt;
export {};
