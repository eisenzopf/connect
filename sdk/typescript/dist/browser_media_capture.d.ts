import { type BrowserMediaConnection } from "./browser_media.js";
import { type BrowserCaptureDevices } from "./browser_capture.js";
export interface BrowserMediaCaptureOptions extends BrowserCaptureDevices {
    audioStreamId?: string;
    videoStreamId?: string;
    videoSource?: "camera" | "screen";
    /** Transfers ownership: all supplied tracks stop when this capture closes. */
    stream?: MediaStream;
    timeoutMs?: number;
    signal?: AbortSignal;
    state?: (state: "active" | "paused" | "closed", error?: Error) => void;
}
export declare function browserCaptureSupport(): {
    microphone: boolean;
    camera: boolean;
    screen: boolean;
    audioWorklet: boolean;
    videoFrames: boolean;
};
/** Owned device/screen capture only. Decoded playback belongs to the renderer. */
export declare class BrowserMediaCapture {
    private readonly connection;
    private readonly options;
    private readonly streams;
    private readonly enabled;
    private readonly ended;
    private readonly abort;
    private readonly until;
    private context?;
    private input?;
    private worklet?;
    private silent?;
    private video?;
    private callback?;
    private timer?;
    private stopOwner?;
    private epoch;
    private stopped;
    private failure?;
    private paused;
    private readonly visibility;
    private readonly cancelled;
    private constructor();
    static open(connection: BrowserMediaConnection, options: BrowserMediaCaptureOptions): Promise<BrowserMediaCapture>;
    private own;
    private permission;
    private bound;
    private resetClock;
    private audio;
    private camera;
    pause(): void;
    resume(): Promise<void>;
    get closed(): boolean;
    close(error?: Error): void;
}
