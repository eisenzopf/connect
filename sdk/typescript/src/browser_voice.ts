import { ConnectError } from "./errors.js";
import { timeoutValue } from "./protocol.js";
import {
    voiceDiagnostics,
    type VoiceDiagnostics,
} from "./browser_voice_stats.js";

interface Signal {
    type?: string;
    sdp?: string;
    connection_id?: string;
    request_id?: string;
}

/** One authenticated browser WebRTC leg retained by its signaling lease. */
export class BrowserVoiceSession {
    readonly remoteStream = new MediaStream();
    private connectionId?: string;
    private closed = false;
    private resolveEnded: () => void = () => {};
    /** Settles after either side ends this leg and microphone capture stops. */
    readonly ended = new Promise<void>((resolve) => {
        this.resolveEnded = resolve;
    });

    private constructor(
        private readonly peer: RTCPeerConnection,
        private readonly socket: WebSocket,
        private readonly localStream: MediaStream,
    ) {
        peer.ontrack = (event) => {
            for (const track of event.streams[0]?.getTracks() ?? [event.track])
                this.remoteStream.addTrack(track);
        };
    }

    static async open(
        signalingUrl: string,
        credential: string,
        timeoutMs = 15000,
    ): Promise<BrowserVoiceSession> {
        timeoutMs = timeoutValue(timeoutMs);
        const parsed = new URL(signalingUrl);
        if (
            parsed.protocol !== "ws:" ||
            !["127.0.0.1", "[::1]", "localhost"].includes(parsed.hostname) ||
            parsed.username ||
            parsed.password ||
            parsed.search ||
            parsed.hash
        )
            throw new Error("Use a loopback WebRTC signaling URL");
        if (!/^[A-Za-z0-9._~-]+$/.test(credential))
            throw new ConnectError(
                "invalid-request",
                "Credential is not WebSocket-safe",
            );
        let stream: MediaStream;
        try {
            stream = await microphoneStream(timeoutMs);
        } catch (error) {
            if (error instanceof ConnectError) throw error;
            throw new ConnectError(
                "media-denied",
                "Microphone access was denied",
            );
        }
        const peer = new RTCPeerConnection();
        for (const track of stream.getTracks()) peer.addTrack(track, stream);
        peer.createDataChannel("connect-control", { ordered: true });
        const socket = new WebSocket(signalingUrl, [
            "rvoip.webrtc.v1",
            `token.${credential}`,
        ]);
        const session = new BrowserVoiceSession(peer, socket, stream);
        try {
            await session.negotiate(timeoutMs);
            session.watchEnd();
            return session;
        } catch (error) {
            await session.close();
            if (error instanceof ConnectError) throw error;
            throw new ConnectError(
                "webrtc-failed",
                "WebRTC negotiation failed",
            );
        }
    }

    /** Attach remote audio and report browser playback-policy/output failure. */
    async attach(element: HTMLMediaElement, timeoutMs = 15000): Promise<void> {
        timeoutMs = timeoutValue(timeoutMs);
        element.srcObject = this.remoteStream;
        element.autoplay = true;
        let timer: ReturnType<typeof setTimeout> | undefined;
        try {
            await Promise.race([
                element.play(),
                new Promise<never>((_, reject) => {
                    timer = setTimeout(
                        () =>
                            reject(
                                new ConnectError(
                                    "media-output-failed",
                                    "Remote audio playback timed out",
                                ),
                            ),
                        timeoutMs,
                    );
                }),
            ]);
        } catch {
            element.pause();
            element.srcObject = null;
            throw new ConnectError(
                "media-output-failed",
                "Remote audio playback could not start",
            );
        } finally {
            clearTimeout(timer);
        }
    }

    /** Inspect the negotiated path and audio counters without exposing SDP. */
    diagnostics(): Promise<VoiceDiagnostics> {
        return voiceDiagnostics(this.peer);
    }

    async close(): Promise<void> {
        if (this.closed) return;
        this.closed = true;
        if (this.socket.readyState === WebSocket.OPEN && this.connectionId) {
            await new Promise<void>((resolve) => {
                const finish = () => {
                    clearTimeout(timer);
                    this.socket.removeEventListener("message", message);
                    this.socket.removeEventListener("close", finish);
                    resolve();
                };
                const message = (event: MessageEvent) => {
                    if (typeof event.data !== "string") return;
                    try {
                        if ((JSON.parse(event.data) as Signal).type === "bye")
                            finish();
                    } catch {
                        // The negotiated session already owns protocol errors.
                    }
                };
                const timer = setTimeout(finish, 500);
                this.socket.addEventListener("message", message);
                this.socket.addEventListener("close", finish, { once: true });
                this.socket.send(
                    JSON.stringify({
                        type: "bye",
                        connection_id: this.connectionId,
                        request_id: crypto.randomUUID(),
                    }),
                );
            });
        }
        if (
            this.socket.readyState === WebSocket.OPEN ||
            this.socket.readyState === WebSocket.CONNECTING
        )
            this.socket.close(1000, "Connect voice ended");
        this.peer.close();
        for (const track of this.localStream.getTracks()) track.stop();
        this.resolveEnded();
    }

    private watchEnd(): void {
        this.socket.onclose = () => void this.close();
        this.socket.onerror = () => void this.close();
        this.socket.onmessage = (event) => {
            if (typeof event.data !== "string") return;
            try {
                const signal = JSON.parse(event.data) as Signal;
                if (
                    signal.type === "bye" &&
                    signal.connection_id === this.connectionId
                )
                    void this.close();
            } catch {
                void this.close();
            }
        };
        this.peer.addEventListener("connectionstatechange", () => {
            if (["failed", "closed"].includes(this.peer.connectionState))
                void this.close();
        });
    }

    private async negotiate(timeoutMs: number): Promise<void> {
        const requestId = crypto.randomUUID();
        const offer = await this.peer.createOffer();
        await this.peer.setLocalDescription(offer);
        await waitForIce(this.peer, timeoutMs);
        const local = this.peer.localDescription;
        if (!local?.sdp || local.sdp.length > 1024 * 1024)
            throw new ConnectError("protocol", "Invalid local WebRTC offer");
        await new Promise<void>((resolve, reject) => {
            const timer = setTimeout(
                () =>
                    reject(
                        new ConnectError(
                            "timeout",
                            "WebRTC signaling timed out",
                        ),
                    ),
                timeoutMs,
            );
            let answerApplied = false;
            let readyReceived = false;
            const finishIfReady = () => {
                if (!answerApplied || !readyReceived) return;
                clearTimeout(timer);
                resolve();
            };
            const fail = () => {
                clearTimeout(timer);
                reject(
                    new ConnectError(
                        "webrtc-failed",
                        "WebRTC signaling closed",
                    ),
                );
            };
            this.socket.onerror = fail;
            this.socket.onclose = fail;
            const sendOffer = () =>
                this.socket.send(
                    JSON.stringify({
                        type: "offer-ready",
                        sdp: local.sdp,
                        request_id: requestId,
                    }),
                );
            this.socket.onopen = sendOffer;
            this.socket.onmessage = async (event) => {
                try {
                    if (
                        typeof event.data !== "string" ||
                        event.data.length > 1024 * 1024
                    )
                        throw new Error();
                    const signal = JSON.parse(event.data) as Signal;
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
                    } else if (signal.type === "ready") {
                        readyReceived = true;
                        finishIfReady();
                    } else if (signal.type === "rejected") {
                        throw new ConnectError(
                            "forbidden",
                            "Voice invitation was rejected",
                        );
                    }
                } catch (error) {
                    clearTimeout(timer);
                    reject(error);
                }
            };
            // ICE gathering can outlast the WebSocket handshake. Install the
            // answer handler before sending, including when `open` already ran.
            if (this.socket.readyState === WebSocket.OPEN) sendOffer();
            else if (this.socket.readyState !== WebSocket.CONNECTING) fail();
        });
    }
}

async function microphoneStream(timeoutMs: number): Promise<MediaStream> {
    const pending = navigator.mediaDevices.getUserMedia({ audio: true });
    return new Promise((resolve, reject) => {
        let expired = false;
        const timer = setTimeout(() => {
            expired = true;
            reject(new ConnectError("timeout", "Microphone access timed out"));
        }, timeoutMs);
        pending.then(
            (stream) => {
                if (expired) {
                    for (const track of stream.getTracks()) track.stop();
                    return;
                }
                clearTimeout(timer);
                resolve(stream);
            },
            (error) => {
                if (expired) return;
                clearTimeout(timer);
                reject(error);
            },
        );
    });
}

async function waitForIce(
    peer: RTCPeerConnection,
    timeoutMs: number,
): Promise<void> {
    if (peer.iceGatheringState === "complete") return;
    await new Promise<void>((resolve, reject) => {
        const timer = setTimeout(
            () =>
                reject(new ConnectError("timeout", "ICE gathering timed out")),
            timeoutMs,
        );
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
