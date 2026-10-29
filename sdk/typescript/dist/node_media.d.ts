/** Typed packet/data handles on the original NodeClient's separate IPC owner. */
import { SdkHelper } from "./sdk_helper.js";
import type { CanonicalRequestOptions } from "./canonical_validation.js";
import type { OperationKey } from "./canonical_error.js";
import { type Json } from "./uctp_json.js";
import type { ParticipantId, MessageId, SdkMutationOptions, SdkOperation } from "./node_sdk.js";
import type { SdkNonEmpty } from "./node_sessions.js";
import type { StreamId } from "./node_connections.js";
import { SdkRtpPacket, type SdkMediaBinding, type SdkMediaNegotiation, type SdkMediaPacket, type SdkSenderClockReport, type SdkLocalMediaSubmission, type SdkMediaStats, type SdkDataPayload, type SdkReceivedData, type SdkDataAcceptance, type SdkDataStats } from "./node_media_types.js";
export interface SdkMediaStream {
    readonly binding: SdkMediaBinding;
    readonly id: StreamId;
    sendRtp(packet: SdkRtpPacket, options: CanonicalRequestOptions): Promise<SdkLocalMediaSubmission>;
    sendClock(sequence: number, report: SdkSenderClockReport, options: CanonicalRequestOptions): Promise<SdkLocalMediaSubmission>;
    receivePacket(options: CanonicalRequestOptions): Promise<SdkMediaPacket>;
    sendData(input: {
        readonly messageId: MessageId;
        readonly to: SdkNonEmpty<ParticipantId>;
        readonly payload: SdkDataPayload;
    }, options: SdkMutationOptions): Promise<SdkOperation<SdkDataAcceptance>>;
    receiveData(options: CanonicalRequestOptions): Promise<SdkReceivedData>;
}
export interface SdkMediaConnection {
    readonly binding: SdkMediaBinding;
    refresh(options: CanonicalRequestOptions): Promise<SdkMediaNegotiation>;
    streamById(id: StreamId): SdkMediaStream;
}
export interface SdkMedia {
    connection(binding: SdkMediaBinding): SdkMediaConnection;
    stats(options: CanonicalRequestOptions): Promise<SdkMediaStats>;
    dataStats(options: CanonicalRequestOptions): Promise<SdkDataStats>;
    close(options: CanonicalRequestOptions): Promise<void>;
}
interface Context {
    restore(value: Json, key: OperationKey): SdkOperation<SdkDataAcceptance>;
    observe(value: Json, key: OperationKey): void;
}
export declare function createMedia(helper: SdkHelper, ctx: Context): SdkMedia;
export {};
