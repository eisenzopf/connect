import { type BrowserReplacementRoot } from "./browser_replacement.js";
import { BrowserClient, type RequestOptions } from "./browser_client.js";
import type { BrowserConnectionOptions } from "./browser_media.js";
import type { BrowserCaptureHandle, BrowserCaptureInput, BrowserRenderHandle } from "./browser_sdk.js";
export interface BrowserRtcConfiguration {
    readonly iceServers?: readonly RTCIceServer[];
    readonly iceTransportPolicy?: RTCIceTransportPolicy;
}
export type BrowserRtcCaptureInput = BrowserCaptureInput;
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
    private preparedDecoder?;
    private replacementCompletion?;
    private replacementVideoSource?;
    private replacementVideoPresented;
    private readonly peer;
    private constructor();
    static begin(client: BrowserClient, input: BrowserConnectionOptions, participantId: string, configuration: BrowserRtcConfiguration, options: RequestOptions): {
        accepted: Promise<void>;
        ready: Promise<BrowserRtcConnection>;
        owner: BrowserRtcConnection;
        readonly submissionAttempted: boolean;
    };
    /** Prepare a second physical without retiring the source. Completion needs
     * this fresh decoder's evidence and the original committed movement receipt. */
    static beginReplacement(client: BrowserClient, input: BrowserConnectionOptions, participantId: string, configuration: BrowserRtcConfiguration, original: BrowserReplacementRoot, options: RequestOptions): {
        prepared: Promise<BrowserRtcConnection>;
        completed: Promise<import("./browser_replacement.js").BrowserReplacementReceipt>;
        owner: BrowserRtcConnection;
        readonly submissionAttempted: boolean;
    };
    get diagnostics(): Readonly<{
        closed: boolean;
        peerConnectionState: RTCPeerConnectionState;
        remoteTracks: number;
        liveRemoteTracks: number;
        captureActive: boolean;
        renderActive: boolean;
    }>;
    get replacementMayHaveCommitted(): boolean;
    private retainMutedDecoderSinks;
    private retireMutedDecoderSinks;
    private check;
    private connected;
    onClose(fn: (error?: Error) => void): () => void;
    restartIce(operationId: string, expectedStreamRevision: number, expectedIceGeneration: number, options?: RequestOptions): Promise<import("./browser_webrtc_restart.js").WebRtcIceRestartReceipt>;
    get savedIceRestart(): {
        scope: import("./uctp_wire.js").Scope;
        payload?: import("./uctp_json.js").JsonObject;
        submissionAttempted: boolean;
    } | undefined;
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
