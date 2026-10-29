import { BrowserDeadline } from "./browser_deadline.js";
import { BrowserTasks } from "./browser_tasks.js";
import { CanonicalWebRtcSignaling } from "./browser_webrtc_signaling.js";
import { browserIceConfiguration } from "./browser_ice.js";
import { captureDevices, validateBrowserAnswer } from "./browser_capture.js";
import { configureReceiverBuffer } from "./browser_receiver.js";
import { startVideoTransform, stopVideoWorker, constrainVideoCapture, configureVideoSender, } from "./browser_video_setup.js";
import { ConnectError } from "./errors.js";
const unsupported = () => new ConnectError("unsupported-media-profile", "The original WebRTC media profile is unavailable");
const closed = () => new ConnectError("closed", "WebRTC media owner retired");
function timeout(options) {
    const n = options.timeoutMs ?? 10000;
    if (!Number.isFinite(n) || n < 1 || n > 60000)
        throw new ConnectError("invalid-request", "Use a bounded media deadline");
    return n;
}
/** One current physical's RTC owner. No capture permission is requested by offer. */
export class BrowserRtcConnection {
    client;
    input;
    tasks = new BrowserTasks();
    stopped = false;
    stop = new AbortController();
    retired = new Set();
    senders = new Map();
    remote = new Map();
    signaling;
    worker;
    stopControl;
    stopEvents;
    captureOwner;
    renderOwner;
    peer;
    constructor(client, input, participantId, configuration) {
        this.client = client;
        this.input = input;
        const video = input.video ?? input.screen;
        if (client.closed)
            throw closed();
        if (client.controlTransport !== "websocket" ||
            typeof RTCPeerConnection !== "function" ||
            typeof RTCRtpSender === "undefined" ||
            typeof RTCRtpSender.getCapabilities !== "function" ||
            !input.audio ||
            (input.screen !== undefined && input.video !== undefined) ||
            input.data?.length)
            throw unsupported();
        const ice = browserIceConfiguration(configuration.iceServers, configuration.iceTransportPolicy);
        this.peer = new RTCPeerConnection(ice);
        try {
            if (ice?.iceTransportPolicy === "relay" &&
                this.peer.getConfiguration().iceTransportPolicy !== "relay")
                throw unsupported();
            const audio = this.peer.addTransceiver("audio", {
                direction: "sendrecv",
            });
            const audioCodecs = RTCRtpSender.getCapabilities("audio")?.codecs.filter((v) => v.mimeType.toLowerCase() === "audio/opus");
            if (!audioCodecs?.length)
                throw unsupported();
            audio.setCodecPreferences(audioCodecs);
            this.senders.set(input.audio, audio);
            if (video) {
                const transceiver = this.peer.addTransceiver("video", {
                    direction: "sendrecv",
                    sendEncodings: [
                        {
                            maxFramerate: 30,
                            maxBitrate: 1_000_000,
                            scaleResolutionDownBy: 1,
                        },
                    ],
                });
                const encoding = transceiver.sender.getParameters().encodings;
                if (transceiver.sender.track !== null ||
                    encoding?.length !== 1 ||
                    encoding[0].maxFramerate !== 30 ||
                    encoding[0].maxBitrate !== 1_000_000 ||
                    encoding[0].scaleResolutionDownBy !== 1)
                    throw unsupported();
                this.senders.set(video, transceiver);
                this.worker = startVideoTransform(this.peer, transceiver);
                this.worker.onerror = () => this.close(unsupported());
                this.worker.onmessage = () => this.close(unsupported());
            }
            configureReceiverBuffer(this.peer);
            this.peer.ontrack = (event) => {
                if (this.stopped)
                    return;
                const selected = [...this.senders].find(([, v]) => v === event.transceiver);
                if (!selected ||
                    event.track.kind !==
                        (selected[0] === input.audio ? "audio" : "video") ||
                    (this.remote.has(selected[0]) &&
                        this.remote.get(selected[0]) !== event.track)) {
                    this.close(new ConnectError("protocol", "Remote track exceeds original Connection scope"));
                    return;
                }
                this.remote.set(selected[0], event.track);
            };
            this.signaling = new CanonicalWebRtcSignaling(client, this.peer, {
                conversationId: input.conversationId,
                sessionId: input.sessionId,
                connectionId: input.connectionId,
                operationId: input.operationId,
                expectedRevision: 0,
                participantId,
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
                                params: { sample_rate: 90000, channels: 0 },
                            },
                        ]
                        : [],
                    max_streams_per_connection: video ? 2 : 1,
                    identity_assurance_offered: "identified",
                },
                streams: [
                    {
                        id: input.audio,
                        kind: "audio",
                        direction: "sendrecv",
                        codec_preferences: ["opus"],
                    },
                    ...(video
                        ? [
                            {
                                id: video,
                                kind: input.screen
                                    ? "screen"
                                    : "video",
                                direction: "sendrecv",
                                codec_preferences: ["vp8"],
                            },
                        ]
                        : []),
                ],
            });
            void this.signaling.finished.then(() => this.close(), (error) => this.close(error instanceof Error ? error : closed()));
            this.stopControl = client.onClose((error) => this.close(error));
            this.stopEvents = client.onEvent((frame) => {
                if (frame.cid === input.conversationId &&
                    (frame.type === "conversation.closed" ||
                        (frame.sid === input.sessionId &&
                            (frame.type === "session.ended" ||
                                (frame.type === "connection.end" &&
                                    frame.connid === input.connectionId)))))
                    this.close();
            });
        }
        catch (error) {
            this.close();
            throw error;
        }
    }
    static begin(client, input, participantId, configuration, options) {
        const budget = new BrowserDeadline(timeout(options));
        if (options.signal?.aborted)
            throw new ConnectError("cancelled", "Media offer cancelled before preparation");
        const captured = { ...input, data: input.data?.map((v) => ({ ...v })) };
        const owner = new BrowserRtcConnection(client, captured, participantId, configuration);
        const signal = AbortSignal.any([
            owner.stop.signal,
            ...(options.signal ? [options.signal] : []),
        ]);
        const accepted = owner
            .signaling.offer({ timeoutMs: budget.remaining(), signal })
            .then(() => {
            owner.check();
            validateBrowserAnswer(owner.peer.remoteDescription?.sdp ?? "", !!(input.video ?? input.screen));
        });
        const ready = accepted
            .then(async () => {
            await owner.connected(budget, signal);
            owner.check();
            return owner;
        })
            .catch((error) => {
            owner.close(error instanceof Error ? error : closed());
            throw error;
        });
        void accepted.catch(() => { });
        void ready.catch(() => { });
        return {
            accepted,
            ready,
            owner,
            get submissionAttempted() {
                return (owner.signaling?.savedOffer?.submissionAttempted ?? false);
            },
        };
    }
    check() {
        if (this.stopped || this.client.closed)
            throw closed();
    }
    async connected(budget, signal) {
        await budget.run(() => new Promise((resolve, reject) => {
            const check = () => {
                if (signal.aborted ||
                    this.stopped ||
                    ["failed", "closed"].includes(this.peer.connectionState)) {
                    cleanup();
                    reject(closed());
                }
                else if (this.peer.connectionState === "connected") {
                    cleanup();
                    resolve();
                }
            };
            const cleanup = () => {
                this.peer.removeEventListener("connectionstatechange", check);
                signal.removeEventListener("abort", check);
            };
            this.peer.addEventListener("connectionstatechange", check);
            signal.addEventListener("abort", check, { once: true });
            check();
        }), signal);
    }
    onClose(fn) {
        return this.onRetire(fn);
    }
    onRetire(fn) {
        this.check();
        if (this.retired.size >= 16)
            throw unsupported();
        this.retired.add(fn);
        return () => this.retired.delete(fn);
    }
    async capture(input, options) {
        this.check();
        if (options.signal?.aborted)
            throw new ConnectError("cancelled", "Capture cancelled before permission");
        if (this.captureOwner && !this.captureOwner.closed)
            throw new ConnectError("conflict", "Capture already owned");
        const selected = { ...input }, audio = !!selected.audio, video = selected.video !== undefined;
        if ((!audio && !video) ||
            (audio && !this.input.audio) ||
            (video &&
                !(selected.video === "screen"
                    ? this.input.screen
                    : this.input.video)))
            throw unsupported();
        const devices = captureDevices(selected, video, !!selected.stream);
        if (selected.video === "screen" && selected.videoDeviceId !== undefined)
            throw new ConnectError("invalid-request", "Screen choice requires browser permission");
        const budget = new BrowserDeadline(timeout(options)), abort = new AbortController(), signal = AbortSignal.any([
            abort.signal,
            this.stop.signal,
            ...(options.signal ? [options.signal] : []),
        ]);
        let stopped = false, paused = false, replacing = false;
        const streams = new Set(), enabled = new Map(), events = new AbortController();
        let detach = () => { };
        const close = (error) => {
            if (stopped)
                return;
            stopped = true;
            abort.abort();
            detach();
            events.abort();
            document.removeEventListener("visibilitychange", visibility);
            for (const stream of streams)
                for (const track of stream.getTracks()) {
                    track.stop();
                }
            streams.clear();
            enabled.clear();
            if (this.captureOwner === handle)
                this.captureOwner = undefined;
            selected.state?.("closed", error);
        };
        const pause = () => {
            if (stopped || paused)
                return;
            paused = true;
            for (const stream of streams)
                for (const track of stream.getTracks()) {
                    enabled.set(track, track.enabled);
                    track.enabled = false;
                }
            selected.state?.("paused");
        };
        const visibility = () => {
            if (document.visibilityState === "hidden")
                pause();
        };
        const handle = Object.freeze({
            get closed() {
                return stopped;
            },
            pause,
            resume: async () => {
                this.check();
                if (stopped)
                    throw closed();
                if (document.visibilityState === "hidden")
                    return;
                for (const [track, was] of enabled)
                    track.enabled = was;
                enabled.clear();
                paused = false;
                selected.state?.("active");
            },
            close: () => close(),
        });
        const cancelled = () => close(new ConnectError("cancelled", "Capture cancelled"));
        signal.addEventListener("abort", cancelled, { once: true });
        const own = (stream) => {
            if (stopped || signal.aborted) {
                for (const track of stream.getTracks())
                    track.stop();
                throw closed();
            }
            streams.add(stream);
        };
        const permission = async (work) => {
            void work.then((stream) => {
                if (stopped || signal.aborted)
                    for (const track of stream.getTracks())
                        track.stop();
            }, () => { });
            own(await budget.run(() => work, signal));
        };
        try {
            detach = this.onRetire(close);
            this.captureOwner = handle;
            if (signal.aborted)
                cancelled();
            if (stopped)
                throw closed();
            if (selected.stream)
                own(selected.stream);
            else {
                if (selected.video === "screen")
                    await permission(navigator.mediaDevices.getDisplayMedia({
                        video: {
                            width: { ideal: 640, max: 640 },
                            height: { ideal: 360, max: 360 },
                            frameRate: { ideal: 30, max: 30 },
                        },
                        audio: false,
                    }));
                if (audio || selected.video === "camera")
                    await permission(navigator.mediaDevices.getUserMedia({
                        audio: audio
                            ? {
                                sampleRate: { ideal: 48000 },
                                channelCount: { ideal: 1, max: 2 },
                                ...(devices.audioDeviceId
                                    ? {
                                        deviceId: {
                                            exact: devices.audioDeviceId,
                                        },
                                    }
                                    : {}),
                            }
                            : false,
                        video: selected.video === "camera"
                            ? {
                                width: { ideal: 640, max: 640 },
                                height: { ideal: 360, max: 360 },
                                frameRate: { ideal: 30, max: 30 },
                                ...(devices.videoDeviceId
                                    ? {
                                        deviceId: {
                                            exact: devices.videoDeviceId,
                                        },
                                    }
                                    : {}),
                            }
                            : false,
                    }));
            }
            const tracks = [...streams].flatMap((stream) => stream.getTracks());
            if (tracks.length !== Number(audio) + Number(video) ||
                tracks.filter((t) => t.kind === "audio").length !==
                    Number(audio) ||
                tracks.filter((t) => t.kind === "video").length !==
                    Number(video) ||
                tracks.some((t) => t.readyState !== "live"))
                throw unsupported();
            if (video) {
                const stream = [...streams].find((v) => v.getVideoTracks().length);
                await budget.run(() => constrainVideoCapture(stream, budget.remaining()), signal);
            }
            for (const track of tracks) {
                track.addEventListener("ended", () => close(closed()), {
                    once: true,
                    signal: events.signal,
                });
                const id = track.kind === "audio"
                    ? this.input.audio
                    : selected.video === "screen"
                        ? this.input.screen
                        : this.input.video;
                replacing = true;
                await budget.run(() => this.tasks.track(this.senders.get(id).sender.replaceTrack(track)), signal);
                replacing = false;
                if (stopped || signal.aborted)
                    throw closed();
            }
            if (video)
                await budget.run(() => configureVideoSender(this.peer), signal);
            this.check();
            if (stopped)
                throw closed();
            document.addEventListener("visibilitychange", visibility);
            visibility();
            selected.state?.(paused ? "paused" : "active");
            return handle;
        }
        catch (error) {
            const failure = error instanceof ConnectError
                ? error
                : new ConnectError(error instanceof DOMException &&
                    error.name === "NotAllowedError"
                    ? "media-denied"
                    : "media", "Owned WebRTC capture failed");
            close(failure);
            if (replacing)
                this.close(failure);
            throw failure;
        }
        finally {
            if (stopped)
                signal.removeEventListener("abort", cancelled);
        }
    }
    async render(input, options) {
        this.check();
        if (options.signal?.aborted)
            throw new ConnectError("cancelled", "Playback cancelled before startup");
        if (this.renderOwner && !this.renderOwner.diagnostics.closed)
            throw new ConnectError("conflict", "Renderer already owned");
        const volume = input.volume ?? 1, audio = input.audio !== false, video = { ...input.video };
        if (!Number.isFinite(volume) ||
            volume < 0 ||
            volume > 1 ||
            Object.keys(video).length > 8)
            throw new ConnectError("invalid-request", "Use bounded render options");
        const budget = new BrowserDeadline(timeout(options)), stop = new AbortController(), signal = AbortSignal.any([
            stop.signal,
            this.stop.signal,
            ...(options.signal ? [options.signal] : []),
        ]);
        let stopped = false, paused = false, count = 0, context, source, gain, playout, detach = () => { };
        const outputs = [];
        let finish, fail;
        const finished = new Promise((yes, no) => {
            finish = yes;
            fail = no;
        });
        void finished.catch(() => { });
        const close = (error) => {
            if (stopped)
                return;
            stopped = true;
            detach();
            stop.abort();
            document.removeEventListener("visibilitychange", visibility);
            if (playout) {
                playout.pause();
                playout.srcObject = null;
            }
            for (const output of outputs) {
                if (output.callback !== undefined)
                    output.element.cancelVideoFrameCallback(output.callback);
                output.element.pause();
                output.element.srcObject = null;
            }
            source?.disconnect();
            gain?.disconnect();
            if (this.renderOwner === handle)
                this.renderOwner = undefined;
            if (context) {
                const cleanup = context.close();
                this.tasks.track(cleanup, true);
                void cleanup.then(() => (error ? fail(error) : finish()), () => fail(new ConnectError("media", "Playback graph cleanup failed")));
            }
            else
                error ? fail(error) : finish();
        };
        const pause = () => {
            if (stopped || paused)
                return;
            paused = true;
            void context?.suspend().catch(() => { });
            playout?.pause();
            for (const output of outputs)
                output.element.pause();
        };
        const visibility = () => {
            if (document.visibilityState === "hidden")
                pause();
        };
        const handle = Object.freeze({
            finished,
            get diagnostics() {
                return Object.freeze({
                    closed: stopped,
                    paused,
                    audio: 0,
                    video: count,
                });
            },
            pause,
            resume: async () => {
                this.check();
                if (stopped)
                    throw closed();
                if (document.visibilityState === "hidden")
                    return;
                const resuming = new BrowserDeadline(timeout(options));
                try {
                    if (context)
                        await resuming.run(() => context.resume(), signal);
                    await playAudio(resuming);
                }
                catch (error) {
                    const failure = error instanceof Error ? error : closed();
                    close(failure);
                    throw failure;
                }
                for (const output of outputs)
                    void output.element.play().catch(() => {
                        if (!stopped)
                            close(new ConnectError("media", "Playback could not resume"));
                    });
                paused = false;
            },
            close: () => close(),
        });
        const cancelled = () => close(new ConnectError("cancelled", "Playback cancelled"));
        const playAudio = (deadline) => playout
            ? deadline.run(async () => {
                try {
                    await playout.play();
                }
                catch {
                    throw new ConnectError("media", "Audio playback failed");
                }
            }, signal)
            : Promise.resolve();
        signal.addEventListener("abort", cancelled, { once: true });
        try {
            detach = this.onRetire(close);
            this.renderOwner = handle;
            if (signal.aborted)
                cancelled();
            if (stopped)
                throw closed();
            if (audio) {
                const track = this.remote.get(this.input.audio);
                if (!track || track.readyState !== "live")
                    throw unsupported();
                context = new AudioContext({
                    sampleRate: 48000,
                    latencyHint: "interactive",
                });
                source = context.createMediaStreamSource(new MediaStream([track]));
                gain = context.createGain();
                gain.gain.value = volume;
                source.connect(gain);
                gain.connect(context.destination);
                // Chrome starts the RTC decoder through this owned muted sink.
                // The Web Audio graph controls audible playback and volume.
                playout = document.createElement("audio");
                playout.muted = true;
                playout.srcObject = new MediaStream([track]);
                await budget.run(() => context.resume(), signal);
                await playAudio(budget);
            }
            for (const [id, canvas] of Object.entries(video)) {
                if (id !== (this.input.video ?? this.input.screen))
                    throw new ConnectError("invalid-request", "Render this Connection's receive Stream");
                const track = this.remote.get(id);
                if (!track || track.readyState !== "live")
                    throw unsupported();
                canvas.width = 640;
                canvas.height = 360;
                const out = canvas.getContext("2d");
                if (!out)
                    throw unsupported();
                const element = document.createElement("video");
                element.muted = true;
                element.autoplay = true;
                element.playsInline = true;
                element.srcObject = new MediaStream([track]);
                const output = {
                    element,
                    canvas: out,
                    callback: undefined,
                };
                outputs.push(output);
                const paint = () => {
                    if (stopped)
                        return;
                    if (!paused && element.readyState >= 2) {
                        out.drawImage(element, 0, 0, 640, 360);
                        count++;
                    }
                    output.callback = element.requestVideoFrameCallback(paint);
                };
                output.callback = element.requestVideoFrameCallback(paint);
                void element.play().catch(() => {
                    if (!stopped)
                        close(new ConnectError("media", "Video playback failed"));
                });
            }
            this.check();
            if (stopped)
                throw closed();
            document.addEventListener("visibilitychange", visibility);
            visibility();
            return handle;
        }
        catch (error) {
            const failure = error instanceof Error
                ? error
                : new ConnectError("media", "Playback startup failed");
            close(failure);
            throw failure;
        }
    }
    close(error) {
        if (this.stopped)
            return;
        this.stopped = true;
        this.stopControl?.();
        this.stopEvents?.();
        for (const fn of [...this.retired])
            try {
                fn(error);
            }
            catch {
                /* Retire every owner. */
            }
        this.retired.clear();
        this.stop.abort();
        this.peer.ontrack = null;
        this.signaling?.close();
        this.peer.close();
        if (this.worker)
            stopVideoWorker(this.worker);
        for (const track of this.remote.values())
            track.stop();
        this.remote.clear();
    }
    async shutdown() {
        this.close();
        await this.signaling?.shutdown();
        await this.tasks.join();
    }
}
