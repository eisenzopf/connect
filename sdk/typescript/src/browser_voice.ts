import { BrowserDeadline } from "./browser_deadline.js";
import { negotiateBrowser, type BrowserSignal } from "./browser_negotiation.js";
import {
    browserOfferSelection,
    type BrowserSessionSelection,
} from "./browser_binding.js";
import {
    microphoneStream,
    captureDevices,
    type BrowserCaptureDevices,
} from "./browser_capture.js";
import { ConnectError } from "./errors.js";
import { timeoutValue } from "./protocol.js";
import {
    voiceDiagnostics,
    type VoiceDiagnostics,
} from "./browser_voice_stats.js";

import {
    constrainVideoCapture,
    startVideoTransform,
    stopVideoWorker,
    configureVideoSender,
} from "./browser_video_setup.js";
import {
    videoDiagnostics,
    type VideoDiagnostics,
} from "./browser_video_stats.js";
export type { VideoDiagnostics } from "./browser_video_stats.js";

/** Optional audio supplied by an application, such as processed or generated speech. */
export interface BrowserVoiceOptions
    extends BrowserSessionSelection, BrowserCaptureDevices {
    /**
     * A live stream matching the selected profile: one audio track by default,
     * or one audio and one video track with video=true. The session owns and
     * stops its tracks on failure/hangup. Clone tracks first to retain a source.
     * Omit this to acquire the user's microphone.
     */
    inputStream?: MediaStream;
    /** Opt into the VP8/640×360 audio/video profile. Audio-only remains the default. */
    video?: boolean;
}

/** One authenticated browser WebRTC leg retained by its signaling lease. */
export class BrowserVoiceSession {
    readonly remoteStream = new MediaStream();
    private connectionId?: string;
    private closed = false;
    private readonly captureEvents = new AbortController();
    private closing?: Promise<void>;
    private resolveEnded: () => void = () => {};
    /** Settles after either side ends this leg and microphone capture stops. */
    readonly ended = new Promise<void>((resolve) => {
        this.resolveEnded = resolve;
    });

    private constructor(
        private readonly peer: RTCPeerConnection,
        private readonly socket: WebSocket,
        private readonly localStream: MediaStream,
        private readonly videoWorker?: Worker,
    ) {
        for (const track of localStream.getTracks())
            track.addEventListener("ended", () => void this.close(), {
                once: true,
                signal: this.captureEvents.signal,
            });
        peer.ontrack = (event) => {
            for (const track of event.streams[0]?.getTracks() ?? [event.track])
                this.remoteStream.addTrack(track);
        };
    }

    static async open(
        signalingUrl: string,
        credential: string,
        timeoutMs = 15000,
        options: BrowserVoiceOptions = {},
    ): Promise<BrowserVoiceSession> {
        timeoutMs = timeoutValue(timeoutMs);
        if (options.video !== undefined && typeof options.video !== "boolean")
            throw new ConnectError(
                "invalid-request",
                "Select a boolean media profile",
            );
        const video = options.video === true;
        const inputStream = options.inputStream;
        const devices = captureDevices(
            options,
            video,
            inputStream !== undefined,
        );
        const selection = browserOfferSelection(options, video);
        const budget = new BrowserDeadline(timeoutMs);
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
            if (inputStream) {
                const tracks = inputStream.getTracks();
                if (
                    tracks.filter((track) => track.kind === "audio").length !==
                        1 ||
                    tracks.filter((track) => track.kind === "video").length !==
                        (video ? 1 : 0) ||
                    tracks.some((track) => track.readyState !== "live")
                )
                    throw new ConnectError(
                        "invalid-request",
                        "Input tracks do not match the selected audio/video profile",
                    );
                stream = inputStream;
            } else
                stream = await microphoneStream(
                    budget.remaining(),
                    video,
                    devices,
                );
        } catch (error) {
            if (error instanceof ConnectError) throw error;
            throw new ConnectError(
                "media-denied",
                "Microphone or camera access was denied",
            );
        }
        let peer: RTCPeerConnection | undefined;
        let socket: WebSocket | undefined;
        let session: BrowserVoiceSession | undefined;
        let videoWorker: Worker | undefined;
        try {
            // Chromium posts an encoded-stream bypass task when addTrack creates
            // a sender. Finish capture awaits before creating that sender, then
            // attach its transform synchronously in the same task. See
            // https://raw.githubusercontent.com/chromium/chromium/main/third_party/blink/renderer/modules/peerconnection/rtc_rtp_sender.cc
            // Constructor:667-672; bypass:1064-1074.
            if (video) await constrainVideoCapture(stream, budget.remaining());
            peer = new RTCPeerConnection();
            for (const track of stream.getTracks())
                peer.addTrack(track, stream);
            if (video) videoWorker = startVideoTransform(peer);
            peer.createDataChannel("connect-control", { ordered: true });
            socket = new WebSocket(signalingUrl, [
                "rvoip.webrtc.v1",
                `token.${credential}`,
            ]);
            session = new BrowserVoiceSession(
                peer,
                socket,
                stream,
                videoWorker,
            );
            if (videoWorker) {
                const owned = session;
                videoWorker.onerror = () => void owned.close();
                videoWorker.onmessage = () => void owned.close();
            }
            await budget.run(() =>
                negotiateBrowser(
                    peer!,
                    socket!,
                    stream,
                    budget,
                    (id) => {
                        session!.connectionId = id;
                    },
                    selection,
                ),
            );
            if (video) await budget.run(() => configureVideoSender(peer!));
            if (session.closed)
                throw new ConnectError(
                    "unsupported-media-profile",
                    "Video encoder support failed",
                );
            session.watchEnd();
            return session;
        } catch (error) {
            if (session) await session.close();
            else {
                socket?.close();
                peer?.close();
                stopVideoWorker(videoWorker);
                for (const track of stream.getTracks()) track.stop();
            }
            if (error instanceof ConnectError) throw error;
            throw new ConnectError(
                "webrtc-failed",
                "WebRTC negotiation failed",
            );
        }
    }

    /** Local capture for a muted preview; this session still owns its tracks. */
    get localPreview(): MediaStream {
        return this.localStream;
    }

    /** Independently mute a captured kind without ending the durable Session. */
    setMuted(kind: "audio" | "video", muted: boolean): void {
        for (const track of this.localStream
            .getTracks()
            .filter((t) => t.kind === kind))
            track.enabled = !muted;
    }

    /** Video decode counters, dimensions and timestamps; no SDP or addresses. */
    async videoDiagnostics(): Promise<Array<VideoDiagnostics>> {
        return videoDiagnostics(this.peer, this.videoWorker);
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

    close(): Promise<void> {
        if (this.closing) return this.closing;
        this.closed = true;
        this.closing = Promise.resolve().then(() => this.release());
        return this.closing;
    }

    private async release(): Promise<void> {
        this.captureEvents.abort();
        try {
            if (
                this.socket.readyState === WebSocket.OPEN &&
                this.connectionId
            ) {
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
                            if (
                                (JSON.parse(event.data) as BrowserSignal)
                                    .type === "bye"
                            )
                                finish();
                        } catch {
                            // The negotiated session already owns protocol errors.
                        }
                    };
                    const timer = setTimeout(finish, 500);
                    this.socket.addEventListener("message", message);
                    this.socket.addEventListener("close", finish, {
                        once: true,
                    });
                    try {
                        this.socket.send(
                            JSON.stringify({
                                type: "bye",
                                connection_id: this.connectionId,
                                request_id: crypto.randomUUID(),
                            }),
                        );
                    } catch {
                        finish();
                    }
                });
            }
        } finally {
            try {
                if (
                    this.socket.readyState === WebSocket.OPEN ||
                    this.socket.readyState === WebSocket.CONNECTING
                )
                    this.socket.close(1000, "Connect voice ended");
            } finally {
                try {
                    this.peer.close();
                } finally {
                    stopVideoWorker(this.videoWorker);
                    for (const track of this.localStream.getTracks())
                        track.stop();
                    this.resolveEnded();
                }
            }
        }
    }

    private watchEnd(): void {
        this.socket.onclose = () => void this.close();
        this.socket.onerror = () => void this.close();
        this.socket.onmessage = (event) => {
            if (typeof event.data !== "string") return;
            try {
                const signal = JSON.parse(event.data) as BrowserSignal;
                if (
                    signal.type === "bye" &&
                    signal.connection_id === this.connectionId
                )
                    void this.close();
            } catch {
                void this.close();
            }
        };
        this.peer.addEventListener("iceconnectionstatechange", () => {
            if (this.peer.iceConnectionState === "failed") void this.close();
        });
        this.peer.addEventListener("connectionstatechange", () => {
            if (["failed", "closed"].includes(this.peer.connectionState))
                void this.close();
        });
    }
}
