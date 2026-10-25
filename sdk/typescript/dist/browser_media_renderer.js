import { ConnectError } from "./errors.js";
/** Synchronous borrowed-frame sinks; protocol and capture ownership remain separate. */
export class BrowserMediaRenderer {
    connection;
    options;
    context;
    gain;
    analyser;
    sources = new Set();
    canvases = new Map();
    abort = new AbortController();
    stopOwner;
    nextAudio = 0;
    lastAudio = -Infinity;
    stopped = false;
    paused = false;
    failure;
    finish;
    failFinish;
    /** Settles only after the owned audio graph closes, including startup failure. */
    finished = new Promise((resolve, reject) => {
        this.finish = resolve;
        this.failFinish = reject;
    });
    counts = {
        audio: 0,
        video: 0,
        endedAudio: 0,
        droppedAudio: 0,
        droppedVideo: 0,
    };
    visibility = () => {
        if (document.visibilityState === "hidden")
            this.pause();
    };
    cancelled = () => this.close(new ConnectError("cancelled", "Playback cancelled"));
    constructor(connection, options) {
        this.connection = connection;
        this.options = options;
        void this.finished.catch(() => { });
    }
    static async open(connection, options = {}) {
        const timeout = options.timeoutMs ?? 10000, volume = options.volume ?? 1;
        if (!Number.isFinite(timeout) ||
            timeout < 1 ||
            timeout > 60000 ||
            !Number.isFinite(volume) ||
            volume < 0 ||
            volume > 1 ||
            Object.keys(options.video ?? {}).length > 8)
            throw new ConnectError("invalid-request", "Use bounded playback options");
        if (options.audio !== false && typeof AudioContext !== "function")
            throw new ConnectError("unsupported-media-profile", "Audio playback unavailable");
        const renderer = new BrowserMediaRenderer(connection, options);
        try {
            renderer.stopOwner = connection.onClose(() => renderer.close());
            options.signal?.addEventListener("abort", renderer.cancelled, {
                once: true,
            });
            if (options.signal?.aborted)
                renderer.cancelled();
            renderer.check();
            for (const [id, canvas] of Object.entries(options.video ?? {})) {
                if (!connection.streams.some((s) => s.streamId === id &&
                    (s.kind === "video" || s.kind === "screen") &&
                    s.direction !== "sendonly"))
                    throw new ConnectError("invalid-request", "Render only negotiated receive Streams");
                canvas.width = 640;
                canvas.height = 360;
                const out = canvas.getContext("2d");
                if (!out)
                    throw new ConnectError("unsupported-media-profile", "Playback canvas unavailable");
                renderer.canvases.set(id, out);
            }
            if (options.audio !== false) {
                const context = (renderer.context = new AudioContext({
                    sampleRate: 48000,
                    latencyHint: "interactive",
                }));
                if (context.sampleRate !== 48000)
                    throw new ConnectError("unsupported-media-profile", "Playback requires a 48 kHz graph");
                const gain = (renderer.gain = context.createGain());
                gain.gain.value = volume;
                const analyser = (renderer.analyser = context.createAnalyser());
                analyser.fftSize = 2048;
                analyser.connect(gain);
                gain.connect(context.destination);
                await renderer.bound(context.resume(), timeout);
                renderer.check();
                if (context.state !== "running")
                    throw new ConnectError("media-playback", "Audio playback did not start");
                context.onstatechange = () => {
                    if (renderer.stopped || context.state === "running")
                        return;
                    if (context.state === "closed")
                        renderer.close(new ConnectError("media-playback", "Playback graph closed"));
                    else
                        renderer.pause();
                };
            }
            document.addEventListener("visibilitychange", renderer.visibility);
            renderer.visibility();
            return renderer;
        }
        catch (error) {
            renderer.close(error instanceof Error
                ? error
                : new ConnectError("media-playback", "Playback startup failed"));
            throw error;
        }
    }
    check() {
        if (this.stopped)
            throw this.failure ?? new ConnectError("closed", "Playback closed");
    }
    bound(work, timeout) {
        return new Promise((resolve, reject) => {
            const cancelled = () => {
                cleanup();
                reject(this.failure ??
                    new ConnectError("closed", "Playback closed"));
            };
            const timer = setTimeout(() => {
                cleanup();
                reject(new ConnectError("timeout", "Playback startup deadline"));
            }, timeout);
            const cleanup = () => {
                clearTimeout(timer);
                this.abort.signal.removeEventListener("abort", cancelled);
            };
            this.abort.signal.addEventListener("abort", cancelled, {
                once: true,
            });
            if (this.stopped)
                cancelled();
            work.then(() => {
                cleanup();
                resolve();
            }, (error) => {
                cleanup();
                reject(error);
            });
        });
    }
    current(binding) {
        return (!this.stopped &&
            !this.paused &&
            binding.connectionId === this.connection.id &&
            this.connection.streams.some((s) => s.handle === binding.handle &&
                s.generation === binding.generation &&
                s.streamId === binding.streamId &&
                s.sessionId === binding.sessionId &&
                s.kind === binding.kind &&
                s.codec === binding.codec &&
                s.direction !== "sendonly"));
    }
    /** Copy synchronously; the decoder retains ownership of this AudioData. */
    audio(binding, frame) {
        if (!this.current(binding) ||
            binding.kind !== "audio" ||
            !this.context ||
            this.context.state !== "running") {
            this.counts.droppedAudio++;
            return false;
        }
        const context = this.context, now = context.currentTime;
        if (frame.sampleRate !== 48000 ||
            frame.numberOfChannels !== 1 ||
            frame.numberOfFrames !== 960 ||
            !Number.isSafeInteger(frame.timestamp))
            throw new ConnectError("unsupported-media-profile", "Playback requires mono 20 ms Opus output");
        const start = Math.max(now + 0.01, this.nextAudio);
        if (this.sources.size >= 10 ||
            start + 0.02 - now > 0.2 ||
            frame.timestamp <= this.lastAudio) {
            this.counts.droppedAudio++;
            return false;
        }
        const pcm = new Float32Array(960);
        frame.copyTo(pcm, { planeIndex: 0, format: "f32-planar" });
        const buffer = context.createBuffer(1, 960, 48000);
        buffer.copyToChannel(pcm, 0);
        const source = context.createBufferSource();
        source.buffer = buffer;
        source.connect(this.analyser);
        this.sources.add(source);
        source.onended = () => {
            this.sources.delete(source);
            source.disconnect();
            this.counts.endedAudio++;
        };
        try {
            source.start(start);
        }
        catch (error) {
            source.onended = null;
            this.sources.delete(source);
            source.disconnect();
            throw error;
        }
        this.nextAudio = start + 0.02;
        this.lastAudio = frame.timestamp;
        this.counts.audio++;
        return true;
    }
    /** Draw synchronously without retaining or closing the decoder's borrowed frame. */
    video(binding, frame) {
        if (!this.current(binding) ||
            (binding.kind !== "video" && binding.kind !== "screen")) {
            this.counts.droppedVideo++;
            return false;
        }
        const out = this.canvases.get(binding.streamId);
        if (!out) {
            this.counts.droppedVideo++;
            return false;
        }
        out.drawImage(frame, 0, 0, 640, 360);
        this.counts.video++;
        return true;
    }
    clearAudio() {
        for (const source of this.sources) {
            source.onended = null;
            try {
                source.stop();
            }
            catch { }
            source.disconnect();
        }
        this.sources.clear();
        this.nextAudio = 0;
        this.lastAudio = -Infinity;
    }
    pause() {
        if (this.stopped || this.paused)
            return;
        this.paused = true;
        this.clearAudio();
        void this.context?.suspend().catch(() => { });
    }
    async resume() {
        this.check();
        if (document.visibilityState === "hidden")
            throw new ConnectError("media-playback", "Resume playback from a visible page");
        try {
            if (this.context) {
                await this.bound(this.context.resume(), 5000);
                this.check();
                if (this.context.state !== "running")
                    throw new ConnectError("media-playback", "Playback did not resume");
            }
            this.paused = false;
        }
        catch (error) {
            this.close(error instanceof Error ? error : undefined);
            throw error;
        }
    }
    get diagnostics() {
        let audioRms = 0;
        if (this.analyser && !this.stopped) {
            const values = new Float32Array(2048);
            this.analyser.getFloatTimeDomainData(values);
            audioRms = Math.sqrt(values.reduce((sum, value) => sum + value * value, 0) /
                values.length);
        }
        return {
            ...this.counts,
            closed: this.stopped,
            paused: this.paused,
            pendingAudio: this.sources.size,
            queuedAudioSeconds: Math.max(0, this.nextAudio - (this.context?.currentTime ?? 0)),
            audioState: this.context?.state,
            audioRms,
            error: this.failure?.message,
        };
    }
    close(error) {
        if (this.stopped)
            return;
        this.stopped = true;
        this.failure = error;
        this.abort.abort();
        this.stopOwner?.();
        this.options.signal?.removeEventListener("abort", this.cancelled);
        if (typeof document !== "undefined")
            document.removeEventListener("visibilitychange", this.visibility);
        this.clearAudio();
        this.analyser?.disconnect();
        this.gain?.disconnect();
        this.canvases.clear();
        if (this.context) {
            this.context.onstatechange = null;
            void this.context.close().then(() => this.finish(), (error) => {
                this.failure =
                    error instanceof Error
                        ? error
                        : new Error("Playback graph failed to close");
                this.failFinish(this.failure);
            });
        }
        else
            this.finish();
    }
}
