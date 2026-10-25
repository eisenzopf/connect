import { ConnectError } from "./errors.js";
import { MEDIA_LIMITS, type Vp8Frame } from "./browser_rtp.js";

export const OPUS_CONFIG: AudioEncoderConfig = Object.freeze({
    codec: "opus",
    sampleRate: 48000,
    numberOfChannels: 1,
    bitrate: 32000,
    opus: Object.freeze({ frameDuration: 20000 }),
});
export const VP8_CONFIG: VideoEncoderConfig = Object.freeze({
    codec: "vp8",
    width: 640,
    height: 360,
    bitrate: 500000,
    framerate: 30,
    latencyMode: "realtime",
});
export interface CodecSupport {
    audio: boolean;
    video: boolean;
    missing: string[];
}
export async function browserCodecSupport(): Promise<CodecSupport> {
    const missing: string[] = [];
    let audio = false,
        video = false;
    if (
        typeof AudioEncoder === "function" &&
        typeof AudioDecoder === "function" &&
        typeof AudioData === "function"
    ) {
        try {
            audio =
                !!(await AudioEncoder.isConfigSupported(OPUS_CONFIG))
                    .supported &&
                !!(await AudioDecoder.isConfigSupported(OPUS_CONFIG)).supported;
        } catch {
            /* An API can exist without this codec. */
        }
    }
    if (!audio) missing.push("Opus mono 48 kHz WebCodecs");
    if (
        typeof VideoEncoder === "function" &&
        typeof VideoDecoder === "function" &&
        typeof VideoFrame === "function"
    ) {
        try {
            video =
                !!(await VideoEncoder.isConfigSupported(VP8_CONFIG))
                    .supported &&
                !!(await VideoDecoder.isConfigSupported({ codec: "vp8" }))
                    .supported;
        } catch {
            /* Report the unsupported profile. */
        }
    }
    if (!video) missing.push("VP8 640x360 WebCodecs");
    return { audio, video, missing };
}
export interface EncodedMedia {
    bytes: Uint8Array;
    timestampUs: number;
    key: boolean;
}
interface CodecCallbacks<T> {
    encoded: (frame: EncodedMedia) => void;
    /** Borrowed during this synchronous callback; the codec always closes it. */
    decoded: (frame: T) => void;
    error: (error: Error) => void;
}
function error(value: unknown): Error {
    return value instanceof Error
        ? value
        : new ConnectError("media", "Browser codec failed");
}

/** Caller capture/clock state is separate from codec and network state. */
export class BrowserOpusCodec {
    private readonly encoder: AudioEncoder;
    private readonly decoder: AudioDecoder;
    private stopped = false;
    constructor(private readonly callbacks: CodecCallbacks<AudioData>) {
        if (
            typeof AudioEncoder !== "function" ||
            typeof AudioDecoder !== "function"
        )
            throw new ConnectError(
                "unsupported-media-profile",
                "Opus WebCodecs is unavailable",
            );
        this.encoder = new AudioEncoder({
            output: (chunk) => {
                if (this.stopped) return;
                try {
                    if (chunk.byteLength > 1160)
                        throw new ConnectError(
                            "media",
                            "Encoded Opus exceeds negotiated RTP limit",
                        );
                    const bytes = new Uint8Array(chunk.byteLength);
                    chunk.copyTo(bytes);
                    callbacks.encoded({
                        bytes,
                        timestampUs: chunk.timestamp,
                        key: true,
                    });
                } catch (e) {
                    this.fail(e);
                }
            },
            error: (e) => this.fail(e),
        });
        this.decoder = new AudioDecoder({
            output: (frame) => {
                try {
                    if (!this.stopped) callbacks.decoded(frame);
                } catch (e) {
                    this.fail(e);
                } finally {
                    frame.close();
                }
            },
            error: (e) => this.fail(e),
        });
        try {
            this.encoder.configure(OPUS_CONFIG);
            this.decoder.configure(OPUS_CONFIG);
        } catch (e) {
            this.close();
            throw new ConnectError(
                "unsupported-media-profile",
                "Opus profile configuration failed",
            );
        }
    }
    encode(samples: Float32Array, timestampUs: number): boolean {
        if (this.stopped) return false;
        if (
            samples.length !== 960 ||
            samples.some((sample) => !Number.isFinite(sample)) ||
            !Number.isSafeInteger(timestampUs) ||
            timestampUs < 0
        )
            throw new ConnectError(
                "media",
                "Use 960 finite mono PCM samples per 20 ms Opus frame",
            );
        if (this.encoder.encodeQueueSize >= MEDIA_LIMITS.audioPackets)
            return false;
        const frame = new AudioData({
            format: "f32-planar",
            sampleRate: 48000,
            numberOfFrames: 960,
            numberOfChannels: 1,
            timestamp: timestampUs,
            data: new Float32Array(samples),
        });
        try {
            this.encoder.encode(frame);
            return true;
        } finally {
            frame.close();
        }
    }
    decode(bytes: Uint8Array, timestampUs: number): boolean {
        if (
            this.stopped ||
            this.decoder.decodeQueueSize >= MEDIA_LIMITS.audioPackets
        )
            return false;
        if (!bytes.length || bytes.length > 1160)
            throw new ConnectError("media", "Invalid Opus RTP payload");
        this.decoder.decode(
            new EncodedAudioChunk({
                type: "key",
                timestamp: timestampUs,
                duration: 20000,
                data: bytes,
            }),
        );
        return true;
    }
    private fail(value: unknown): void {
        if (!this.stopped) {
            this.close();
            this.callbacks.error(error(value));
        }
    }
    close(): void {
        if (this.stopped) return;
        this.stopped = true;
        if (this.encoder.state !== "closed") this.encoder.close();
        if (this.decoder.state !== "closed") this.decoder.close();
    }
}

export class BrowserVp8Codec {
    private readonly encoder: VideoEncoder;
    private readonly decoder: VideoDecoder;
    private stopped = false;
    private lastKey = -Infinity;
    private lastFrame = -Infinity;
    private readonly frameTimes: number[] = [];
    private forceKey = false;
    private dropUntilKey = false;
    private suspendedUntil = -Infinity;
    private rateBytes = 0;
    private readonly rateWindow: { timestamp: number; bytes: number }[] = [];
    private needsKey = true;
    private ssrc?: number;
    constructor(private readonly callbacks: CodecCallbacks<VideoFrame>) {
        if (
            typeof VideoEncoder !== "function" ||
            typeof VideoDecoder !== "function"
        )
            throw new ConnectError(
                "unsupported-media-profile",
                "VP8 WebCodecs is unavailable",
            );
        this.encoder = new VideoEncoder({
            output: (chunk) => {
                if (this.stopped) return;
                try {
                    if (chunk.byteLength > MEDIA_LIMITS.frameBytes)
                        throw new ConnectError(
                            "media",
                            "Encoded VP8 exceeds frame limit",
                        );
                    while (
                        this.rateWindow[0] &&
                        chunk.timestamp - this.rateWindow[0].timestamp >=
                            1000000
                    )
                        this.rateBytes -= this.rateWindow.shift()!.bytes;
                    if (this.rateBytes + chunk.byteLength > 125000) {
                        this.dropUntilKey = this.forceKey = true;
                        this.suspendedUntil =
                            (this.rateWindow[0]?.timestamp ?? chunk.timestamp) +
                            1000000;
                        return;
                    }
                    if (this.dropUntilKey && chunk.type !== "key") return;
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
                } catch (e) {
                    this.fail(e);
                }
            },
            error: (e) => this.fail(e),
        });
        this.decoder = new VideoDecoder({
            output: (frame) => {
                try {
                    if (!this.stopped) callbacks.decoded(frame);
                } catch (e) {
                    this.fail(e);
                } finally {
                    frame.close();
                }
            },
            error: (e) => this.fail(e),
        });
        try {
            this.encoder.configure(VP8_CONFIG);
            this.decoder.configure({ codec: "vp8", optimizeForLatency: true });
        } catch (e) {
            this.close();
            throw new ConnectError(
                "unsupported-media-profile",
                "VP8 profile configuration failed",
            );
        }
    }
    /** Consumes and closes the caller's VideoFrame on every path. */
    encode(frame: VideoFrame): boolean {
        try {
            while (
                this.frameTimes.length &&
                frame.timestamp - this.frameTimes[0] >= 1000000
            )
                this.frameTimes.shift();
            if (
                this.stopped ||
                this.encoder.encodeQueueSize >= 2 ||
                frame.timestamp <= this.lastFrame ||
                this.frameTimes.length >= 30 ||
                frame.timestamp < this.suspendedUntil
            )
                return false;
            this.lastFrame = frame.timestamp;
            this.frameTimes.push(frame.timestamp);
            const keyFrame =
                this.forceKey || frame.timestamp - this.lastKey >= 1000000;
            if (keyFrame) this.lastKey = frame.timestamp;
            this.forceKey = false;
            this.encoder.encode(frame, { keyFrame });
            return true;
        } finally {
            frame.close();
        }
    }
    /** Call after loss/expired assembly; delta frames cannot establish readiness. */
    lost(): void {
        this.needsKey = true;
    }
    requestKeyFrame(): void {
        this.forceKey = true;
    }
    decode(frame: Vp8Frame, timestampUs: number): boolean {
        if (this.stopped || this.decoder.decodeQueueSize >= 8) {
            this.needsKey = true;
            return false;
        }
        if (this.ssrc !== frame.ssrc) {
            this.ssrc = frame.ssrc;
            this.needsKey = true;
            this.decoder.reset();
            this.decoder.configure({ codec: "vp8", optimizeForLatency: true });
        }
        if (!frame.bytes.length || frame.bytes.length > MEDIA_LIMITS.frameBytes)
            throw new ConnectError("media", "Invalid complete VP8 frame");
        if (this.needsKey && !frame.key) return false;
        this.decoder.decode(
            new EncodedVideoChunk({
                type: frame.key ? "key" : "delta",
                timestamp: timestampUs,
                data: frame.bytes,
            }),
        );
        this.needsKey = false;
        return true;
    }
    private fail(value: unknown): void {
        if (!this.stopped) {
            this.close();
            this.callbacks.error(error(value));
        }
    }
    close(): void {
        if (this.stopped) return;
        this.stopped = true;
        if (this.encoder.state !== "closed") this.encoder.close();
        if (this.decoder.state !== "closed") this.decoder.close();
    }
}

/** RTP wrap is unwrapped within a bounded half-range; late frames keep their old time. */
export class RtpClock {
    private origin?: number;
    private newest?: number;
    constructor(private readonly rate: 48000 | 90000) {}
    timestamp(value: number): number {
        if (this.origin === undefined) this.origin = this.newest = value;
        const low = this.newest! >>> 0;
        const signed = (value - low) | 0;
        const extended = this.newest! + signed;
        if (extended > this.newest!) this.newest = extended;
        return Math.round(((extended - this.origin) * 1000000) / this.rate);
    }
}
