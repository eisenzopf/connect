/** Captured negotiated media and typed data. No capture, renderer or grant. */
import { inspect } from "node:util";
import { type Json, type JsonObject } from "./uctp_json.js";
import type { ConversationId, ParticipantId, MessageId, OperationId, SdkRevision } from "./node_sdk.js";
import type { ConnectionId, StreamId, SdkDirection } from "./node_connections.js";
import type { SessionId, SdkMedium } from "./node_sessions.js";
export declare function mediaInvalid(): never;
export declare function mediaFrozen<T>(value: T): T;
/** Charge the output budget before serializing/copying caller-owned values. */
export declare function mediaJson(value: Json, maximumBytes: number): Buffer;
export declare function uint(value: unknown, bits?: number): number;
export declare function decimal(value: unknown): bigint;
export interface SdkMediaBinding {
    readonly conversationId: ConversationId;
    readonly sessionId: SessionId;
    readonly connectionId: ConnectionId;
}
export declare function captureBinding(v: SdkMediaBinding): SdkMediaBinding;
export declare function bindingWire(v: SdkMediaBinding): JsonObject;
export interface SdkMediaNegotiation {
    readonly binding: SdkMediaBinding;
    readonly sourceClock: boolean;
    readonly streamRevision: SdkRevision<"stream">;
    readonly streams: readonly {
        readonly id: StreamId;
        readonly medium: SdkMedium;
        readonly direction: SdkDirection;
        readonly codec: Json;
        readonly streamLocalId?: number;
    }[];
}
export interface SdkRtpHeader {
    readonly payloadType: number;
    readonly sequenceNumber: number;
    readonly timestamp: number;
    readonly ssrc: number;
    readonly marker: boolean;
}
export declare class SdkRtpPacket {
    #private;
    readonly kind: "rtp";
    readonly datagramSequence: number;
    readonly header: SdkRtpHeader;
    constructor(sequence: number, bytes: Uint8Array);
    get bytes(): Uint8Array;
    get payload(): Uint8Array;
    [inspect.custom](): string;
}
export declare function capturedRtp(value: SdkRtpPacket): Buffer;
export interface SdkSenderClockReport {
    readonly ssrc: number;
    readonly ntp: bigint;
    readonly rtp: number;
    readonly packets: number;
    readonly octets: number;
    readonly cname: string;
}
export declare function clockWire(v: SdkSenderClockReport): JsonObject;
export type SdkMediaPacket = SdkRtpPacket | {
    readonly kind: "clock";
    readonly datagramSequence: number;
    readonly report: SdkSenderClockReport;
} | {
    readonly kind: "gap";
    readonly droppedPackets: bigint;
};
export interface SdkLocalMediaSubmission {
    readonly boundary: "local-media-queue";
    readonly datagramSequence: number;
}
export interface SdkMediaStats {
    readonly receivedDatagrams: bigint;
    readonly rtpPackets: bigint;
    readonly clockReports: bigint;
    readonly malformedDatagrams: bigint;
    readonly unknownHandles: bigint;
    readonly overflowDrops: bigint;
    readonly controlDrops: bigint;
    readonly bindingChangeDrops: bigint;
    readonly queuedPackets: number;
    readonly queuedBytes: number;
}
export declare class SdkTextData {
    readonly value: string;
    readonly kind: "text";
    constructor(value: string);
    [inspect.custom](): string;
}
export declare class SdkJsonData {
    readonly kind: "json";
    readonly value: Json;
    constructor(value: Json);
    [inspect.custom](): string;
}
export declare class SdkBinaryData {
    readonly kind: "binary";
    constructor(bytes: Uint8Array);
    get value(): Uint8Array;
    [inspect.custom](): string;
}
export type SdkDataPayload = SdkTextData | SdkJsonData | SdkBinaryData;
export declare function capturedData(value: SdkDataPayload): Buffer;
export declare function receivedData(format: Json, bytes: Buffer): SdkDataPayload;
export interface SdkReceivedData {
    readonly messageId: MessageId;
    readonly operationId: OperationId;
    readonly fromParticipant: ParticipantId;
    readonly payload: SdkDataPayload;
}
export interface SdkDataAcceptance {
    readonly messageId: MessageId;
    readonly connectionId: ConnectionId;
    readonly boundary: "core-queue-admission";
}
export interface SdkDataStats {
    readonly receivedFrames: bigint;
    readonly acceptedMessages: bigint;
    readonly malformedFrames: bigint;
    readonly unownedFrames: bigint;
    readonly overflowDrops: bigint;
    readonly bindingChangeDrops: bigint;
    readonly queuedMessages: number;
    readonly queuedBytes: number;
}
