import { type WebRtcBridgeReadyMarker } from "./browser_webrtc_public_video_bridge.js";
import { BrowserClient, type RequestOptions } from "./browser_client.js";
import { BrowserDeadline } from "./browser_deadline.js";
import { type BrowserReplacementReceipt, type BrowserReplacementRoot } from "./browser_replacement.js";
import { type JsonObject } from "./uctp_json.js";
import { type Scope } from "./uctp_wire.js";
import type { OriginalWebRtcReceiveReservation } from "./browser_webrtc_receive_reservation.js";
import type { WebRtcPublicVideoReadyMarker } from "./browser_webrtc_public_video_readiness.js";
export interface WebRtcReplacementChallenge {
    readonly nonce: string;
    readonly source_generation: number;
    readonly target_generation: number;
    readonly stream_revision: number;
}
export interface WebRtcDecodedReadiness {
    readonly audio: {
        readonly stream_id: string;
        readonly decoded_samples: number;
    };
    readonly video: null | {
        readonly stream_id: string;
        readonly decoded_frames: number;
        readonly decoded_keyframes: number | null;
        readonly decoder_startup: "fresh-peer-first-decoded-frame";
    };
}
export declare const recoverableReadinessFailure: (error: unknown) => boolean;
/** One target's original replacement phases. A readiness assertion is a signed
 * observation of this fresh decoder, never a new replacement mutation. */
export declare class BrowserWebRtcReplacement {
    private readonly client;
    private readonly receiveReservation?;
    readonly root: BrowserReplacementRoot;
    private allocated;
    private readyAttempted;
    private refused;
    private original?;
    private originalScope?;
    private observedRevision;
    private selected?;
    private readyOffer?;
    private publicReadyOffer?;
    private bridgeReadyOffer?;
    private bridgeOptIn?;
    private readonly originalObservations;
    constructor(client: BrowserClient, root: BrowserReplacementRoot, receiveReservation?: OriginalWebRtcReceiveReservation | undefined);
    get mayHaveCommitted(): boolean;
    get challenge(): WebRtcReplacementChallenge;
    get savedReadiness(): JsonObject | undefined;
    private observe;
    private status;
    allocate(payload: JsonObject, scope: Scope, options: RequestOptions, attempted?: () => void, originalBudget?: BrowserDeadline): Promise<void>;
    mediaOffer(payload: JsonObject): JsonObject;
    answer(payload: JsonObject): void;
    private completedReceipt;
    /** Internal reserved-profile context. The caller still owns the exact native
     * peer/roles; no plain option or caller boolean enables the workflow. */
    reservedContext(): Readonly<{
        root: BrowserReplacementRoot;
        reservation: OriginalWebRtcReceiveReservation;
        challenge: WebRtcReplacementChallenge;
        bridgeOptIn: Readonly<Readonly<{
            profile: "connect/webrtc-public-video-precut-bridge/1";
            carrier: "webrtc-rtp";
            presentation: "exclusive-public";
            feed: "public-programme-private-decode";
        }>> | undefined;
    }>;
    private checkReservedStatus;
    reservedStatus(options: RequestOptions): Promise<JsonObject>;
    reservedCompleted(value: JsonObject): BrowserReplacementReceipt;
    /** Same frozen private A/audio observation. Only an original signed
     * pre-cut read can permit replay; no decoder is observed again. */
    reservedPrivateReady(evidence: WebRtcDecodedReadiness, options: RequestOptions, original?: JsonObject): Promise<void>;
    reservedBridgeReady(marker: WebRtcBridgeReadyMarker, options: RequestOptions, original?: JsonObject): Promise<void>;
    reservedReceiverWitness(value: JsonObject, final: boolean): unknown;
    /** Frozen B marker uses the original signed offer continuation, never a new
     * business operation, challenge, private readiness or media observation. */
    reservedPublicReady(marker: WebRtcPublicVideoReadyMarker, options: RequestOptions, original?: JsonObject): Promise<void>;
    /** Internal read-only recovery: never repeat decoded readiness or mutation. */
    recoverCompletion(error: unknown, options: RequestOptions): Promise<BrowserReplacementReceipt | undefined>;
    complete(evidence: WebRtcDecodedReadiness, options: RequestOptions): Promise<BrowserReplacementReceipt>;
}
