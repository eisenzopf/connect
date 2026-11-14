import { type WebRtcDecodedReadiness } from "./browser_webrtc_replacement.js";
import type { BrowserReplacementRoot } from "./browser_replacement.js";
import { type WebRtcIceRestartReceipt } from "./browser_webrtc_restart.js";
import { BrowserClient, type RequestOptions } from "./browser_client.js";
import { type JsonObject } from "./uctp_json.js";
import { type Scope } from "./uctp_wire.js";
export interface WebRtcSignalingOptions {
    conversationId: string;
    sessionId: string;
    connectionId: string;
    operationId: string;
    expectedRevision: number;
    participantId: string;
    capabilities: JsonObject;
    purpose?: "handoff-preparation";
    replacement?: BrowserReplacementRoot;
    streams: ReadonlyArray<{
        id: string;
        kind: "audio" | "video" | "screen" | "data";
        direction: "sendrecv" | "sendonly" | "recvonly" | "inactive";
        codec_preferences: readonly string[];
    }>;
}
export interface SavedWebRtcOffer {
    scope: Scope;
    payload: JsonObject;
    submissionAttempted: boolean;
}
/** Internal canonical signaling owner. Owns the supplied RTCPeerConnection, not
 * its capture tracks or the shared signed client. An applied answer establishes
 * signaling only; media readiness needs independent WebRTC/media observations.
 * The explicit server host admits its bounded negotiated profile; an answer
 * alone does not establish Core readiness or decoded playback.
 */
export declare class CanonicalWebRtcSignaling {
    private readonly client;
    private readonly peer;
    private readonly restartStop;
    private restarts?;
    private readonly tasks;
    readonly finished: Promise<void>;
    private resolveFinished;
    private rejectFinished;
    private readonly scope;
    private readonly template;
    private readonly relayOnly;
    private readonly replacement?;
    private attempted;
    private stopped;
    private failure?;
    private submitted;
    private original?;
    private remoteApplied;
    private ice;
    private candidateCount;
    private candidateBytes;
    private draining?;
    private stopEvents?;
    private stopLifetime?;
    private rejectWait?;
    private readonly peerStateChanged;
    constructor(client: BrowserClient, peer: RTCPeerConnection, options: WebRtcSignalingOptions);
    /** Save this exact original intent for status recovery, never synthesize a replacement offer. */
    get savedOffer(): SavedWebRtcOffer | undefined;
    get savedIceRestart(): {
        scope: Scope;
        payload?: JsonObject;
        submissionAttempted: boolean;
    } | undefined;
    restartIce(operationId: string, expectedStreamRevision: number, expectedIceGeneration: number, options?: RequestOptions): Promise<WebRtcIceRestartReceipt>;
    private check;
    private matches;
    private receive;
    private drain;
    /** One complete-ICE offer. No automatic retry, fallback, redial or message send. */
    offer(options?: RequestOptions): Promise<void>;
    get replacementMayHaveCommitted(): boolean;
    get replacementChallenge(): import("./browser_webrtc_replacement.js").WebRtcReplacementChallenge | undefined;
    completeReplacement(evidence: WebRtcDecodedReadiness, options: RequestOptions): Promise<import("./browser_replacement.js").BrowserReplacementReceipt>;
    recoverReplacementCompletion(error: unknown, options: RequestOptions): Promise<import("./browser_replacement.js").BrowserReplacementReceipt | undefined>;
    private gatheringCleanup?;
    private gather;
    private die;
    private retire;
    /** Local owner retirement only. It does not cancel the durable remote operation. */
    close(): void;
    shutdown(): Promise<void>;
}
