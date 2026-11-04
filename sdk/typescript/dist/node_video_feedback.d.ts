/** Original helper-owned feedback lifetime; no media or route authority. */
import { type Json } from "./uctp_json.js";
import type { MediaFrame } from "./node_media_channel.js";
import type { SdkMediaBinding } from "./node_media_types.js";
import type { StreamId } from "./node_connections.js";
export interface SdkVideoReceiveEpoch {
    readonly epoch: string;
    readonly incarnation: string;
    readonly revision: number;
    readonly ssrc: number;
    readonly firstSequence: number;
    readonly firstTimestamp: number;
}
export interface SdkVideoFeedback {
    readonly datagramSequence: number;
    readonly ssrc: number;
    readonly epoch: string;
    readonly epochRevision: number;
}
export interface SdkVideoKeyframePermit {
    readonly feedback: SdkVideoFeedback;
    /** Local original-age bound only; current server authority still gates media. */
    isFresh(): boolean;
    readonly expiresInMs: number;
}
export declare function captureVideoEpoch(value: Json): SdkVideoReceiveEpoch;
export declare function captureVideoFeedback(owner: object, binding: SdkMediaBinding, id: StreamId, frame: MediaFrame): SdkVideoFeedback;
export declare function takeVideoFeedback(owner: object, binding: SdkMediaBinding, id: StreamId, value: SdkVideoFeedback): string;
export declare function consumedVideoFeedback(binding: SdkMediaBinding, id: StreamId, feedback: SdkVideoFeedback, token: string, started: number, frame: MediaFrame): SdkVideoKeyframePermit | undefined;
