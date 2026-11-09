import { type NativeActivationBoundary } from "./browser_native_activation.js";
/** Internal original native replacement proof. Decoder outputs are observations;
 * challenge, counters and acknowledgements never grant public ownership. */
import { BrowserClient } from "./browser_client.js";
import { type JsonObject } from "./uctp_json.js";
import { type Envelope, type Scope } from "./uctp_wire.js";
import { type BrowserReplacementRoot } from "./browser_replacement.js";
export interface NativeDecodeIdentity {
    readonly connectionId: string;
    readonly streamId: string;
    readonly generation: number;
    readonly ssrc: number;
}
interface AudioOutput {
    identity: NativeDecodeIdentity;
    sequence: number;
    timestamp: number;
    arrived: number;
    samples: number;
}
interface VideoOutput {
    identity: NativeDecodeIdentity;
    epoch: string;
    arrived: number;
    key: boolean;
    width: number;
    height: number;
}
export interface CurrentNativePreparation {
    readonly audio: NativeDecodeIdentity | undefined;
    readonly video: Readonly<{
        identity: NativeDecodeIdentity;
        epoch: string;
    }> | undefined;
}
/** A new instance belongs to precisely one installed target route set. */
export declare class BrowserNativeReadiness {
    readonly root: BrowserReplacementRoot;
    readonly audioStreamId: string;
    readonly videoStreamId: string | undefined;
    private readonly challenge;
    private readonly original;
    private readonly scope;
    private readonly activationReceivers;
    private activation?;
    get activationBoundary(): NativeActivationBoundary | undefined;
    private audio?;
    private audioSamples;
    private video?;
    private videoFrames;
    private videoKeys;
    private frozen?;
    private frozenAudio?;
    private frozenVideo?;
    private attempted;
    private refused;
    private stopped;
    private completed;
    private revision;
    private terminalDiagnosticLogged;
    readonly targetGeneration: number;
    constructor(root: BrowserReplacementRoot, original: JsonObject, scope: Scope, answer: Envelope, callback: Envelope, requestId: string, audioStreamId: string, videoStreamId: string | undefined, connectionGeneration: number);
    /** Consumed before invoking the original readiness continuation. A later
     * definitive refusal never erases the fact that its write was attempted. */
    get readinessSubmissionAttempted(): boolean;
    get mayHaveCommitted(): boolean;
    get observing(): boolean;
    get savedReadiness(): JsonObject | undefined;
    private accepts;
    audioDecoded(value: AudioOutput): void;
    videoDecoded(value: VideoOutput): void;
    private sameCurrent;
    private sameReplayRoute;
    private freeze;
    close(): void;
    /** One terminal metadata row, before caller cleanup; never an eligibility input. */
    private logTerminal;
    /** The original caller supplies one absolute deadline and actual route check. */
    finish(client: BrowserClient, until: number, current: () => CurrentNativePreparation, signal?: AbortSignal): Promise<JsonObject>;
}
export {};
