import { BrowserClient, type RequestOptions } from "./browser_client.js";
import { type BrowserVoiceOptions, type VideoDiagnostics } from "./browser_voice.js";
/** Authenticated WebRTC participation in an accepted audio/video Session.
 * Live camera data stays on WebRTC; this is not a native QUIC browser client.
 */
export declare class BrowserVideoSession {
    private readonly session;
    private constructor();
    static open(client: BrowserClient, options: Omit<BrowserVoiceOptions, "video">, request?: RequestOptions): Promise<BrowserVideoSession>;
    get savedOffer(): import("./browser_webrtc_signaling.js").SavedWebRtcOffer | undefined;
    get savedEnd(): import("./browser_voice.js").SavedWebRtcEnd | undefined;
    end(operationId: string, expectedRevision: number, request?: RequestOptions): Promise<void>;
    get remoteStream(): MediaStream;
    get localPreview(): MediaStream;
    get ended(): Promise<void>;
    setCameraMuted(muted: boolean): void;
    setMicrophoneMuted(muted: boolean): void;
    attach(element: HTMLMediaElement, timeoutMs?: number): Promise<void>;
    diagnostics(): Promise<Array<VideoDiagnostics>>;
    close(): Promise<void>;
}
