import { BrowserClient, type RequestOptions } from "./browser_client.js";
import { type BrowserReplacementReceipt, type BrowserReplacementRoot } from "./browser_replacement.js";
import { type JsonObject } from "./uctp_json.js";
import { type Scope } from "./uctp_wire.js";
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
/** One target's original replacement phases. A readiness assertion is a signed
 * observation of this fresh decoder, never a new replacement mutation. */
export declare class BrowserWebRtcReplacement {
    private readonly client;
    readonly root: BrowserReplacementRoot;
    private allocated;
    private readyAttempted;
    private refused;
    private original?;
    private originalScope?;
    private observedRevision;
    private selected?;
    private readyOffer?;
    constructor(client: BrowserClient, root: BrowserReplacementRoot);
    get mayHaveCommitted(): boolean;
    get challenge(): WebRtcReplacementChallenge;
    get savedReadiness(): JsonObject | undefined;
    private observe;
    private status;
    allocate(payload: JsonObject, scope: Scope, options: RequestOptions, attempted?: () => void): Promise<void>;
    mediaOffer(payload: JsonObject): JsonObject;
    answer(payload: JsonObject): void;
    private completedReceipt;
    /** Internal read-only recovery: never repeat decoded readiness or mutation. */
    recoverCompletion(error: unknown, options: RequestOptions): Promise<BrowserReplacementReceipt | undefined>;
    complete(evidence: WebRtcDecodedReadiness, options: RequestOptions): Promise<BrowserReplacementReceipt>;
}
