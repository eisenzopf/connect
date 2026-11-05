export interface CanvasVideoCapture {
    readonly stream: MediaStream;
    close(): void;
}
/** An explicit canvas source is never inferred from an arbitrary supplied track.
 * Validate before taking track ownership or requesting a device permission. */
export declare function captureVideoCanvas(options: {
    videoCanvas?: HTMLCanvasElement;
    videoDeviceId?: string;
    stream?: MediaStream;
}, source: "camera" | "screen" | undefined): HTMLCanvasElement | undefined;
/** Capture original canvas changes without adding another sampling clock.
 * The original RTC sender or WT encoder owns the 30fps output bound. No scaling,
 * frame rewriting, timestamp offset or claim about decoded playback is added.
 */
export declare function boundedCanvasVideoCapture(input: MediaStream, canvas: HTMLCanvasElement, fail: (error: Error) => void): CanvasVideoCapture;
