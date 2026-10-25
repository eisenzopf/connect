import { type Vp8Frame } from "./browser_rtp.js";
export declare const OPUS_CONFIG: AudioEncoderConfig;
export declare const VP8_CONFIG: VideoEncoderConfig;
export interface CodecSupport {
    audio: boolean;
    video: boolean;
    missing: string[];
}
export declare function browserCodecSupport(): Promise<CodecSupport>;
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
/** Caller capture/clock state is separate from codec and network state. */
export declare class BrowserOpusCodec {
    private readonly callbacks;
    private readonly encoder;
    private readonly decoder;
    private stopped;
    private readonly pendingDecode;
    constructor(callbacks: CodecCallbacks<AudioData>);
    encode(samples: Float32Array, timestampUs: number): boolean;
    decode(bytes: Uint8Array, timestampUs: number): boolean;
    private fail;
    close(): void;
}
export declare class BrowserVp8Codec {
    private readonly callbacks;
    private readonly encoder;
    private readonly decoder;
    private stopped;
    private lastKey;
    private lastFrame;
    private readonly frameTimes;
    private forceKey;
    private dropUntilKey;
    private suspendedUntil;
    private rateBytes;
    private readonly rateWindow;
    private needsKey;
    private ssrc?;
    private lastSequence?;
    private lastTimestamp?;
    private readonly decodeCounts;
    get decodeDiagnostics(): Readonly<typeof this.decodeCounts & {
        waitingForKey: boolean;
    }>;
    constructor(callbacks: CodecCallbacks<VideoFrame>);
    /** Consumes and closes the caller's VideoFrame on every path. */
    encode(frame: VideoFrame): boolean;
    /** Call after loss/expired assembly; delta frames cannot establish readiness. */
    lost(): void;
    requestKeyFrame(): void;
    decode(frame: Vp8Frame, timestampUs: number): boolean;
    private fail;
    close(): void;
}
/** RTP wrap is unwrapped within a bounded half-range; late frames keep their old time. */
export declare class RtpClock {
    private readonly rate;
    private origin?;
    private newest?;
    constructor(rate: 48000 | 90000);
    timestamp(value: number): number;
}
export {};
