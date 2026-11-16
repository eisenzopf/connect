import { BrowserDeadline } from "./browser_deadline.js";
import type { BrowserReplacementReceipt } from "./browser_replacement.js";
import { BrowserWebRtcReplacement, type WebRtcDecodedReadiness } from "./browser_webrtc_replacement.js";
import { WebRtcReceiveRoles } from "./browser_webrtc_receive_roles.js";
import type { WebRtcReceiveReservationScope } from "./browser_webrtc_receive_reservation.js";
import type { WebRtcPreparedVideo } from "./browser_webrtc_readiness.js";
type TrackTask = <T>(work: Promise<T>, cleanup?: boolean) => Promise<T>;
/** The same original warmed element is loaned to one renderer at a time. A
 * renderer close may pause it, but cannot detach/reset its decoder or replace
 * the original B track. Connection retirement owns its final release. */
export interface WebRtcPublicVideoLoan extends WebRtcPreparedVideo {
    presented(value: number): void;
    release(): void;
}
/** One original reserved B workflow. Verified operation.get observations are
 * supplied by the same signaling owner; plain caller phase/options never enter
 * this state machine. It creates no server authority, source cut or media grant. */
export declare class BrowserWebRtcPublicVideoLifecycle {
    private readonly original;
    private peer;
    private roles;
    private readonly budget;
    private readonly trackTask;
    private readonly phaseOwner;
    private readonly cancel;
    private readonly signal;
    private running?;
    private observation?;
    private completed?;
    private stopped;
    constructor(original: BrowserWebRtcReplacement, peer: RTCPeerConnection | undefined, roles: WebRtcReceiveRoles | undefined, scope: WebRtcReceiveReservationScope, budget: BrowserDeadline, signal: AbortSignal, trackTask: TrackTask);
    private check;
    private options;
    private run;
    private delay;
    private read;
    private phase;
    complete(observePrivate: () => Promise<WebRtcDecodedReadiness>, privateCurrent: () => boolean): Promise<BrowserReplacementReceipt>;
    private perform;
    /** No original operation budget check: this lease was transferred while
     * actual Completed+Activated and that deadline were valid. */
    assertActivated(): void;
    borrow(track: MediaStreamTrack): WebRtcPublicVideoLoan;
    close(): Promise<void>;
}
export {};
