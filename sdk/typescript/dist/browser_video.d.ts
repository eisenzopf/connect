import { type BrowserVoiceOptions, type VideoDiagnostics } from "./browser_voice.js";
/** Authenticated WebRTC participation in an accepted audio/video Session.
 * Live camera data stays on WebRTC; this is not a native QUIC browser client.
 */
export declare class BrowserVideoSession {
    private readonly session;
    private constructor();
    static open(url: string, credential: string, timeoutMs?: number, options?: Omit<BrowserVoiceOptions, "video">): Promise<BrowserVideoSession>;
    get remoteStream(): MediaStream;
    get localPreview(): MediaStream;
    get ended(): Promise<void>;
    setCameraMuted(muted: boolean): void;
    setMicrophoneMuted(muted: boolean): void;
    attach(element: HTMLMediaElement, timeoutMs?: number): Promise<void>;
    diagnostics(): Promise<Array<VideoDiagnostics>>;
    close(): Promise<void>;
}
