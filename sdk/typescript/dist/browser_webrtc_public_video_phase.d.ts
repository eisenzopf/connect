import { type BrowserReplacementRoot } from "./browser_replacement.js";
import type { OriginalWebRtcReceiveReservation, WebRtcReceiveReservationScope } from "./browser_webrtc_receive_reservation.js";
export interface WebRtcPublicVideoBinding {
    readonly stream_id: string;
    readonly preview_stream_id: string;
    readonly mid: string;
    readonly stream_generation: number;
    readonly physical_writer_incarnation: string;
    readonly connection_generation: number;
    readonly primary_ssrc: number;
    readonly payload_type: number;
    readonly clock_rate_hz: 90000;
    readonly codec: "vp8";
}
export type WebRtcPublicVideoPhaseName = "negotiated-never-fed" | "cut-bound-post-f" | "decoded-ready" | "activated" | "discarded";
export interface WebRtcPublicVideoPhase {
    readonly profile: "connect/webrtc-public-video-phase/1";
    readonly phase: WebRtcPublicVideoPhaseName;
    readonly reservation_id: string;
    readonly operation_record_id: string;
    readonly operation_id: string;
    readonly target_connection_id: string;
    readonly target_generation: number;
    readonly binding: Readonly<WebRtcPublicVideoBinding>;
    readonly source_cut_id?: string | null;
    readonly reason?: "cancelled" | "expired" | "refused" | "owner-retired" | "binding-changed" | "cut-unavailable";
    readonly public_readiness?: Readonly<{
        nonce: string;
        decoder_startup: "fresh-receiver-first-decoded-frame";
    }>;
}
export interface OriginalPublicVideoObservation {
    readonly recordId: string;
    readonly operationId: string;
    readonly targetConnectionId: string;
    readonly targetGeneration: number;
    /** From the SAME original verified completed movement receipt, not an option. */
    readonly rootCompleted: boolean;
}
/** Metadata validator only. observe() requires the SAME original signed operation
 * observation. Parsing never supplies a grant, source cut, readiness, or IO. */
export declare class WebRtcPublicVideoPhaseOwner {
    private readonly reservation;
    private readonly targetGeneration;
    private readonly privateReadinessNonce;
    readonly root: Readonly<BrowserReplacementRoot>;
    readonly scope: Readonly<WebRtcReceiveReservationScope>;
    private selected?;
    private binding?;
    private cut?;
    private discardedCut?;
    private stopped;
    private readonly retired;
    constructor(root: BrowserReplacementRoot, scope: WebRtcReceiveReservationScope, reservation: OriginalWebRtcReceiveReservation, targetGeneration: number, privateReadinessNonce: string);
    observe(value: unknown, original: OriginalPublicVideoObservation): Readonly<WebRtcPublicVideoPhase>;
    check(): void;
    exactLocalPeer(peer: RTCPeerConnection): boolean;
    accepts(value: Readonly<WebRtcPublicVideoPhase>): boolean;
    cutBound(): Readonly<WebRtcPublicVideoPhase>;
    activated(value: Readonly<WebRtcPublicVideoPhase>): boolean;
    onRetire(fn: () => void): () => void;
    private retire;
    close(): void;
}
