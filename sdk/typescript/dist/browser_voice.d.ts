import { type BrowserSessionSelection } from "./browser_binding.js";
import { type BrowserCaptureDevices } from "./browser_capture.js";
import { type VoiceDiagnostics } from "./browser_voice_stats.js";
import { type VideoDiagnostics } from "./browser_video_stats.js";
export type { VideoDiagnostics } from "./browser_video_stats.js";
/** Optional audio supplied by an application, such as processed or generated speech. */
export interface BrowserVoiceOptions extends BrowserSessionSelection, BrowserCaptureDevices {
    /**
     * A live stream matching the selected profile: one audio track by default,
     * or one audio and one video track with video=true. The session owns and
     * stops its tracks on failure/hangup. Clone tracks first to retain a source.
     * Omit this to acquire the user's microphone.
     */
    inputStream?: MediaStream;
    /** Opt into the VP8/640×360 audio/video profile. Audio-only remains the default. */
    video?: boolean;
}
/** One authenticated browser WebRTC leg retained by its signaling lease. */
export declare class BrowserVoiceSession {
    private readonly peer;
    private readonly socket;
    private readonly localStream;
    private readonly videoWorker?;
    readonly remoteStream: MediaStream;
    private connectionId?;
    private closed;
    private readonly captureEvents;
    private closing?;
    private resolveEnded;
    /** Settles after either side ends this leg and microphone capture stops. */
    readonly ended: Promise<void>;
    private constructor();
    static open(signalingUrl: string, credential: string, timeoutMs?: number, options?: BrowserVoiceOptions): Promise<BrowserVoiceSession>;
    /** Local capture for a muted preview; this session still owns its tracks. */
    get localPreview(): MediaStream;
    /** Independently mute a captured kind without ending the durable Session. */
    setMuted(kind: "audio" | "video", muted: boolean): void;
    /** Video decode counters, dimensions and timestamps; no SDP or addresses. */
    videoDiagnostics(): Promise<Array<VideoDiagnostics>>;
    /** Attach remote audio and report browser playback-policy/output failure. */
    attach(element: HTMLMediaElement, timeoutMs?: number): Promise<void>;
    /** Inspect the negotiated path and audio counters without exposing SDP. */
    diagnostics(): Promise<VoiceDiagnostics>;
    close(): Promise<void>;
    private release;
    private watchEnd;
}
