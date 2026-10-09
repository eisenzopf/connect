import { ConnectError } from "./errors.js";
import { timeoutValue } from "./protocol.js";
import { voiceDiagnostics, } from "./browser_voice_stats.js";
/** One authenticated browser WebRTC leg retained by its signaling lease. */
export class BrowserVoiceSession {
    peer;
    socket;
    localStream;
    remoteStream = new MediaStream();
    connectionId;
    closed = false;
    constructor(peer, socket, localStream) {
        this.peer = peer;
        this.socket = socket;
        this.localStream = localStream;
        peer.ontrack = (event) => {
            for (const track of event.streams[0]?.getTracks() ?? [event.track])
                this.remoteStream.addTrack(track);
        };
    }
    static async open(signalingUrl, credential, timeoutMs = 15000) {
        timeoutMs = timeoutValue(timeoutMs);
        const parsed = new URL(signalingUrl);
        if (parsed.protocol !== "ws:" ||
            !["127.0.0.1", "[::1]", "localhost"].includes(parsed.hostname) ||
            parsed.username ||
            parsed.password ||
            parsed.search ||
            parsed.hash)
            throw new Error("Use a loopback WebRTC signaling URL");
        if (!/^[A-Za-z0-9._~-]+$/.test(credential))
            throw new ConnectError("invalid-request", "Credential is not WebSocket-safe");
        let stream;
        try {
            stream = await microphoneStream(timeoutMs);
        }
        catch (error) {
            if (error instanceof ConnectError)
                throw error;
            throw new ConnectError("media-denied", "Microphone access was denied");
        }
        const peer = new RTCPeerConnection();
        for (const track of stream.getTracks())
            peer.addTrack(track, stream);
        peer.createDataChannel("connect-control", { ordered: true });
        const socket = new WebSocket(signalingUrl, [
            "rvoip.webrtc.v1",
            `token.${credential}`,
        ]);
        const session = new BrowserVoiceSession(peer, socket, stream);
        try {
            await session.negotiate(timeoutMs);
            return session;
        }
        catch (error) {
            await session.close();
            if (error instanceof ConnectError)
                throw error;
            throw new ConnectError("webrtc-failed", "WebRTC negotiation failed");
        }
    }
    /** Attach remote audio and report browser playback-policy/output failure. */
    async attach(element, timeoutMs = 15000) {
        timeoutMs = timeoutValue(timeoutMs);
        element.srcObject = this.remoteStream;
        element.autoplay = true;
        let timer;
        try {
            await Promise.race([
                element.play(),
                new Promise((_, reject) => {
                    timer = setTimeout(() => reject(new ConnectError("media-output-failed", "Remote audio playback timed out")), timeoutMs);
                }),
            ]);
        }
        catch {
            element.pause();
            element.srcObject = null;
            throw new ConnectError("media-output-failed", "Remote audio playback could not start");
        }
        finally {
            clearTimeout(timer);
        }
    }
    /** Inspect the negotiated path and audio counters without exposing SDP. */
    diagnostics() {
        return voiceDiagnostics(this.peer);
    }
    async close() {
        if (this.closed)
            return;
        this.closed = true;
        if (this.socket.readyState === WebSocket.OPEN && this.connectionId) {
            await new Promise((resolve) => {
                const finish = () => {
                    clearTimeout(timer);
                    this.socket.removeEventListener("message", message);
                    this.socket.removeEventListener("close", finish);
                    resolve();
                };
                const message = (event) => {
                    if (typeof event.data !== "string")
                        return;
                    try {
                        if (JSON.parse(event.data).type === "bye")
                            finish();
                    }
                    catch {
                        // The negotiated session already owns protocol errors.
                    }
                };
                const timer = setTimeout(finish, 500);
                this.socket.addEventListener("message", message);
                this.socket.addEventListener("close", finish, { once: true });
                this.socket.send(JSON.stringify({
                    type: "bye",
                    connection_id: this.connectionId,
                    request_id: crypto.randomUUID(),
                }));
            });
        }
        if (this.socket.readyState === WebSocket.OPEN)
            this.socket.close(1000, "Connect voice ended");
        this.peer.close();
        for (const track of this.localStream.getTracks())
            track.stop();
    }
    async negotiate(timeoutMs) {
        const requestId = crypto.randomUUID();
        const offer = await this.peer.createOffer();
        await this.peer.setLocalDescription(offer);
        await waitForIce(this.peer, timeoutMs);
        const local = this.peer.localDescription;
        if (!local?.sdp || local.sdp.length > 1024 * 1024)
            throw new ConnectError("protocol", "Invalid local WebRTC offer");
        await new Promise((resolve, reject) => {
            const timer = setTimeout(() => reject(new ConnectError("timeout", "WebRTC signaling timed out")), timeoutMs);
            let answerApplied = false;
            let readyReceived = false;
            const finishIfReady = () => {
                if (!answerApplied || !readyReceived)
                    return;
                clearTimeout(timer);
                resolve();
            };
            const fail = () => {
                clearTimeout(timer);
                reject(new ConnectError("webrtc-failed", "WebRTC signaling closed"));
            };
            this.socket.onerror = fail;
            this.socket.onclose = fail;
            this.socket.onopen = () => this.socket.send(JSON.stringify({
                type: "offer-ready",
                sdp: local.sdp,
                request_id: requestId,
            }));
            this.socket.onmessage = async (event) => {
                try {
                    if (typeof event.data !== "string" ||
                        event.data.length > 1024 * 1024)
                        throw new Error();
                    const signal = JSON.parse(event.data);
                    if (signal.request_id && signal.request_id !== requestId)
                        return;
                    if (signal.type === "answer") {
                        if (!signal.sdp || !signal.connection_id)
                            throw new Error();
                        this.connectionId = signal.connection_id;
                        await this.peer.setRemoteDescription({
                            type: "answer",
                            sdp: signal.sdp,
                        });
                        answerApplied = true;
                        finishIfReady();
                    }
                    else if (signal.type === "ready") {
                        readyReceived = true;
                        finishIfReady();
                    }
                    else if (signal.type === "rejected") {
                        throw new ConnectError("forbidden", "Voice invitation was rejected");
                    }
                }
                catch (error) {
                    clearTimeout(timer);
                    reject(error);
                }
            };
        });
    }
}
async function microphoneStream(timeoutMs) {
    const pending = navigator.mediaDevices.getUserMedia({ audio: true });
    return new Promise((resolve, reject) => {
        let expired = false;
        const timer = setTimeout(() => {
            expired = true;
            reject(new ConnectError("timeout", "Microphone access timed out"));
        }, timeoutMs);
        pending.then((stream) => {
            if (expired) {
                for (const track of stream.getTracks())
                    track.stop();
                return;
            }
            clearTimeout(timer);
            resolve(stream);
        }, (error) => {
            if (expired)
                return;
            clearTimeout(timer);
            reject(error);
        });
    });
}
async function waitForIce(peer, timeoutMs) {
    if (peer.iceGatheringState === "complete")
        return;
    await new Promise((resolve, reject) => {
        const timer = setTimeout(() => reject(new ConnectError("timeout", "ICE gathering timed out")), timeoutMs);
        const changed = () => {
            if (peer.iceGatheringState === "complete") {
                clearTimeout(timer);
                peer.removeEventListener("icegatheringstatechange", changed);
                resolve();
            }
        };
        peer.addEventListener("icegatheringstatechange", changed);
    });
}
