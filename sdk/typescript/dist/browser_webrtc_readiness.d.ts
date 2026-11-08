import { BrowserDeadline } from "./browser_deadline.js";
import type { WebRtcDecodedReadiness } from "./browser_webrtc_replacement.js";
/** Readiness comes from this new RTC receiver's decoded PCM and video output.
 * ICE, DTLS, RTP counters, and an SDP answer cannot satisfy it on their own.
 * The private outputs remain muted; an application owns audible rendering. */
export declare function observeWebRtcDecodedReadiness(peer: RTCPeerConnection, audio: {
    id: string;
    track: MediaStreamTrack;
}, video: {
    id: string;
    track: MediaStreamTrack;
} | undefined, budget: BrowserDeadline, signal: AbortSignal, trackTask: <T>(work: Promise<T>, cleanup?: boolean) => Promise<T>, retainMutedDecoderSinks?: (release: () => Promise<void>) => void): Promise<WebRtcDecodedReadiness>;
