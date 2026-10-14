import {
    BrowserVoiceSession,
    type BrowserVoiceOptions,
    type VideoDiagnostics,
} from "./browser_voice.js";

/** Authenticated WebRTC participation in an accepted audio/video Session.
 * Live camera data stays on WebRTC; this is not a native QUIC browser client.
 */
export class BrowserVideoSession {
    private constructor(private readonly session: BrowserVoiceSession) {}
    static async open(
        url: string,
        credential: string,
        timeoutMs = 15000,
        options: Omit<BrowserVoiceOptions, "video"> = {},
    ): Promise<BrowserVideoSession> {
        return new BrowserVideoSession(
            await BrowserVoiceSession.open(url, credential, timeoutMs, {
                ...options,
                video: true,
            }),
        );
    }
    get remoteStream(): MediaStream {
        return this.session.remoteStream;
    }
    get localPreview(): MediaStream {
        return this.session.localPreview;
    }
    get ended(): Promise<void> {
        return this.session.ended;
    }
    setCameraMuted(muted: boolean): void {
        this.session.setMuted("video", muted);
    }
    setMicrophoneMuted(muted: boolean): void {
        this.session.setMuted("audio", muted);
    }
    attach(element: HTMLMediaElement, timeoutMs = 15000): Promise<void> {
        return this.session.attach(element, timeoutMs);
    }
    diagnostics(): Promise<Array<VideoDiagnostics>> {
        return this.session.videoDiagnostics();
    }
    close(): Promise<void> {
        return this.session.close();
    }
}
