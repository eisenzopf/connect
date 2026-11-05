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
    counts = {
        inputCalls: 0,
        encodeAccepted: 0,
        encodeQueueRefusals: 0,
        encodedFrames: 0,
        decodeCalls: 0,
        decodeAccepted: 0,
        decodeQueueRefusals: 0,
        decodedFrames: 0,
        maxEncoderQueue: 0,
        maxDecoderQueue: 0,
        maxPendingEncode: 0,
        maxPendingDecode: 0,
        lastInputTimestampUs: null,
        lastEncodedTimestampUs: null,
        lastDecodeTimestampUs: null,
        lastDecodedTimestampUs: null,
        captureTimelineGaps: 0,
        nonforwardInputTimestamps: 0,
        maximumInputStepUs: 0,
    };
    /** Original codec-local counters only; never decoded-quality evidence. */
    get diagnostics() {
        return { ...this.counts };
    }
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
                    this.counts.encodedFrames++;
                    this.counts.lastEncodedTimestampUs = original.timestampUs;
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
                    this.counts.decodedFrames++;
                    this.counts.lastDecodedTimestampUs = original.timestampUs;
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
        this.counts.inputCalls++;
        const prior = this.counts.lastInputTimestampUs;
        if (prior !== null) {
            const step = timestampUs - prior;
            if (step > 20000)
                this.counts.captureTimelineGaps++;
            if (step <= 0)
                this.counts.nonforwardInputTimestamps++;
            this.counts.maximumInputStepUs = Math.max(this.counts.maximumInputStepUs, step);
        }
        this.counts.lastInputTimestampUs = timestampUs;
        this.counts.maxEncoderQueue = Math.max(this.counts.maxEncoderQueue, this.encoder.encodeQueueSize);
        if (this.encoder.encodeQueueSize >= MEDIA_LIMITS.audioPackets ||
            this.pendingEncode.length >= MEDIA_LIMITS.audioPackets) {
            this.counts.encodeQueueRefusals++;
            return false;
        }
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
        this.counts.maxPendingEncode = Math.max(this.counts.maxPendingEncode, this.pendingEncode.length);
        try {
            this.encoder.encode(frame);
            this.counts.encodeAccepted++;
            this.counts.maxEncoderQueue = Math.max(this.counts.maxEncoderQueue, this.encoder.encodeQueueSize);
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
        if (!this.stopped) {
            this.counts.decodeCalls++;
            this.counts.lastDecodeTimestampUs = timestampUs;
            this.counts.maxDecoderQueue = Math.max(this.counts.maxDecoderQueue, this.decoder.decodeQueueSize);
        }
        if (this.stopped ||
            this.decoder.decodeQueueSize >= MEDIA_LIMITS.audioPackets ||
            this.pendingDecode.length >= MEDIA_LIMITS.audioPackets) {
            if (!this.stopped)
                this.counts.decodeQueueRefusals++;
            return false;
        }
        if (!bytes.length ||
            bytes.length > 1160 ||
            !Number.isSafeInteger(timestampUs))
            throw new ConnectError("media", "Invalid Opus RTP payload or timestamp");
        const original = { timestampUs };
        this.pendingDecode.push(original);
        this.counts.maxPendingDecode = Math.max(this.counts.maxPendingDecode, this.pendingDecode.length);
        try {
            this.decoder.decode(new EncodedAudioChunk({
                type: "key",
                timestamp: timestampUs,
                duration: 20000,
                data: bytes,
            }));
            this.counts.decodeAccepted++;
            this.counts.maxDecoderQueue = Math.max(this.counts.maxDecoderQueue, this.decoder.decodeQueueSize);
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
    encodeCounts = {
        inputCalls: 0,
        acceptedFrames: 0,
        nativeOutputs: 0,
        forwardedOutputs: 0,
        stoppedRefusals: 0,
        queueRefusals: 0,
        timestampRefusals: 0,
        rateRefusals: 0,
        suspendedRefusals: 0,
        outputRateDrops: 0,
        outputWaitingForKeyDrops: 0,
        maxEncoderQueue: 0,
        lastInputTimestampUs: null,
        lastEncodedTimestampUs: null,
    };
    get encodeDiagnostics() {
        return { ...this.encodeCounts };
    }
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
        submittedFrames: 0,
        decodedFrames: 0,
        waitingForKeyDrops: 0,
        maxDecoderQueue: 0,
        lastDecodedTimestampUs: null,
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
                this.encodeCounts.nativeOutputs++;
                try {
                    if (chunk.byteLength > MEDIA_LIMITS.frameBytes)
                        throw new ConnectError("media", "Encoded VP8 exceeds frame limit");
                    while (this.rateWindow[0] &&
                        chunk.timestamp - this.rateWindow[0].timestamp >=
                            1000000)
                        this.rateBytes -= this.rateWindow.shift().bytes;
                    if (this.rateBytes + chunk.byteLength > 125000) {
                        this.encodeCounts.outputRateDrops++;
                        this.dropUntilKey = this.forceKey = true;
                        this.suspendedUntil =
                            (this.rateWindow[0]?.timestamp ?? chunk.timestamp) +
                                1000000;
                        return;
                    }
                    if (this.dropUntilKey && chunk.type !== "key") {
                        this.encodeCounts.outputWaitingForKeyDrops++;
                        return;
                    }
                    this.dropUntilKey = false;
                    this.rateWindow.push({
                        timestamp: chunk.timestamp,
                        bytes: chunk.byteLength,
                    });
                    this.rateBytes += chunk.byteLength;
                    const bytes = new Uint8Array(chunk.byteLength);
                    chunk.copyTo(bytes);
                    this.encodeCounts.forwardedOutputs++;
                    this.encodeCounts.lastEncodedTimestampUs = chunk.timestamp;
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
                    if (!this.stopped) {
                        this.decodeCounts.decodedFrames++;
                        this.decodeCounts.lastDecodedTimestampUs =
                            frame.timestamp;
                        callbacks.decoded(frame);
                    }
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
    encode(frame, currentFeedback = false) {
        try {
            while (this.frameTimes.length &&
                frame.timestamp - this.frameTimes[0] >= 1000000)
                this.frameTimes.shift();
            this.encodeCounts.inputCalls++;
            this.encodeCounts.lastInputTimestampUs = frame.timestamp;
            if (!this.stopped)
                this.encodeCounts.maxEncoderQueue = Math.max(this.encodeCounts.maxEncoderQueue, this.encoder.encodeQueueSize);
            const refusal = this.stopped
                ? "stoppedRefusals"
                : this.encoder.encodeQueueSize >= 2
                    ? "queueRefusals"
                    : frame.timestamp <= this.lastFrame
                        ? "timestampRefusals"
                        : this.frameTimes.length >= 30
                            ? "rateRefusals"
                            : frame.timestamp < this.suspendedUntil
                                ? "suspendedRefusals"
                                : null;
            if (refusal) {
                this.encodeCounts[refusal]++;
                return false;
            }
            this.lastFrame = frame.timestamp;
            this.frameTimes.push(frame.timestamp);
            // Refresh references within 500 ms after loss; the profile permits
            // at most one second between keyframes. Bitrate bounds still apply.
            const keyFrame = this.forceKey ||
                currentFeedback ||
                frame.timestamp - this.lastKey >= 500000;
            if (keyFrame)
                this.lastKey = frame.timestamp;
            this.forceKey = false;
            this.encoder.encode(frame, { keyFrame });
            this.encodeCounts.acceptedFrames++;
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
    /** A new authenticated history discards old decoder references and queued
     * outputs while preserving the caller's original source/receive clocks. */
    resetReceiveHistory() {
        if (this.stopped)
            return;
        this.needsKey = true;
        this.ssrc = undefined;
        this.lastSequence = this.lastTimestamp = undefined;
        this.decoder.reset();
        this.decoder.configure({ codec: "vp8", optimizeForLatency: true });
    }
    requestKeyFrame() {
        this.forceKey = true;
    }
    decode(frame, timestampUs) {
        if (!this.stopped)
            this.decodeCounts.maxDecoderQueue = Math.max(this.decodeCounts.maxDecoderQueue, this.decoder.decodeQueueSize);
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
        if (this.needsKey && !frame.key) {
            this.decodeCounts.waitingForKeyDrops++;
            return false;
        }
        this.decoder.decode(new EncodedVideoChunk({
            type: frame.key ? "key" : "delta",
            timestamp: timestampUs,
            data: frame.bytes,
        }));
        this.decodeCounts.submittedFrames++;
        this.decodeCounts.maxDecoderQueue = Math.max(this.decodeCounts.maxDecoderQueue, this.decoder.decodeQueueSize);
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
