import { ConnectError } from "./errors.js";
import { keyframeAdapter, stopVideoWorker } from "./browser_video_keyframes.js";
export { stopVideoWorker } from "./browser_video_keyframes.js";

/** Bound the original video encoding before an offer can activate media. */
export function addBoundedVideoSender(
    peer: RTCPeerConnection,
    track: MediaStreamTrack,
    stream: MediaStream,
): void {
    const transceiver = peer.addTransceiver(track, {
        direction: "sendrecv",
        streams: [stream],
        sendEncodings: [
            {
                maxFramerate: 30,
                maxBitrate: 1_000_000,
                scaleResolutionDownBy: 1,
            },
        ],
    });
    const encodings = transceiver.sender.getParameters().encodings;
    if (
        transceiver.sender.track !== track ||
        encodings?.length !== 1 ||
        encodings[0].maxFramerate !== 30 ||
        encodings[0].maxBitrate !== 1_000_000 ||
        encodings[0].scaleResolutionDownBy !== 1
    )
        throw new ConnectError(
            "unsupported-media-profile",
            "Original video sender did not retain its bounded encoding",
        );
}

/** Constrain application-supplied capture as well as getUserMedia capture. */
export async function constrainVideoCapture(
    stream: MediaStream,
    timeoutMs: number,
): Promise<void> {
    const track = stream.getVideoTracks()[0];
    if (!track)
        throw new ConnectError(
            "unsupported-media-profile",
            "A live video track is required",
        );
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
        await Promise.race([
            track.applyConstraints({
                width: { max: 640 },
                height: { max: 360 },
                frameRate: { max: 30 },
            }),
            new Promise<never>((_, reject) => {
                timer = setTimeout(
                    () =>
                        reject(
                            new ConnectError(
                                "timeout",
                                "Video capture constraints timed out",
                            ),
                        ),
                    timeoutMs,
                );
            }),
        ]);
    } finally {
        clearTimeout(timer);
    }
    const settings = track.getSettings();
    if (
        !settings.width ||
        !settings.height ||
        !settings.frameRate ||
        settings.width > 640 ||
        settings.height > 360 ||
        settings.frameRate > 30
    )
        throw new ConnectError(
            "unsupported-media-profile",
            "Video capture must fit 640×360 at at most 30 fps",
        );
}

/** Owned encoded transform; its keyframe feedback is separate from capture. */
export function startVideoTransform(peer: RTCPeerConnection): Worker {
    // Capabilities contain one generic RTX entry without an apt parameter.
    // Preserve it so the browser can associate repair with the selected VP8 PT.
    const codecs = RTCRtpSender.getCapabilities("video")?.codecs.filter(
        (codec) =>
            ["video/vp8", "video/rtx"].includes(codec.mimeType.toLowerCase()),
    );
    if (!codecs?.some((codec) => codec.mimeType.toLowerCase() === "video/vp8"))
        throw new ConnectError(
            "unsupported-media-profile",
            "This browser does not support VP8",
        );
    const transceiver = peer
        .getTransceivers()
        .find((t) => t.sender.track?.kind === "video");
    if (
        !transceiver ||
        typeof RTCRtpScriptTransform !== "function" ||
        !("transform" in transceiver.sender)
    )
        throw new ConnectError(
            "unsupported-media-profile",
            "Video requires encoded-transform keyframe support",
        );
    transceiver.setCodecPreferences(codecs);
    const worker = new Worker(
        new URL("./browser_video_transform.js", import.meta.url),
        { type: "module" },
    );
    try {
        const port = keyframeAdapter(worker, transceiver.sender);
        const transform = new RTCRtpScriptTransform(
            worker,
            { keyframePort: port },
            [port],
        );
        transceiver.sender.transform = transform;
        if (transceiver.sender.transform !== transform)
            throw new ConnectError(
                "unsupported-media-profile",
                "Video sender did not retain its encoded transform",
            );
    } catch (error) {
        stopVideoWorker(worker);
        throw error;
    }
    return worker;
}

/** Configure negotiated send limits without allowing resolution adaptation. */
export async function configureVideoSender(
    peer: RTCPeerConnection,
): Promise<void> {
    const sender = peer.getSenders().find((s) => s.track?.kind === "video");
    if (!sender)
        throw new ConnectError(
            "unsupported-media-profile",
            "The peer did not retain its video sender",
        );
    const parameters = sender.getParameters();
    parameters.degradationPreference = "maintain-resolution";
    if (!parameters.encodings?.length) parameters.encodings = [{}];
    for (const encoding of parameters.encodings) {
        encoding.maxBitrate = 1_000_000;
        encoding.maxFramerate = 30;
        encoding.scaleResolutionDownBy = 1;
    }
    await sender.setParameters(parameters);
}
