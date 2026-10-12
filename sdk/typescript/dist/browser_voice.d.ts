import { type VoiceDiagnostics } from "./browser_voice_stats.js";
/** Optional audio supplied by an application, such as processed or generated speech. */
export interface BrowserVoiceOptions {
    /**
     * A live audio-only stream. The session takes ownership of its tracks and
     * stops them on failure or hangup. Clone tracks first to retain a source.
     * Omit this to acquire the user's microphone.
     */
    inputStream?: MediaStream;
}
/** One authenticated browser WebRTC leg retained by its signaling lease. */
export declare class BrowserVoiceSession {
    private readonly peer;
    private readonly socket;
    private readonly localStream;
    readonly remoteStream: MediaStream;
    private connectionId?;
    private closed;
    private resolveEnded;
    /** Settles after either side ends this leg and microphone capture stops. */
    readonly ended: Promise<void>;
    private constructor();
    static open(signalingUrl: string, credential: string, timeoutMs?: number, options?: BrowserVoiceOptions): Promise<BrowserVoiceSession>;
    /** Attach remote audio and report browser playback-policy/output failure. */
    attach(element: HTMLMediaElement, timeoutMs?: number): Promise<void>;
    /** Inspect the negotiated path and audio counters without exposing SDP. */
    diagnostics(): Promise<VoiceDiagnostics>;
    close(): Promise<void>;
    private watchEnd;
    private negotiate;
}
