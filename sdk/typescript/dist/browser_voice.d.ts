import { type VoiceDiagnostics } from "./browser_voice_stats.js";
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
    static open(signalingUrl: string, credential: string, timeoutMs?: number): Promise<BrowserVoiceSession>;
    /** Attach remote audio and report browser playback-policy/output failure. */
    attach(element: HTMLMediaElement, timeoutMs?: number): Promise<void>;
    /** Inspect the negotiated path and audio counters without exposing SDP. */
    diagnostics(): Promise<VoiceDiagnostics>;
    close(): Promise<void>;
    private watchEnd;
    private negotiate;
}
