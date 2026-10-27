import { BrowserDeadline } from "./browser_deadline.js";
import { browserOfferSelection, } from "./browser_binding.js";
import { CanonicalWebRtcSignaling, } from "./browser_webrtc_signaling.js";
import { microphoneStream, captureDevices, validateBrowserAnswer, } from "./browser_capture.js";
import { timeoutValue } from "./protocol.js";
import { ConnectError } from "./errors.js";
import { identifier } from "./uctp_wire.js";
import { voiceDiagnostics, } from "./browser_voice_stats.js";
import { constrainVideoCapture, startVideoTransform, stopVideoWorker, configureVideoSender, } from "./browser_video_setup.js";
import { videoDiagnostics, } from "./browser_video_stats.js";
/** An unsuccessful open retains its original offer for authoritative status recovery.
 * This snapshot can contain private SDP; keep it in application-owned private state. */
export class BrowserVoiceOpenError extends ConnectError {
    savedOffer;
    constructor(error, savedOffer) {
        super(error instanceof ConnectError ? error.code : "webrtc-failed", error instanceof ConnectError
            ? error.message
            : "WebRTC participation failed");
        this.savedOffer = savedOffer;
    }
}
/** Owns capture, the RTC peer and video worker; borrows the shared signed client.
 * open() establishes signaling only. Observe actual media/playback independently. */
export class BrowserVoiceSession {
    client;
    selection;
    remoteStream = new MediaStream();
    closed = false;
    failure;
    captureEvents = new AbortController();
    closing;
    resolveEnded;
    /** Settles after local media resources retire. It is not a durable end receipt. */
    ended = new Promise((resolve) => {
        this.resolveEnded = resolve;
    });
    peer;
    localStream;
    ownedTracks = [];
    videoWorker;
    signaling;
    stopLifetime;
    stopEvents;
    endIntent;
    ending;
    constructor(client, selection) {
        this.client = client;
        this.selection = selection;
        this.stopLifetime = client.onClose((error) => this.fail(error));
        if (this.closed) {
            this.stopLifetime();
            return;
        }
        try {
            this.stopEvents = client.onEvent((frame) => {
                if ((frame.type === "conversation.closed" &&
                    frame.cid === selection.conversationId) ||
                    (frame.type === "session.ended" &&
                        frame.cid === selection.conversationId &&
                        frame.sid === selection.sessionId) ||
                    (frame.type === "connection.end" &&
                        frame.cid === selection.conversationId &&
                        frame.sid === selection.sessionId &&
                        frame.connid === selection.connectionId))
                    void this.close();
            });
            if (this.closed)
                this.stopEvents();
        }
        catch (error) {
            this.stopLifetime?.();
            throw error;
        }
    }
    check() {
        if (this.closed)
            throw (this.failure ??
                new ConnectError("closed", "Browser media owner closed"));
    }
    fail(error) {
        if (this.closed)
            return;
        this.failure = error;
        void this.close();
    }
    async bound(work, budget) {
        const result = await budget.run(() => Promise.race([
            work,
            this.ended.then(() => {
                this.check();
                throw new ConnectError("closed", "Browser media owner closed");
            }),
        ]));
        this.check();
        return result;
    }
    static async open(client, options, request = {}) {
        const signal = request.signal;
        const timeoutMs = request.timeoutMs ?? 15000;
        if (!Number.isFinite(timeoutMs) || timeoutMs < 1 || timeoutMs > 60000)
            throw new ConnectError("invalid-request", "WebRTC deadline must be 1–60000 ms");
        if (client.controlTransport !== "websocket")
            throw new ConnectError("unsupported-transport", "Browser WebRTC requires an existing signed WebSocket client");
        if (client.closed)
            throw new ConnectError("closed", "Canonical control client closed");
        const selection = browserOfferSelection(options);
        const video = selection.videoStreamId !== undefined;
        const inputStream = options.inputStream;
        const devices = captureDevices(options, video, inputStream !== undefined);
        const tracks = inputStream?.getTracks();
        if (tracks &&
            (tracks.filter((t) => t.kind === "audio").length !== 1 ||
                tracks.filter((t) => t.kind === "video").length !==
                    (video ? 1 : 0) ||
                tracks.length !== (video ? 2 : 1) ||
                tracks.some((t) => t.readyState !== "live")))
            throw new ConnectError("invalid-request", "Input tracks do not match the selected audio/video profile");
        if (signal?.aborted)
            throw new ConnectError("cancelled", "Browser media open cancelled before capture");
        const budget = new BrowserDeadline(timeoutMs);
        let session;
        try {
            session = new BrowserVoiceSession(client, selection);
        }
        catch (error) {
            throw new BrowserVoiceOpenError(error);
        }
        const abort = () => session.fail(new ConnectError("cancelled", "Browser media open cancelled; retain original operation ID"));
        signal?.addEventListener("abort", abort, { once: true });
        try {
            if (signal?.aborted)
                abort();
            session.check();
            try {
                session.localStream =
                    inputStream ??
                        (await microphoneStream(budget.remaining(), video, devices, session.captureEvents.signal));
            }
            catch (error) {
                session.check();
                if (error instanceof ConnectError)
                    throw error;
                throw new ConnectError("media-denied", "Microphone or camera access was denied");
            }
            session.ownedTracks = [
                ...(tracks ?? session.localStream.getTracks()),
            ];
            if (session.ownedTracks.length !== (video ? 2 : 1) ||
                session.ownedTracks.filter((t) => t.kind === "audio").length !==
                    1 ||
                session.ownedTracks.filter((t) => t.kind === "video").length !==
                    (video ? 1 : 0))
                throw new ConnectError("unsupported-media-profile", "Acquired tracks do not match the original audio/video profile");
            // Capture can finish in the same microtask as control retirement.
            if (session.closed) {
                for (const track of session.ownedTracks)
                    track.stop();
                session.ownedTracks = [];
                session.check();
            }
            for (const track of session.ownedTracks) {
                track.addEventListener("ended", () => session.fail(new ConnectError("closed", "Owned capture ended")), { once: true, signal: session.captureEvents.signal });
                if (track.readyState !== "live")
                    session.fail(new ConnectError("closed", "Owned capture ended"));
            }
            session.check();
            if (video)
                await session.bound(constrainVideoCapture(session.localStream, budget.remaining()), budget);
            // Attach the encoded transform in the same task as addTrack. Awaiting
            // capture constraints first prevents Chromium's sender bypass task.
            session.peer = new RTCPeerConnection();
            for (const track of session.ownedTracks)
                session.peer.addTrack(track, session.localStream);
            if (video)
                session.videoWorker = startVideoTransform(session.peer);
            session.peer.ontrack = (event) => {
                if (session.closed)
                    return;
                const track = event.track, received = session.remoteStream.getTracks();
                if (!["audio", ...(video ? ["video"] : [])].includes(track.kind) ||
                    received.some((t) => t.kind === track.kind && t !== track)) {
                    session.fail(new ConnectError("protocol", "Remote tracks exceed the original media profile"));
                    return;
                }
                if (!received.includes(track))
                    session.remoteStream.addTrack(track);
            };
            session.signaling = new CanonicalWebRtcSignaling(client, session.peer, {
                ...selection,
                capabilities: {
                    audio_codecs: [
                        {
                            name: "opus",
                            params: { sample_rate: 48000, channels: 1 },
                        },
                    ],
                    video_codecs: video
                        ? [
                            {
                                name: "vp8",
                                params: {
                                    sample_rate: 90000,
                                    channels: 0,
                                },
                            },
                        ]
                        : [],
                    max_streams_per_connection: video ? 2 : 1,
                    identity_assurance_offered: "identified",
                },
                streams: [
                    {
                        id: selection.audioStreamId,
                        kind: "audio",
                        direction: "sendrecv",
                        codec_preferences: ["opus"],
                    },
                    ...(video
                        ? [
                            {
                                id: selection.videoStreamId,
                                kind: "video",
                                direction: "sendrecv",
                                codec_preferences: ["vp8"],
                            },
                        ]
                        : []),
                ],
            });
            void session.signaling.finished.then(() => session.close(), (error) => session.fail(error));
            if (session.videoWorker) {
                session.videoWorker.onerror = () => session.fail(new ConnectError("unsupported-media-profile", "Video encoder support failed"));
                session.videoWorker.onmessage = () => session.fail(new ConnectError("unsupported-media-profile", "Video encoder support failed"));
            }
            await session.bound(session.signaling.offer({
                timeoutMs: Math.max(1, budget.remaining()),
                signal,
            }), budget);
            validateBrowserAnswer(session.peer.remoteDescription?.sdp ?? "", video);
            if (video)
                await session.bound(configureVideoSender(session.peer), budget);
            session.check();
            return session;
        }
        catch (error) {
            await session.close();
            throw new BrowserVoiceOpenError(session.failure ?? error, session.signaling?.savedOffer);
        }
        finally {
            signal?.removeEventListener("abort", abort);
        }
    }
    /** A private snapshot of original SDP/operation intent, including uncertain submission. */
    get savedOffer() {
        return this.signaling?.savedOffer;
    }
    get savedEnd() {
        return this.endIntent
            ? {
                ...this.endIntent,
                scope: { ...this.endIntent.scope },
                payload: { ...this.endIntent.payload },
            }
            : undefined;
    }
    /** Local capture for a muted preview; this session still owns its tracks. */
    get localPreview() {
        return this.localStream;
    }
    /** Independently mute a captured kind without ending the durable Session. */
    setMuted(kind, muted) {
        for (const track of this.localStream.getTracks().filter((t) => t.kind === kind))
            track.enabled = !muted;
    }
    /** Video decode counters, dimensions and timestamps; no SDP or addresses. */
    async videoDiagnostics() {
        return videoDiagnostics(this.peer, this.videoWorker);
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
    /** Submit one explicit durable end, then observe its original journal status.
     * Repeating the same call shares its result; another operation/revision refuses.
     * Local capture retires even when the durable outcome remains uncertain. */
    end(operationId, expectedRevision, request = {}) {
        if (this.endIntent) {
            if (this.endIntent.scope.operation_id !== operationId ||
                this.endIntent.scope.expected_revision !== expectedRevision)
                return Promise.reject(new ConnectError("conflict", "Original end already attempted; inspect its original operation status"));
            return this.ending;
        }
        const signal = request.signal;
        const timeoutMs = request.timeoutMs ?? 15000;
        if (!identifier(operationId) ||
            operationId === this.selection.operationId ||
            !Number.isSafeInteger(expectedRevision) ||
            expectedRevision < 0 ||
            !Number.isFinite(timeoutMs) ||
            timeoutMs < 1 ||
            timeoutMs > 60000)
            return Promise.reject(new ConnectError("invalid-request", "Use a distinct stable end operation, current revision and bounded deadline"));
        if (signal?.aborted)
            return Promise.reject(new ConnectError("cancelled", "End cancelled before submission"));
        this.endIntent = {
            scope: {
                cid: this.selection.conversationId,
                sid: this.selection.sessionId,
                connid: this.selection.connectionId,
                operation_id: operationId,
                expected_revision: expectedRevision,
            },
            payload: { reason_code: 200, reason: "normal-clearing" },
            submissionAttempted: false,
        };
        const until = performance.now() + timeoutMs;
        const remaining = () => ({
            timeoutMs: Math.max(1, until - performance.now()),
            signal,
        });
        this.ending = Promise.resolve().then(async () => {
            try {
                this.endIntent.submissionAttempted = true;
                await this.client.request("connection.end", { ...this.endIntent.payload }, { ...this.endIntent.scope }, remaining());
                for (;;) {
                    if (performance.now() >= until)
                        throw new ConnectError("timeout", "Original Connection end remains uncertain; inspect its operation status");
                    const state = await this.client.operationStatus(this.selection.conversationId, operationId, remaining());
                    if (state.state === "completed")
                        return;
                    if (["failed", "cancelled"].includes(String(state.state)))
                        throw new ConnectError("refused", "Original Connection end did not complete");
                    if (performance.now() >= until)
                        throw new ConnectError("timeout", "Original Connection end remains uncertain; inspect its operation status");
                    await new Promise((resolve) => setTimeout(resolve, Math.max(0, Math.min(20, until - performance.now()))));
                }
            }
            finally {
                await this.close();
            }
        });
        return this.ending;
    }
    /** Local retirement only; never sends a durable end or closes the shared client. */
    close() {
        if (this.closing)
            return this.closing;
        this.closed = true;
        this.captureEvents.abort();
        this.closing = Promise.resolve().then(() => this.release());
        return this.closing;
    }
    release() {
        this.stopEvents?.();
        this.stopLifetime?.();
        if (this.peer)
            this.peer.ontrack = null;
        try {
            if (this.signaling)
                this.signaling.close();
            else
                this.peer?.close();
        }
        finally {
            try {
                stopVideoWorker(this.videoWorker);
            }
            finally {
                try {
                    for (const track of this.ownedTracks)
                        track.stop();
                }
                finally {
                    this.ownedTracks = [];
                    this.resolveEnded();
                }
            }
        }
    }
}
