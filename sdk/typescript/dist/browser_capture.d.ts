export interface BrowserCaptureDevices {
    audioDeviceId?: string;
    videoDeviceId?: string;
}
/** Device-local IDs stay in capture constraints, never signaling or diagnostics. */
export declare function captureDevices(options: BrowserCaptureDevices, video: boolean, supplied: boolean): BrowserCaptureDevices;
/** A video join must not turn an audio-only or inactive answer into success. */
export declare function validateVideoAnswer(sdp: string): void;
/** The public fixed Opus/VP8 program must be accepted in full, without extra channels. */
export declare function validateBrowserAnswer(sdp: string, video: boolean): void;
/** Capture owns late permission results after deadline or caller cancellation. */
export declare function microphoneStream(timeoutMs: number, video: boolean, devices?: BrowserCaptureDevices, signal?: AbortSignal): Promise<MediaStream>;
