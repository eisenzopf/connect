import type { BrowserReplacementRoot } from "./browser_replacement.js";
import type { WebRtcPublicVideoBinding, WebRtcPublicVideoPhase } from "./browser_webrtc_public_video_phase.js";
import type { ReceiverRtpOutput, ReceiverRtpReleaseFloor, ReceiverVideoReleaseScope, ReceiverOriginalFrame } from "./browser_receiver_video_release.js";
export declare const WEBRTC_BRIDGE_OPT_IN: Readonly<{
    profile: "connect/webrtc-public-video-precut-bridge/1";
    carrier: "webrtc-rtp";
    presentation: "exclusive-public";
    feed: "public-programme-private-decode";
}>;
export type WebRtcBridgeOptIn = typeof WEBRTC_BRIDGE_OPT_IN;
export interface WebRtcBridgeFeeding {
    readonly profile: "connect/webrtc-public-video-precut-bridge/1";
    readonly phase: "feeding";
    readonly reservation_id: string;
    readonly operation_record_id: string;
    readonly operation_id: string;
    readonly target_connection_id: string;
    readonly target_generation: number;
    readonly binding: Readonly<WebRtcPublicVideoBinding>;
    readonly bridge_readiness: Readonly<{
        nonce: string;
        expires_at: number;
        decoder_startup: "fresh-b-receiver-precut-keyframe";
    }>;
}
export interface WebRtcBridgeReadyMarker {
    readonly profile: "connect-replacement-public-video-bridge-ready-v1";
    readonly operation_record_id: string;
    readonly bridge_readiness: Readonly<{
        reservation_id: string;
        nonce: string;
        binding: Readonly<WebRtcPublicVideoBinding>;
        decoded_rtp_timestamp: number;
        decoded_frames: number;
        decoder_startup: "fresh-b-receiver-precut-keyframe";
    }>;
}
export interface WebRtcBridgeSnapshot {
    readonly opt_in: WebRtcBridgeOptIn;
    readonly feeding: Readonly<WebRtcBridgeFeeding> | null;
    readonly decoded_ready: Readonly<WebRtcBridgeReadyMarker> | null;
}
export interface WebRtcReceiverPostCut extends ReceiverVideoReleaseScope {
    readonly profile: "connect-receiver-video-post-cut-v1";
    readonly original_frame: Readonly<ReceiverOriginalFrame>;
    readonly floor: Readonly<ReceiverRtpReleaseFloor>;
}
export declare const bridgeCanonical: (value: unknown) => string;
export declare function bridgeClosed(value: unknown, fields: readonly string[]): Record<string, unknown>;
export declare function parseWebRtcBridgeOptIn(value: unknown): WebRtcBridgeOptIn;
export declare function parseWebRtcBridgeSnapshot(value: unknown, root: BrowserReplacementRoot, reservationId: string, expected: WebRtcPublicVideoBinding, privateNonce: string, reservationNonce: string, targetGeneration: number): Readonly<WebRtcBridgeSnapshot>;
export declare function receiverScope(root: BrowserReplacementRoot, phase: WebRtcPublicVideoPhase): Readonly<ReceiverVideoReleaseScope>;
export declare function parseWebRtcReceiverPostCut(value: unknown, root: BrowserReplacementRoot, phase: WebRtcPublicVideoPhase): Readonly<WebRtcReceiverPostCut>;
/** SAME physical writer only. Half-range/absence/negative/fraction never
 * becomes fresh output; no counter/mediaTime/generation substitution. */
export declare function postCutRtpEligible(witness: WebRtcReceiverPostCut, output: ReceiverRtpOutput | undefined): boolean;
