export interface CanvasVideoCapture {
    readonly stream: MediaStream;
    close(): void;
}
/** Canvas tracks do not uniformly implement camera constraints. Reopen their
 * actual canvas with an SDK-owned explicit frameRequestRate instead. No scaling,
 * frame rewriting, timestamp offset or claim about decoded playback is added.
 */
export declare function boundedCanvasVideoCapture(input: MediaStream, canvas: HTMLCanvasElement, fail: (error: Error) => void): CanvasVideoCapture;
