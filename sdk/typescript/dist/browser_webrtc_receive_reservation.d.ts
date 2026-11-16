/** Setup metadata only. Authentication of the enclosing original operation is
 * the caller's prerequisite; parsing this object grants no media or readiness. */
export interface WebRtcReceiveReservationIntent {
    readonly profile: "connect/webrtc-public-video-reservation/1";
    readonly phase: "intent";
    readonly reservation_id: string;
    readonly nonce: string;
    readonly preview_stream_id: string;
    readonly preview_role: "receive";
    readonly public_stream_id: string;
}
export interface WebRtcReceiveReservationScope {
    readonly recordId: string;
    readonly operationId: string;
    readonly targetConnectionId: string;
    readonly audioStreamId: string;
    readonly previewStreamId: string;
    readonly publicStreamId: string;
}
/** Internal transfer from the original verified preparing snapshot. Not an
 * application media option or a public activation/readiness certificate. */
export interface OriginalWebRtcReceiveReservation {
    readonly owner: WebRtcReceiveReservationIntentOwner;
    readonly intent: WebRtcReceiveReservationIntent;
}
type Original = Pick<WebRtcReceiveReservationScope, "recordId" | "operationId" | "targetConnectionId">;
/** Internal provenance check for accidental structural copies/foreign Roots.
 * It is not a signature verifier or a public-output permission check. */
export declare function originalWebRtcReceiveIntent(value: unknown, scope: WebRtcReceiveReservationScope): value is WebRtcReceiveReservationIntent;
/** One original operation's immutable pre-offer intent. Future negotiated/cut
 * phases need different validators and cannot be smuggled through this one. */
export declare class WebRtcReceiveReservationIntentOwner {
    private readonly scope;
    private selected?;
    private closed;
    private physical?;
    constructor(scope: WebRtcReceiveReservationScope);
    observe(value: unknown, original: Original): WebRtcReceiveReservationIntent | undefined;
    owns(intent: WebRtcReceiveReservationIntent, scope: WebRtcReceiveReservationScope): boolean;
    claimPhysical(peer: object, intent: WebRtcReceiveReservationIntent, scope: WebRtcReceiveReservationScope): void;
    ownsPhysical(peer: object, intent: WebRtcReceiveReservationIntent, scope: WebRtcReceiveReservationScope): boolean;
    close(): void;
}
export {};
