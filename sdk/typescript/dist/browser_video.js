import { BrowserVoiceSession, } from "./browser_voice.js";
/** Authenticated WebRTC participation in an accepted audio/video Session.
 * Live camera data stays on WebRTC; this is not a native QUIC browser client.
 */
export class BrowserVideoSession {
    session;
    constructor(session) {
        this.session = session;
    }
    static async open(url, credential, timeoutMs = 15000, options = {}) {
        return new BrowserVideoSession(await BrowserVoiceSession.open(url, credential, timeoutMs, {
            ...options,
            video: true,
        }));
    }
    get remoteStream() {
        return this.session.remoteStream;
    }
    get localPreview() {
        return this.session.localPreview;
    }
    get ended() {
        return this.session.ended;
    }
    setCameraMuted(muted) {
        this.session.setMuted("video", muted);
    }
    setMicrophoneMuted(muted) {
        this.session.setMuted("audio", muted);
    }
    attach(element, timeoutMs = 15000) {
        return this.session.attach(element, timeoutMs);
    }
    diagnostics() {
        return this.session.videoDiagnostics();
    }
    close() {
        return this.session.close();
    }
}
