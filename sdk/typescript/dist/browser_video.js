import { BrowserVoiceSession, } from "./browser_voice.js";
/** Authenticated WebRTC participation in an accepted audio/video Session.
 * Live camera data stays on WebRTC; this is not a native QUIC browser client.
 */
export class BrowserVideoSession {
    session;
    constructor(session) {
        this.session = session;
    }
    static async open(client, options, request = {}) {
        return new BrowserVideoSession(await BrowserVoiceSession.open(client, { ...options, video: true }, request));
    }
    get savedOffer() {
        return this.session.savedOffer;
    }
    get savedEnd() {
        return this.session.savedEnd;
    }
    /** Restart transport on this same audio/video peer, preserving its capture
     * and encoded video worker. Completion is separate from decoded readiness. */
    restartIce(operationId, expectedStreamRevision, expectedIceGeneration, request = {}) {
        return this.session.restartIce(operationId, expectedStreamRevision, expectedIceGeneration, request);
    }
    get savedIceRestart() {
        return this.session.savedIceRestart;
    }
    end(operationId, expectedRevision, request = {}) {
        return this.session.end(operationId, expectedRevision, request);
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
