import { BrowserDeadline } from "./browser_deadline.js";
import type { WebRtcDecodedReadiness } from "./browser_webrtc_replacement.js";
/** Internal, one-shot transfer between this peer's readiness and renderer. */
export interface WebRtcPreparedAudio {
    readonly context: AudioContext;
    readonly source: MediaStreamAudioSourceNode;
    readonly mutedGain: GainNode;
    readonly playout: HTMLAudioElement;
}
export interface WebRtcPreparedVideo {
    readonly element: HTMLVideoElement;
    readonly presentedFloor: number;
}
export interface WebRtcPreparedDecoder {
    canTakeVideo(track: MediaStreamTrack): boolean;
    takeVideo(track: MediaStreamTrack): WebRtcPreparedVideo | undefined;
    takeAudio(track: MediaStreamTrack): WebRtcPreparedAudio | undefined;
    release(): Promise<void>;
}
/** Readiness comes from this new RTC receiver's decoded PCM and video output.
 * ICE, DTLS, RTP counters, and an SDP answer cannot satisfy it on their own.
 * The private outputs remain muted; an application owns audible rendering. */
export declare function observeWebRtcDecodedReadiness(peer: RTCPeerConnection, audio: {
    id: string;
    track: MediaStreamTrack;
}, video: {
    id: string;
    track: MediaStreamTrack;
} | undefined, budget: BrowserDeadline, signal: AbortSignal, trackTask: <T>(work: Promise<T>, cleanup?: boolean) => Promise<T>, retainMutedDecoderSinks?: (decoder: WebRtcPreparedDecoder) => void): Promise<WebRtcDecodedReadiness>;
