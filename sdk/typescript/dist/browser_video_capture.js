import { ConnectError } from "./errors.js";
const unsupported = () => new ConnectError("unsupported-media-profile", "Canvas capture must retain bounded dimensions and original native frames");
/** An explicit canvas source is never inferred from an arbitrary supplied track.
 * Validate before taking track ownership or requesting a device permission. */
export function captureVideoCanvas(options, source) {
    if (options.videoCanvas === undefined)
        return;
    if (source !== "camera" ||
        options.videoDeviceId !== undefined ||
        (options.stream && options.stream.getVideoTracks().length !== 0))
        throw new ConnectError("invalid-request", "Canvas input selects camera video without a device or supplied video track");
    validateCanvas(options.videoCanvas);
    return options.videoCanvas;
}
function validateCanvas(canvas) {
    if (typeof HTMLCanvasElement !== "function" ||
        !(canvas instanceof HTMLCanvasElement) ||
        !Number.isInteger(canvas.width) ||
        canvas.width < 1 ||
        canvas.width > 640 ||
        !Number.isInteger(canvas.height) ||
        canvas.height < 1 ||
        canvas.height > 360 ||
        typeof canvas.captureStream !== "function" ||
        typeof MutationObserver !== "function")
        throw unsupported();
}
/** Capture original canvas changes without adding another sampling clock.
 * The original RTC sender or WT encoder owns the 30fps output bound. No scaling,
 * frame rewriting, timestamp offset or claim about decoded playback is added.
 */
export function boundedCanvasVideoCapture(input, canvas, fail) {
    validateCanvas(canvas);
    const width = canvas.width, height = canvas.height;
    let owned, observer, closed = false;
    const close = () => {
        if (closed)
            return;
        closed = true;
        observer?.disconnect();
        for (const track of owned?.getTracks() ?? [])
            track.stop();
    };
    try {
        // An independent frameRequestRate can discard changing source frames.
        // Preserve native canvas changes; the sender enforces the output rate.
        owned = canvas.captureStream();
        const tracks = owned.getTracks(), track = owned.getVideoTracks()[0];
        if (tracks.length !== 1 || !track || track.readyState !== "live")
            throw unsupported();
        const settings = track.getSettings();
        if ((settings.width !== undefined && settings.width !== width) ||
            (settings.height !== undefined && settings.height !== height) ||
            (settings.frameRate !== undefined &&
                (!Number.isFinite(settings.frameRate) ||
                    settings.frameRate <= 0)))
            throw unsupported();
        observer = new MutationObserver(() => {
            if (!closed &&
                (canvas.width !== width || canvas.height !== height)) {
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
    }
    catch (error) {
        close();
        if (error instanceof ConnectError)
            throw error;
        throw unsupported();
    }
}
