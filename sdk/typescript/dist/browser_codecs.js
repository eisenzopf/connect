import { ConnectError } from "./errors.js";
import { MEDIA_LIMITS } from "./browser_rtp.js";
export const OPUS_CONFIG = Object.freeze({
    codec: "opus",
    sampleRate: 48000,
    numberOfChannels: 1,
    bitrate: 32000,
    opus: Object.freeze({ frameDuration: 20000 }),
});
export const VP8_CONFIG = Object.freeze({
    codec: "vp8",
    width: 640,
    height: 360,
    bitrate: 500000,
    framerate: 30,
    latencyMode: "realtime",
});
export async function browserCodecSupport() {
    const missing = [];
    let audio = false, video = false;
    if (typeof AudioEncoder === "function" &&
        typeof AudioDecoder === "function" &&
        typeof AudioData === "function") {
        try {
            audio =
                !!(await AudioEncoder.isConfigSupported(OPUS_CONFIG))
                    .supported &&
                    !!(await AudioDecoder.isConfigSupported(OPUS_CONFIG)).supported;
        }
        catch {
            /* An API can exist without this codec. */
        }
    }
    if (!audio)
        missing.push("Opus mono 48 kHz WebCodecs");
    if (typeof VideoEncoder === "function" &&
        typeof VideoDecoder === "function" &&
        typeof VideoFrame === "function") {
        try {
            video =
                !!(await VideoEncoder.isConfigSupported(VP8_CONFIG))
                    .supported &&
                    !!(await VideoDecoder.isConfigSupported({ codec: "vp8" }))
                        .supported;
        }
        catch {
            /* Report the unsupported profile. */
        }
    }
    if (!video)
        missing.push("VP8 640x360 WebCodecs");
    return { audio, video, missing };
}
function error(value) {
    return value instanceof Error
        ? value
        : new ConnectError("media", "Browser codec failed");
}
/** Caller capture/clock state is separate from codec and network state. */
export class BrowserOpusCodec {
    callbacks;
    encoder;
    decoder;
    stopped = false;
    pendingEncode = [];
    pendingDecode = [];
    constructor(callbacks) {
        this.callbacks = callbacks;
        if (typeof AudioEncoder !== "function" ||
            typeof AudioDecoder !== "function")
            throw new ConnectError("unsupported-media-profile", "Opus WebCodecs is unavailable");
        this.encoder = new AudioEncoder({
            output: (chunk) => {
                if (this.stopped)
                    return;
                try {
                    const original = this.pendingEncode.shift();
                    if (!original)
                        throw new ConnectError("media", "Unmatched Opus encoder output");
                    if (chunk.byteLength > 1160)
                        throw new ConnectError("media", "Encoded Opus exceeds negotiated RTP limit");
                    const bytes = new Uint8Array(chunk.byteLength);
                    chunk.copyTo(bytes);
                    callbacks.encoded({
                        bytes,
                        // AudioEncoder can compress capture gaps, including a
                        // suspended graph, into a continuous sample clock.
                        timestampUs: original.timestampUs,
                        key: true,
                    });
                }
                catch (e) {
                    this.fail(e);
                }
            },
            error: (e) => this.fail(e),
        });
        this.decoder = new AudioDecoder({
            output: (frame) => {
                let restored;
                try {
                    if (this.stopped)
                        return;
                    const original = this.pendingDecode.shift();
                    if (!original)
                        throw new ConnectError("media", "Unmatched Opus decoder output");
                    if (frame.sampleRate !== 48000 ||
                        frame.numberOfChannels !== 1 ||
                        frame.numberOfFrames !== 960)
                        throw new ConnectError("unsupported-media-profile", "Decoded Opus requires mono 20 ms at 48 kHz");
                    // WebCodecs may emit a continuous sample clock across missing
                    // RTP packets. Keep the caller's original timeline so gaps,
                    // late packets and duplicates do not acquire invented times.
                    if (frame.timestamp !== original.timestampUs) {
                        const samples = new Float32Array(960);
                        frame.copyTo(samples, {
                            planeIndex: 0,
                            format: "f32-planar",
                        });
                        restored = new AudioData({
                            format: "f32-planar",
                            sampleRate: 48000,
                            numberOfFrames: 960,
                            numberOfChannels: 1,
                            timestamp: original.timestampUs,
                            data: samples,
                        });
                    }
                    callbacks.decoded(restored ?? frame);
                }
                catch (e) {
                    this.fail(e);
                }
                finally {
                    restored?.close();
                    frame.close();
                }
            },
            error: (e) => this.fail(e),
        });
        try {
            this.encoder.configure(OPUS_CONFIG);
            this.decoder.configure(OPUS_CONFIG);
        }
        catch (e) {
            this.close();
            throw new ConnectError("unsupported-media-profile", "Opus profile configuration failed");
        }
    }
    encode(samples, timestampUs) {
        if (this.stopped)
            return false;
        if (samples.length !== 960 ||
            samples.some((sample) => !Number.isFinite(sample)) ||
            !Number.isSafeInteger(timestampUs) ||
            timestampUs < 0)
            throw new ConnectError("media", "Use 960 finite mono PCM samples per 20 ms Opus frame");
        if (this.encoder.encodeQueueSize >= MEDIA_LIMITS.audioPackets ||
            this.pendingEncode.length >= MEDIA_LIMITS.audioPackets)
            return false;
        const frame = new AudioData({
            format: "f32-planar",
            sampleRate: 48000,
            numberOfFrames: 960,
            numberOfChannels: 1,
            timestamp: timestampUs,
            data: new Float32Array(samples),
        });
        const original = { timestampUs };
        this.pendingEncode.push(original);
        try {
            this.encoder.encode(frame);
            return true;
        }
        catch (e) {
            const index = this.pendingEncode.indexOf(original);
            if (index >= 0)
                this.pendingEncode.splice(index, 1);
            throw e;
        }
        finally {
            frame.close();
        }
    }
    decode(bytes, timestampUs) {
        if (this.stopped ||
            this.decoder.decodeQueueSize >= MEDIA_LIMITS.audioPackets ||
            this.pendingDecode.length >= MEDIA_LIMITS.audioPackets)
            return false;
        if (!bytes.length ||
            bytes.length > 1160 ||
            !Number.isSafeInteger(timestampUs))
            throw new ConnectError("media", "Invalid Opus RTP payload or timestamp");
        const original = { timestampUs };
        this.pendingDecode.push(original);
        try {
            this.decoder.decode(new EncodedAudioChunk({
                type: "key",
                timestamp: timestampUs,
                duration: 20000,
                data: bytes,
            }));
        }
        catch (e) {
            const index = this.pendingDecode.indexOf(original);
            if (index >= 0)
                this.pendingDecode.splice(index, 1);
            throw e;
        }
        return true;
    }
    fail(value) {
        if (!this.stopped) {
            this.close();
            this.callbacks.error(error(value));
        }
    }
    close() {
        if (this.stopped)
            return;
        this.stopped = true;
        this.pendingEncode.length = 0;
        this.pendingDecode.length = 0;
        if (this.encoder.state !== "closed")
            this.encoder.close();
        if (this.decoder.state !== "closed")
            this.decoder.close();
    }
}
export class BrowserVp8Codec {
    callbacks;
    encoder;
    decoder;
    stopped = false;
    lastKey = -Infinity;
    lastFrame = -Infinity;
    frameTimes = [];
    forceKey = false;
    dropUntilKey = false;
    suspendedUntil = -Infinity;
    rateBytes = 0;
    rateWindow = [];
    needsKey = true;
    ssrc;
    lastSequence;
    lastTimestamp;
    decodeCounts = {
        sourceEpochs: 0,
        sequenceGaps: 0,
        lateFrames: 0,
        assemblyLosses: 0,
        queueDrops: 0,
    };
    get decodeDiagnostics() {
        return { ...this.decodeCounts, waitingForKey: this.needsKey };
    }
    constructor(callbacks) {
        this.callbacks = callbacks;
        if (typeof VideoEncoder !== "function" ||
            typeof VideoDecoder !== "function")
            throw new ConnectError("unsupported-media-profile", "VP8 WebCodecs is unavailable");
        this.encoder = new VideoEncoder({
            output: (chunk) => {
                if (this.stopped)
                    return;
                try {
                    if (chunk.byteLength > MEDIA_LIMITS.frameBytes)
                        throw new ConnectError("media", "Encoded VP8 exceeds frame limit");
                    while (this.rateWindow[0] &&
                        chunk.timestamp - this.rateWindow[0].timestamp >=
                            1000000)
                        this.rateBytes -= this.rateWindow.shift().bytes;
                    if (this.rateBytes + chunk.byteLength > 125000) {
                        this.dropUntilKey = this.forceKey = true;
                        this.suspendedUntil =
                            (this.rateWindow[0]?.timestamp ?? chunk.timestamp) +
                                1000000;
                        return;
                    }
                    if (this.dropUntilKey && chunk.type !== "key")
                        return;
                    this.dropUntilKey = false;
                    this.rateWindow.push({
                        timestamp: chunk.timestamp,
                        bytes: chunk.byteLength,
                    });
                    this.rateBytes += chunk.byteLength;
                    const bytes = new Uint8Array(chunk.byteLength);
                    chunk.copyTo(bytes);
                    callbacks.encoded({
                        bytes,
                        timestampUs: chunk.timestamp,
                        key: chunk.type === "key",
                    });
                }
                catch (e) {
                    this.fail(e);
                }
            },
            error: (e) => this.fail(e),
        });
        this.decoder = new VideoDecoder({
            output: (frame) => {
                try {
                    if (!this.stopped)
                        callbacks.decoded(frame);
                }
                catch (e) {
                    this.fail(e);
                }
                finally {
                    frame.close();
                }
            },
            error: (e) => this.fail(e),
        });
        try {
            this.encoder.configure(VP8_CONFIG);
            this.decoder.configure({ codec: "vp8", optimizeForLatency: true });
        }
        catch (e) {
            this.close();
            throw new ConnectError("unsupported-media-profile", "VP8 profile configuration failed");
        }
    }
    /** Consumes and closes the caller's VideoFrame on every path. */
    encode(frame) {
        try {
            while (this.frameTimes.length &&
                frame.timestamp - this.frameTimes[0] >= 1000000)
                this.frameTimes.shift();
            if (this.stopped ||
                this.encoder.encodeQueueSize >= 2 ||
                frame.timestamp <= this.lastFrame ||
                this.frameTimes.length >= 30 ||
                frame.timestamp < this.suspendedUntil)
                return false;
            this.lastFrame = frame.timestamp;
            this.frameTimes.push(frame.timestamp);
            const keyFrame = this.forceKey || frame.timestamp - this.lastKey >= 1000000;
            if (keyFrame)
                this.lastKey = frame.timestamp;
            this.forceKey = false;
            this.encoder.encode(frame, { keyFrame });
            return true;
        }
        finally {
            frame.close();
        }
    }
    /** Call after loss/expired assembly; delta frames cannot establish readiness. */
    lost() {
        this.decodeCounts.assemblyLosses++;
        this.needsKey = true;
    }
    requestKeyFrame() {
        this.forceKey = true;
    }
    decode(frame, timestampUs) {
        if (this.stopped || this.decoder.decodeQueueSize >= 8) {
            if (!this.stopped)
                this.decodeCounts.queueDrops++;
            this.needsKey = true;
            return false;
        }
        if (this.ssrc !== frame.ssrc) {
            this.decodeCounts.sourceEpochs++;
            this.ssrc = frame.ssrc;
            this.needsKey = true;
            this.lastSequence = undefined;
            this.lastTimestamp = undefined;
            this.decoder.reset();
            this.decoder.configure({ codec: "vp8", optimizeForLatency: true });
        }
        if (!frame.bytes.length || frame.bytes.length > MEDIA_LIMITS.frameBytes)
            throw new ConnectError("media", "Invalid complete VP8 frame");
        // A wholly absent frame leaves no partial assembly to expire. Preserve
        // the assembler's exact RTP range so a later complete delta cannot be
        // decoded against a broken reference chain. Completed late/duplicate
        // frames must not move the reference clock backwards, including wrap.
        if (this.lastTimestamp !== undefined &&
            ((frame.timestamp - this.lastTimestamp) | 0) <= 0) {
            this.decodeCounts.lateFrames++;
            return false;
        }
        if (this.lastSequence !== undefined &&
            frame.firstSequence !== ((this.lastSequence + 1) & 65535)) {
            this.decodeCounts.sequenceGaps++;
            this.needsKey = true;
        }
        this.lastSequence = frame.lastSequence;
        this.lastTimestamp = frame.timestamp;
        if (this.needsKey && !frame.key)
            return false;
        this.decoder.decode(new EncodedVideoChunk({
            type: frame.key ? "key" : "delta",
            timestamp: timestampUs,
            data: frame.bytes,
        }));
        this.needsKey = false;
        return true;
    }
    fail(value) {
        if (!this.stopped) {
            this.close();
            this.callbacks.error(error(value));
        }
    }
    close() {
        if (this.stopped)
            return;
        this.stopped = true;
        if (this.encoder.state !== "closed")
            this.encoder.close();
        if (this.decoder.state !== "closed")
            this.decoder.close();
    }
}
/** RTP wrap is unwrapped within a bounded half-range; late frames keep their old time. */
export class RtpClock {
    rate;
    origin;
    newest;
    constructor(rate) {
        this.rate = rate;
    }
    timestamp(value) {
        if (this.origin === undefined)
            this.origin = this.newest = value;
        const low = this.newest >>> 0;
        const signed = (value - low) | 0;
        const extended = this.newest + signed;
        if (extended > this.newest)
            this.newest = extended;
        return Math.round(((extended - this.origin) * 1000000) / this.rate);
    }
}
