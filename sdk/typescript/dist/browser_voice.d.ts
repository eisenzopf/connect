import { BrowserClient, type RequestOptions } from "./browser_client.js";
import { type BrowserSessionSelection } from "./browser_binding.js";
import { type SavedWebRtcOffer } from "./browser_webrtc_signaling.js";
import { type BrowserCaptureDevices } from "./browser_capture.js";
import { ConnectError } from "./errors.js";
import { type Scope } from "./uctp_wire.js";
import type { JsonObject } from "./uctp_json.js";
import { type VoiceDiagnostics } from "./browser_voice_stats.js";
import { type VideoDiagnostics } from "./browser_video_stats.js";
export type { VideoDiagnostics } from "./browser_video_stats.js";
/** Explicit canonical Connection intent, snapshotted before capture permission. */
export interface BrowserVoiceOptions extends BrowserSessionSelection, BrowserCaptureDevices {
    /** A live one-audio or one-audio/one-video stream. Ownership transfers after validation.
     * Clone tracks first to retain a source. Omit to acquire microphone/camera capture. */
    inputStream?: MediaStream;
    /** Explicit canvas source for video. The original sender is bounded to30fps
     * before signaling; inputStream supplies audio only. Images stay on this canvas. */
    videoCanvas?: HTMLCanvasElement;
    /** Opt into bounded VP8 camera capture; audio uses mono Opus/48000. */
    video?: boolean;
    /** Explicit STUN/TURN services. Copied before capture; credentials stay local.
     * Omit to use native host candidates without an implicit public service. */
    iceServers?: readonly RTCIceServer[];
    /** Relay-only refuses before capture without explicit TURN services and
     * requires the native peer to retain the requested policy before signaling. */
    iceTransportPolicy?: RTCIceTransportPolicy;
}
/** An unsuccessful open retains its original offer for authoritative status recovery.
 * This snapshot can contain private SDP; keep it in application-owned private state. */
export declare class BrowserVoiceOpenError extends ConnectError {
    readonly savedOffer?: SavedWebRtcOffer | undefined;
    constructor(error: unknown, savedOffer?: SavedWebRtcOffer | undefined);
}
export interface SavedWebRtcEnd {
    scope: Scope;
    payload: JsonObject;
    submissionAttempted: boolean;
}
/** Owns capture, the RTC peer and video worker; borrows the shared signed client.
 * open() establishes signaling only. Observe actual media/playback independently. */
export declare class BrowserVoiceSession {
    private readonly client;
    private readonly selection;
    readonly remoteStream: MediaStream;
    private closed;
    private failure?;
    private readonly captureEvents;
    private closing?;
    private resolveEnded;
    /** Settles after local media resources retire. It is not a durable end receipt. */
    readonly ended: Promise<void>;
    private peer?;
    private localStream?;
    private ownedTracks;
    private videoWorker?;
    private canvasCapture?;
    private signaling?;
    private stopLifetime?;
    private stopEvents?;
    private endIntent?;
    private ending?;
    private constructor();
    private check;
    private fail;
    private bound;
    static open(client: BrowserClient, options: BrowserVoiceOptions, request?: RequestOptions): Promise<BrowserVoiceSession>;
    /** A private snapshot of original SDP/operation intent, including uncertain submission. */
    get savedOffer(): SavedWebRtcOffer | undefined;
    /** Same original peer and logical request through uncertain ICE recovery. */
    restartIce(operationId: string, expectedStreamRevision: number, expectedIceGeneration: number, options?: RequestOptions): Promise<import("./browser_webrtc_restart.js").WebRtcIceRestartReceipt>;
    get savedIceRestart(): {
        scope: Scope;
        payload?: JsonObject;
        submissionAttempted: boolean;
    } | undefined;
    get savedEnd(): SavedWebRtcEnd | undefined;
    /** Local capture for a muted preview; this session still owns its tracks. */
    get localPreview(): MediaStream;
    /** Independently mute a captured kind without ending the durable Session. */
    setMuted(kind: "audio" | "video", muted: boolean): void;
    /** Video decode counters, dimensions and timestamps; no SDP or addresses. */
    videoDiagnostics(): Promise<Array<VideoDiagnostics>>;
    /** Attach remote audio and report browser playback-policy/output failure. */
    attach(element: HTMLMediaElement, timeoutMs?: number): Promise<void>;
    /** Inspect the negotiated path and audio counters without exposing SDP. */
    diagnostics(): Promise<VoiceDiagnostics>;
    /** Submit one explicit durable end, then observe its original journal status.
     * Repeating the same call shares its result; another operation/revision refuses.
     * Local capture retires even when the durable outcome remains uncertain. */
    end(operationId: string, expectedRevision: number, request?: RequestOptions): Promise<void>;
    /** Local retirement only; never sends a durable end or closes the shared client. */
    close(): Promise<void>;
    private release;
}
