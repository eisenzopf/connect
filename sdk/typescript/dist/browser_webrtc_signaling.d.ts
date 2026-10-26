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
    streams: ReadonlyArray<{
        id: string;
        kind: "audio" | "video" | "screen" | "data";
        direction: "sendrecv" | "sendonly" | "recvonly" | "inactive";
        codec_preferences: readonly string[];
    }>;
}
/** Internal canonical signaling owner. Owns the supplied RTCPeerConnection, not
 * its capture tracks or the shared signed client. An applied answer establishes
 * signaling only; media readiness needs independent WebRTC/media observations.
 * The current server has not yet wired this WebRTC substrate into canonical IO.
 */
export declare class CanonicalWebRtcSignaling {
    private readonly client;
    private readonly peer;
    readonly finished: Promise<void>;
    private resolveFinished;
    private rejectFinished;
    private readonly scope;
    private readonly template;
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
    get savedOffer(): Readonly<{
        scope: Scope;
        payload: JsonObject;
        submissionAttempted: boolean;
    }> | undefined;
    private check;
    private matches;
    private receive;
    private drain;
    /** One complete-ICE offer. No automatic retry, fallback, redial or message send. */
    offer(options?: RequestOptions): Promise<void>;
    private gatheringCleanup?;
    private gather;
    private die;
    private retire;
    /** Local owner retirement only. It does not cancel the durable remote operation. */
    close(): void;
}
