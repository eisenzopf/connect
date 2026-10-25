import { captureDevices, } from "./browser_capture.js";
import { ConnectError } from "./errors.js";
export function browserCaptureSupport() {
    const media = typeof navigator === "undefined" ? undefined : navigator.mediaDevices;
    return {
        microphone: typeof media?.getUserMedia === "function",
        camera: typeof media?.getUserMedia === "function",
        screen: typeof media?.getDisplayMedia === "function",
        audioWorklet: typeof AudioContext === "function" &&
            typeof AudioWorkletNode === "function",
        videoFrames: typeof VideoFrame === "function" &&
            typeof HTMLVideoElement === "function" &&
            "requestVideoFrameCallback" in HTMLVideoElement.prototype,
    };
}
/** Owned device/screen capture only. Decoded playback belongs to the renderer. */
export class BrowserMediaCapture {
    connection;
    options;
    streams = new Set();
    enabled = new Map();
    ended = new Map();
    abort = new AbortController();
    until;
    context;
    input;
    worklet;
    silent;
    video;
    callback;
    timer;
    stopOwner;
    epoch = 0;
    stopped = false;
    failure;
    paused = false;
    visibility = () => {
        if (document.visibilityState === "hidden")
            this.pause();
    };
    cancelled = () => this.close(new ConnectError("cancelled", "Media capture cancelled"));
    constructor(connection, options, timeout) {
        this.connection = connection;
        this.options = options;
        this.until = performance.now() + timeout;
        this.stopOwner = connection.onClose(() => this.close());
        this.timer = setTimeout(() => this.close(new ConnectError("timeout", "Media capture deadline elapsed")), timeout);
        options.signal?.addEventListener("abort", this.cancelled, {
            once: true,
        });
        if (options.signal?.aborted)
            this.cancelled();
    }
    static async open(connection, options) {
        const timeout = options.timeoutMs ?? 10000;
        if (!Number.isFinite(timeout) ||
            timeout < 1 ||
            timeout > 60000 ||
            (!options.audioStreamId && !options.videoStreamId))
            throw new ConnectError("invalid-request", "Select negotiated capture Streams and a 1–60000 ms deadline");
        const audio = options.audioStreamId, video = options.videoStreamId;
        const bindings = connection.streams;
        for (const [id, kind] of [
            [audio, "audio"],
            [video, options.videoSource === "screen" ? "screen" : "video"],
        ]) {
            if (id &&
                !bindings.some((s) => s.streamId === id &&
                    s.kind === kind &&
                    s.direction !== "recvonly"))
                throw new ConnectError("invalid-request", "Capture must use this Connection's negotiated sender Streams");
        }
        const devices = captureDevices(options, !!video, !!options.stream);
        if (options.videoSource === "screen" &&
            options.videoDeviceId !== undefined)
            throw new ConnectError("invalid-request", "Screen selection uses explicit browser permission");
        const support = browserCaptureSupport();
        if ((audio && !support.audioWorklet) ||
            (video && !support.videoFrames) ||
            (!options.stream &&
                ((audio && !support.microphone) ||
                    (video &&
                        !(options.videoSource === "screen"
                            ? support.screen
                            : support.camera)))))
            throw new ConnectError("unsupported-media-profile", "Required capture APIs are unavailable");
        const capture = new BrowserMediaCapture(connection, options, timeout);
        try {
            if (capture.stopped)
                throw new ConnectError("cancelled", "Media capture cancelled before permission");
            if (options.stream)
                capture.own(options.stream);
            else {
                if (video && options.videoSource === "screen")
                    await capture.permission(navigator.mediaDevices.getDisplayMedia({
                        video: {
                            width: { ideal: 640, max: 640 },
                            height: { ideal: 360, max: 360 },
                            frameRate: { ideal: 30, max: 30 },
                        },
                        audio: false,
                    }));
                const camera = video && options.videoSource !== "screen";
                if (audio || camera)
                    await capture.permission(navigator.mediaDevices.getUserMedia({
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
                        video: camera
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
            const tracks = [...capture.streams].flatMap((s) => s.getTracks());
            if (tracks.filter((t) => t.kind === "audio").length !==
                (audio ? 1 : 0) ||
                tracks.filter((t) => t.kind === "video").length !==
                    (video ? 1 : 0) ||
                tracks.some((t) => t.readyState !== "live"))
                throw new ConnectError("unsupported-media-profile", "Capture did not supply the requested live tracks");
            if (audio)
                await capture.audio(tracks.find((t) => t.kind === "audio"));
            if (video)
                await capture.camera(tracks.find((t) => t.kind === "video"));
            document.addEventListener("visibilitychange", capture.visibility);
            capture.visibility();
            if (capture.stopped)
                throw new ConnectError("closed", "Capture retired during startup");
            clearTimeout(capture.timer);
            capture.timer = undefined;
            capture.options.state?.(capture.paused ? "paused" : "active");
            return capture;
        }
        catch (error) {
            const failure = error instanceof ConnectError
                ? error
                : new ConnectError(error instanceof DOMException &&
                    error.name === "NotAllowedError"
                    ? "media-denied"
                    : "media-capture", "Browser capture could not start");
            capture.close(failure);
            throw failure;
        }
    }
    own(stream) {
        if (this.stopped) {
            for (const t of stream.getTracks())
                t.stop();
            return;
        }
        this.streams.add(stream);
        for (const track of stream.getTracks()) {
            this.enabled.set(track, track.enabled);
            const ended = () => this.close(new ConnectError("media-ended", "Capture track ended"));
            this.ended.set(track, ended);
            track.addEventListener("ended", ended, { once: true });
        }
    }
    async permission(work) {
        void work.then((stream) => {
            if (this.stopped)
                for (const t of stream.getTracks())
                    t.stop();
        }, () => { });
        this.own(await this.bound(work));
    }
    bound(work, limit = this.until) {
        return new Promise((resolve, reject) => {
            const cancelled = () => {
                cleanup();
                reject(this.failure ??
                    new ConnectError("closed", "Capture retired"));
            };
            const timer = setTimeout(() => {
                cleanup();
                reject(new ConnectError("timeout", "Capture step timed out"));
            }, Math.max(0, limit - performance.now()));
            const cleanup = () => {
                clearTimeout(timer);
                this.abort.signal.removeEventListener("abort", cancelled);
            };
            this.abort.signal.addEventListener("abort", cancelled, {
                once: true,
            });
            if (this.stopped)
                cancelled();
            work.then((v) => {
                cleanup();
                resolve(v);
            }, (e) => {
                cleanup();
                reject(e);
            });
        });
    }
    resetClock() {
        this.epoch++;
        if (this.context && this.worklet)
            this.worklet.port.postMessage({
                type: "reset",
                epoch: this.epoch,
                originUs: performance.now() * 1000 -
                    this.context.currentTime * 1000000,
            });
    }
    async audio(track) {
        const context = (this.context = new AudioContext({
            sampleRate: 48000,
            latencyHint: "interactive",
        }));
        if (context.sampleRate !== 48000)
            throw new ConnectError("unsupported-media-profile", "Capture requires a 48 kHz audio graph");
        await this.bound(context.audioWorklet.addModule(new URL("./browser_audio_worklet.js", import.meta.url)));
        if (this.stopped)
            return;
        const node = (this.worklet = new AudioWorkletNode(context, "connect-pcm-48k", {
            channelCount: 1,
            channelCountMode: "explicit",
            numberOfInputs: 1,
            numberOfOutputs: 1,
            outputChannelCount: [1],
        }));
        this.resetClock();
        node.port.onmessage = (event) => {
            if (this.stopped)
                return;
            const data = event.data;
            try {
                if (data?.epoch !== this.epoch || this.paused)
                    return;
                if (!(data.samples instanceof Float32Array) ||
                    data.samples.length !== 960 ||
                    !Number.isSafeInteger(data.timestampUs))
                    throw new ConnectError("media-capture", "Invalid capture worklet frame");
                this.connection.encodeAudio(this.options.audioStreamId, data.samples, data.timestampUs);
            }
            catch (error) {
                this.close(error instanceof Error
                    ? error
                    : new ConnectError("media-capture", "Capture failed"));
            }
            finally {
                if (!this.stopped)
                    node.port.postMessage({
                        type: "credit",
                        epoch: data?.epoch,
                    });
            }
        };
        const input = (this.input = context.createMediaStreamSource(new MediaStream([track])));
        const silent = (this.silent = context.createGain());
        silent.gain.value = 0;
        input.connect(node);
        node.connect(silent);
        silent.connect(context.destination);
        await this.bound(context.resume());
        if (context.state !== "running")
            throw new ConnectError("media-playback", "Capture audio graph did not start");
        context.onstatechange = () => {
            if (this.stopped || context.state === "running")
                return;
            if (context.state === "closed")
                this.close(new ConnectError("media-capture", "Capture audio graph closed"));
            else
                this.pause();
        };
    }
    async camera(track) {
        const video = (this.video = document.createElement("video"));
        video.muted = true;
        video.playsInline = true;
        video.srcObject = new MediaStream([track]);
        const canvas = document.createElement("canvas");
        canvas.width = 640;
        canvas.height = 360;
        const out = canvas.getContext("2d");
        if (!out)
            throw new ConnectError("unsupported-media-profile", "Video capture canvas unavailable");
        await this.bound(video.play());
        let origin;
        const frame = (_now, metadata) => {
            if (this.stopped)
                return;
            try {
                if (!this.paused) {
                    // A callback may run a vsync or more after frame submission.
                    // Use the frame's clock metadata, never callback execution.
                    const captureTime = metadata.captureTime;
                    let timestamp;
                    if (captureTime !== undefined) {
                        timestamp = Math.round(captureTime * 1000);
                        if (!Number.isFinite(captureTime) || captureTime < 0)
                            throw new ConnectError("media-capture", "Invalid video capture clock");
                    }
                    else {
                        const presentationUs = Math.round(metadata.presentationTime * 1000);
                        const mediaUs = Math.round(metadata.mediaTime * 1000000);
                        if (!Number.isSafeInteger(presentationUs) ||
                            !Number.isSafeInteger(mediaUs) ||
                            !Number.isFinite(metadata.presentationTime) ||
                            metadata.presentationTime < 0 ||
                            !Number.isFinite(metadata.mediaTime) ||
                            metadata.mediaTime < 0)
                            throw new ConnectError("media-capture", "Video frame timing unavailable");
                        origin ??= presentationUs - mediaUs;
                        timestamp = origin + mediaUs;
                    }
                    if (!Number.isSafeInteger(timestamp) || timestamp < 0)
                        throw new ConnectError("media-capture", "Invalid video capture timestamp");
                    out.drawImage(video, 0, 0, 640, 360);
                    this.connection.encodeVideo(this.options.videoStreamId, new VideoFrame(canvas, {
                        timestamp,
                    }));
                }
            }
            catch (error) {
                this.close(error instanceof Error
                    ? error
                    : new ConnectError("media-capture", "Video capture failed"));
                return;
            }
            this.callback = video.requestVideoFrameCallback(frame);
        };
        this.callback = video.requestVideoFrameCallback(frame);
    }
    pause() {
        if (this.stopped || this.paused)
            return;
        this.paused = true;
        for (const track of this.enabled.keys())
            track.enabled = false;
        this.resetClock();
        void this.context?.suspend().catch(() => { });
        this.options.state?.("paused");
    }
    async resume() {
        if (this.stopped)
            throw new ConnectError("closed", "Capture closed");
        if (document.visibilityState === "hidden")
            throw new ConnectError("media-capture", "Resume capture from a visible page");
        if (this.context)
            try {
                await this.bound(this.context.resume(), performance.now() + 5000);
            }
            catch (error) {
                this.close(error instanceof Error ? error : undefined);
                throw error;
            }
        if (this.stopped)
            return;
        this.resetClock();
        for (const [track, enabled] of this.enabled)
            track.enabled = enabled;
        this.paused = false;
        this.options.state?.("active");
    }
    get closed() {
        return this.stopped;
    }
    close(error) {
        if (this.stopped)
            return;
        this.stopped = true;
        this.failure = error;
        clearTimeout(this.timer);
        this.abort.abort();
        this.stopOwner?.();
        this.options.signal?.removeEventListener("abort", this.cancelled);
        if (typeof document !== "undefined")
            document.removeEventListener("visibilitychange", this.visibility);
        if (this.video) {
            if (this.callback !== undefined)
                this.video.cancelVideoFrameCallback(this.callback);
            this.video.pause();
            this.video.srcObject = null;
        }
        if (this.worklet) {
            this.worklet.port.postMessage({ type: "stop" });
            this.worklet.port.close();
            this.worklet.disconnect();
        }
        this.input?.disconnect();
        this.silent?.disconnect();
        void this.context?.close().catch(() => { });
        for (const [track, handler] of this.ended)
            track.removeEventListener("ended", handler);
        this.ended.clear();
        for (const stream of this.streams)
            for (const track of stream.getTracks())
                track.stop();
        this.streams.clear();
        this.enabled.clear();
        this.options.state?.("closed", error);
    }
}
