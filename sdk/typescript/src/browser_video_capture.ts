import { ConnectError } from "./errors.js";

export interface CanvasVideoCapture {
    readonly stream: MediaStream;
    close(): void;
}
const unsupported = () =>
    new ConnectError(
        "unsupported-media-profile",
        "Canvas capture must retain bounded dimensions and original native frames",
    );

/** Capture original canvas changes without adding another sampling clock.
 * The original sender owns the30fps output bound before signaling. No scaling,
 * frame rewriting, timestamp offset or claim about decoded playback is added.
 */
export function boundedCanvasVideoCapture(
    input: MediaStream,
    canvas: HTMLCanvasElement,
    fail: (error: Error) => void,
): CanvasVideoCapture {
    if (
        typeof HTMLCanvasElement !== "function" ||
        !(canvas instanceof HTMLCanvasElement)
    )
        throw unsupported();
    const width = canvas.width,
        height = canvas.height;
    if (
        !Number.isInteger(width) ||
        width < 1 ||
        width > 640 ||
        !Number.isInteger(height) ||
        height < 1 ||
        height > 360 ||
        typeof canvas.captureStream !== "function" ||
        typeof MutationObserver !== "function"
    )
        throw unsupported();
    let owned: MediaStream | undefined,
        observer: MutationObserver | undefined,
        closed = false;
    const close = () => {
        if (closed) return;
        closed = true;
        observer?.disconnect();
        for (const track of owned?.getTracks() ?? []) track.stop();
    };
    try {
        // An independent frameRequestRate can discard changing source frames.
        // Preserve native canvas changes; the sender enforces the output rate.
        owned = canvas.captureStream();
        const tracks = owned.getTracks(),
            track = owned.getVideoTracks()[0];
        if (tracks.length !== 1 || !track || track.readyState !== "live")
            throw unsupported();
        const settings = track.getSettings();
        if (
            (settings.width !== undefined && settings.width !== width) ||
            (settings.height !== undefined && settings.height !== height) ||
            (settings.frameRate !== undefined &&
                (!Number.isFinite(settings.frameRate) ||
                    settings.frameRate <= 0))
        )
            throw unsupported();
        observer = new MutationObserver(() => {
            if (
                !closed &&
                (canvas.width !== width || canvas.height !== height)
            ) {
                close();
                fail(unsupported());
            }
        });
        observer.observe(canvas, {
            attributes: true,
            attributeFilter: ["width", "height"],
        });
        return {
            stream: new MediaStream([...input.getAudioTracks(), track]),
            close,
        };
    } catch (error) {
        close();
        if (error instanceof ConnectError) throw error;
        throw unsupported();
    }
}
