import { type BrowserMediaConnection } from "./browser_media.js";
import { type MediaBinding } from "./browser_rtp.js";
export interface BrowserMediaRendererOptions {
    audio?: boolean;
    /** Existing application-owned canvases, indexed by negotiated receive Stream. */
    video?: Readonly<Record<string, HTMLCanvasElement>>;
    volume?: number;
    timeoutMs?: number;
    signal?: AbortSignal;
}
/** Borrowed-frame entry points with bounded owned presentation queues. */
export declare class BrowserMediaRenderer {
    private readonly connection;
    private readonly options;
    private context?;
    private gain?;
    private analyser?;
    private readonly sources;
    private readonly videos;
    private readonly lastVideo;
    private clock?;
    private presentationTimer?;
    private readonly canvases;
    private readonly abort;
    private stopOwner?;
    private nextAudio;
    private lastAudio;
    private stopped;
    private paused;
    private failure?;
    private finish;
    private failFinish;
    /** Settles only after the owned audio graph closes, including startup failure. */
    readonly finished: Promise<void>;
    private counts;
    private readonly visibility;
    private readonly cancelled;
    private constructor();
    static open(connection: BrowserMediaConnection, options?: BrowserMediaRendererOptions): Promise<BrowserMediaRenderer>;
    private check;
    private bound;
    private current;
    /** Copy synchronously; the decoder retains ownership of this AudioData. */
    audio(binding: Readonly<MediaBinding>, frame: AudioData): boolean;
    private now;
    private presentationTime;
    /** Copy borrowed frames into a bounded owned presentation queue. */
    video(binding: Readonly<MediaBinding>, frame: VideoFrame): boolean;
    private discardVideo;
    private pumpPresentation;
    private schedulePresentation;
    private clearPresentation;
    private clearAudio;
    pause(): void;
    resume(): Promise<void>;
    get diagnostics(): Readonly<typeof this.counts & {
        closed: boolean;
        paused: boolean;
        pendingAudio: number;
        pendingVideo: number;
        queuedAudioSeconds: number;
        audioState?: AudioContextState;
        audioRms: number;
        error?: string;
    }>;
    close(error?: Error): void;
}
