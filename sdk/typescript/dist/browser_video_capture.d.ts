export interface CanvasVideoCapture {
    readonly stream: MediaStream;
    close(): void;
}
/** Capture original canvas changes without adding another sampling clock.
 * The original sender owns the30fps output bound before signaling. No scaling,
 * frame rewriting, timestamp offset or claim about decoded playback is added.
 */
export declare function boundedCanvasVideoCapture(input: MediaStream, canvas: HTMLCanvasElement, fail: (error: Error) => void): CanvasVideoCapture;
