/** Internal state owned by one authenticated BrowserMediaPeer. Wire tags and
 * notices are history, not permission; the server rechecks current authority. */
import { type JsonObject } from "./uctp_json.js";
import { type Envelope } from "./uctp_wire.js";
import { captureNativeVideoFeedback } from "./native_video_feedback.js";
import type { MediaBinding, RtpDatagram } from "./browser_rtp.js";
import { type NativeVideoEpochPacket, type NativeVideoFeedbackPacket } from "./browser_video_wire.js";
type Allocation = ReturnType<typeof captureNativeVideoFeedback>;
export interface VideoRoot {
    cid: string;
    sid: string;
    connid: string;
}
export interface VideoNotice {
    readonly root: VideoRoot;
    readonly stream: string;
    readonly handle: number;
    readonly allocation: Allocation;
    readonly role: "publisher" | "receiver";
    readonly epoch: string;
    readonly incarnation: string;
    readonly revision: number;
    readonly ssrc: number;
    readonly firstSequence: number;
    readonly firstTimestamp: number;
}
export interface VideoFeedbackWrite {
    readonly bytes: Uint8Array;
    readonly notice: VideoNotice;
    readonly version: number;
    readonly started: number;
}
export declare function videoAllocation(stream: JsonObject, local: JsonObject, remote: JsonObject, operation: unknown, offer: unknown, generation: number, previous?: Allocation, connectionGeneration?: number): Allocation | undefined;
/** Call only with an unsolicited frame from this physical client's verified
 * event reader. An operation projection or arbitrary payload is not input. */
export declare function videoNotice(frame: Envelope): VideoNotice;
export declare class BrowserVideoEpochs {
    readonly root: VideoRoot;
    readonly binding: Readonly<MediaBinding>;
    readonly allocation: Allocation;
    private receiver?;
    private publisher?;
    private receiveActive;
    private publishActive;
    private receiveGate;
    private publishGate;
    private version;
    private stopped;
    private pending;
    private hint?;
    private feedbackSequence?;
    private sendSequence;
    private lastRequest;
    private readonly senderSsrc;
    constructor(root: VideoRoot, binding: Readonly<MediaBinding>, allocation: Allocation);
    get pendingPackets(): number;
    matches(n: VideoNotice): boolean;
    /** Retain revision high water even when a gate is disabled. Replayed signed
     * notices cannot reactivate old history after hold/unsubscribe. */
    observe(n: VideoNotice): boolean;
    invalidateRequests(): void;
    unselect(): void;
    gates(send: boolean, receive: boolean): void;
    private admits;
    private stale;
    packet(value: NativeVideoEpochPacket, arrived: number, now: number, capacity?: boolean): RtpDatagram | undefined;
    drain(now: number): {
        packet: RtpDatagram;
        arrived: number;
    }[];
    expire(now: number): void;
    feedback(value: NativeVideoFeedbackPacket, arrived: number): void;
    /** Consumed once immediately before encoding. Age starts at physical read,
     * never at notice arrival, dequeue, encoder acceptance or retry. */
    takeFeedback(now: number, ssrc: number): boolean;
    request(now: number): VideoFeedbackWrite | undefined;
    current(write: VideoFeedbackWrite, now: number): boolean;
    close(): void;
}
export {};
