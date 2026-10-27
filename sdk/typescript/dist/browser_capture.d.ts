export interface BrowserCaptureDevices {
    audioDeviceId?: string;
    videoDeviceId?: string;
}
/** Device-local IDs stay in capture constraints, never signaling or diagnostics. */
export declare function captureDevices(options: BrowserCaptureDevices, video: boolean, supplied: boolean): BrowserCaptureDevices;
/** A video join must not turn an audio-only or inactive answer into success. */
export declare function validateVideoAnswer(sdp: string): void;
/** Capture owns late permission results after deadline or caller cancellation. */
export declare function microphoneStream(timeoutMs: number, video: boolean, devices?: BrowserCaptureDevices, signal?: AbortSignal): Promise<MediaStream>;
export declare function waitForIce(peer: RTCPeerConnection, timeoutMs: number): Promise<void>;
