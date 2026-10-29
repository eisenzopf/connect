import { BrowserClient, type RequestOptions } from "./browser_client.js";
import type { BrowserConnectionOptions } from "./browser_media.js";
import type { BrowserCaptureHandle, BrowserRenderHandle } from "./browser_sdk.js";
import type { BrowserMediaCaptureOptions } from "./browser_media_capture.js";
export interface BrowserRtcConfiguration {
    readonly iceServers?: readonly RTCIceServer[];
    readonly iceTransportPolicy?: RTCIceTransportPolicy;
}
export interface BrowserRtcCaptureInput {
    audio?: boolean;
    video?: "camera" | "screen";
    audioDeviceId?: string;
    videoDeviceId?: string;
    stream?: MediaStream;
    state?: BrowserMediaCaptureOptions["state"];
}
/** One current physical's RTC owner. No capture permission is requested by offer. */
export declare class BrowserRtcConnection {
    private readonly client;
    readonly input: BrowserConnectionOptions;
    private readonly tasks;
    private stopped;
    private readonly stop;
    private readonly retired;
    private readonly senders;
    private readonly remote;
    private signaling?;
    private worker?;
    private stopControl?;
    private stopEvents?;
    private captureOwner?;
    private renderOwner?;
    private readonly peer;
    private constructor();
    static begin(client: BrowserClient, input: BrowserConnectionOptions, participantId: string, configuration: BrowserRtcConfiguration, options: RequestOptions): {
        accepted: Promise<void>;
        ready: Promise<BrowserRtcConnection>;
        owner: BrowserRtcConnection;
        readonly submissionAttempted: boolean;
    };
    private check;
    private connected;
    onClose(fn: (error?: Error) => void): () => void;
    private onRetire;
    capture(input: BrowserRtcCaptureInput, options: RequestOptions): Promise<BrowserCaptureHandle>;
    render(input: {
        audio?: boolean;
        video?: Readonly<Record<string, HTMLCanvasElement>>;
        volume?: number;
    }, options: RequestOptions): Promise<BrowserRenderHandle>;
    close(error?: Error): void;
    shutdown(): Promise<void>;
}
