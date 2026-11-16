import { BrowserDeadline } from "./browser_deadline.js";
import type { WebRtcPreparedVideo } from "./browser_webrtc_readiness.js";
import { WebRtcReceiveRoles } from "./browser_webrtc_receive_roles.js";
import { WebRtcPublicVideoPhaseOwner, type WebRtcPublicVideoPhase, type WebRtcPublicVideoBinding } from "./browser_webrtc_public_video_phase.js";
type TrackTask = <T>(work: Promise<T>, cleanup?: boolean) => Promise<T>;
export interface WebRtcPublicVideoReadyMarker {
    readonly profile: "connect-replacement-public-video-ready-v1";
    readonly operation_record_id: string;
    readonly public_video_readiness: Readonly<{
        reservation_id: string;
        source_cut_id: string;
        nonce: string;
        binding: Readonly<WebRtcPublicVideoBinding>;
        decoded_frames: number;
        decoded_keyframes: number | null;
        decoder_startup: "fresh-receiver-first-decoded-frame";
    }>;
}
export interface WebRtcPreparedPublicVideo {
    readonly readiness: WebRtcPublicVideoReadyMarker;
    assertCurrent(): void;
    take(activated: Readonly<WebRtcPublicVideoPhase>, track: MediaStreamTrack): WebRtcPreparedVideo | undefined;
    release(): Promise<void>;
}
/** One original B observation, reached only after the owning workflow observes
 * the original signed cut-bound-post-f phase and its exact native role. */
export declare function observeWebRtcPublicVideoReadiness(owner: WebRtcPublicVideoPhaseOwner, peer: RTCPeerConnection, roles: WebRtcReceiveRoles, budget: BrowserDeadline, signal: AbortSignal, trackTask: TrackTask): Promise<WebRtcPreparedPublicVideo>;
export {};
