import { type BrowserSdkRequestOptions, type BrowserConnection, type BrowserCaptureHandle, type BrowserOperation, type BrowserCompletion } from "./browser_sdk.js";
export interface BrowserInputDevice {
    readonly id: string;
    readonly label: string;
}
/** Enumerates local input choices without prompting for capture permission. */
export declare function browserInputDevices(options: BrowserSdkRequestOptions): Promise<{
    readonly microphones: readonly BrowserInputDevice[];
    readonly cameras: readonly BrowserInputDevice[];
}>;
export interface BrowserSourceSelection {
    readonly audio: boolean;
    readonly video?: "camera" | "screen";
    readonly audioDeviceId?: string;
    readonly videoDeviceId?: string;
}
/** App-owned source choice. Saving intent and offering the Connection remain explicit. */
export declare class BrowserSourcePicker {
    readonly element: HTMLFieldSetElement;
    private readonly audio;
    private readonly video;
    private readonly camera;
    private readonly events;
    private stopped;
    private generation;
    private readonly videoRequired;
    constructor(allowed: {
        readonly audio: boolean;
        readonly video: readonly ("camera" | "screen")[];
    });
    private update;
    selection(): BrowserSourceSelection;
    refresh(options: BrowserSdkRequestOptions): Promise<void>;
    close(): void;
}
export type BrowserUiState = "idle" | "pending" | "ready" | "failed" | "closed";
/** Capture-only UI. A retry requests permission for the same Connection, never a new offer. */
export declare class BrowserCaptureControls {
    private readonly connection;
    private readonly selection;
    private readonly options;
    readonly element: HTMLElement;
    private readonly message;
    private readonly startButton;
    private readonly stopButton;
    private readonly events;
    private pending?;
    private abort?;
    private owner?;
    private phase;
    get state(): BrowserUiState;
    constructor(connection: BrowserConnection, selection: BrowserSourceSelection, options: BrowserSdkRequestOptions);
    start(): Promise<BrowserCaptureHandle>;
    private off;
    stop(): void;
    close(): void;
}
/** Reads one original Operation. Rechecking or stopping a wait never mutates it. */
export declare class BrowserOperationProgress<T> {
    private readonly operation;
    private readonly options;
    readonly element: HTMLElement;
    private readonly message;
    private readonly checkButton;
    private readonly stopButton;
    private readonly events;
    private abort?;
    private pending?;
    private phase;
    get state(): BrowserUiState;
    constructor(operation: BrowserOperation<T>, options: BrowserSdkRequestOptions);
    observe(): Promise<BrowserCompletion<T>>;
    close(): void;
}
