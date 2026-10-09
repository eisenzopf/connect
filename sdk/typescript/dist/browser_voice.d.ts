/** One authenticated browser WebRTC leg retained by its signaling lease. */
export declare class BrowserVoiceSession {
    private readonly peer;
    private readonly socket;
    private readonly localStream;
    readonly remoteStream: MediaStream;
    private connectionId?;
    private closed;
    private constructor();
    static open(signalingUrl: string, credential: string, timeoutMs?: number): Promise<BrowserVoiceSession>;
    /** Attach remote audio and report browser playback-policy/output failure. */
    attach(element: HTMLMediaElement): Promise<void>;
    close(): Promise<void>;
    private negotiate;
}
