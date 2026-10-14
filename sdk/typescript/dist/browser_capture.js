import { ConnectError } from "./errors.js";
/** Device-local IDs stay in capture constraints, never signaling or diagnostics. */
export function captureDevices(options, video, supplied) {
    const selection = {
        audioDeviceId: options.audioDeviceId,
        videoDeviceId: options.videoDeviceId,
    };
    for (const id of Object.values(selection))
        if (id !== undefined &&
            (typeof id !== "string" ||
                id.length === 0 ||
                id.length > 512 ||
                /[\u0000-\u001f\u007f-\u009f]/.test(id)))
            throw new ConnectError("invalid-request", "Use a valid local device selection");
    if ((!video && selection.videoDeviceId !== undefined) ||
        (supplied && Object.values(selection).some((id) => id !== undefined)))
        throw new ConnectError("invalid-request", "Device selection does not match the supplied capture profile");
    return selection;
}
/** A video join must not turn an audio-only or inactive answer into success. */
export function validateVideoAnswer(sdp) {
    const sections = sdp.replace(/\r/g, "").split(/\n(?=m=)/);
    const inherited = sections[0].match(/^a=(sendrecv|sendonly|recvonly|inactive)$/m)?.[1] ??
        "sendrecv";
    const media = sections.slice(1);
    const kinds = media.map((section) => section.split(/\r?\n/, 1)[0].split(/\s+/));
    const fail = () => {
        throw new ConnectError("unsupported-media-profile", "The peer did not accept duplex audio and VP8 video");
    };
    if (media.length > 3 ||
        kinds.filter((kind) => kind[0] === "m=audio").length !== 1 ||
        kinds.filter((kind) => kind[0] === "m=video").length !== 1 ||
        kinds.filter((kind) => kind[0] === "m=application").length > 1 ||
        kinds.some((kind) => !["m=audio", "m=video", "m=application"].includes(kind[0])))
        fail();
    for (const [index, section] of media.entries()) {
        const kind = kinds[index];
        // The established browser transport also negotiates its ordinary
        // Connect control datachannel. It is not a third media stream.
        if (kind[0] === "m=application")
            continue;
        const direction = section.match(/^a=(sendrecv|sendonly|recvonly|inactive)$/m)?.[1] ??
            inherited;
        if (!/^\d+$/.test(kind[1]) ||
            Number(kind[1]) === 0 ||
            direction !== "sendrecv")
            fail();
        if (kind[0] === "m=video") {
            if (kind.length < 4 ||
                !kind
                    .slice(3)
                    .every((pt) => /^\d+$/.test(pt) &&
                    new RegExp(`^a=rtpmap:${pt} VP8/90000\\r?$`, "im").test(section)))
                fail();
        }
    }
}
export async function microphoneStream(timeoutMs, video, devices = {}) {
    const pending = navigator.mediaDevices.getUserMedia({
        audio: devices.audioDeviceId
            ? { deviceId: { exact: devices.audioDeviceId } }
            : true,
        video: video
            ? {
                width: { ideal: 640, max: 640 },
                height: { ideal: 360, max: 360 },
                frameRate: { ideal: 30, max: 30 },
                ...(devices.videoDeviceId
                    ? { deviceId: { exact: devices.videoDeviceId } }
                    : {}),
            }
            : false,
    });
    return new Promise((resolve, reject) => {
        let expired = false;
        const timer = setTimeout(() => {
            expired = true;
            reject(new ConnectError("timeout", "Microphone access timed out"));
        }, timeoutMs);
        pending.then((stream) => {
            if (expired) {
                for (const track of stream.getTracks())
                    track.stop();
                return;
            }
            clearTimeout(timer);
            resolve(stream);
        }, (error) => {
            if (expired)
                return;
            clearTimeout(timer);
            reject(error);
        });
    });
}
export async function waitForIce(peer, timeoutMs) {
    if (peer.iceGatheringState === "complete")
        return;
    await new Promise((resolve, reject) => {
        const finish = (error) => {
            clearTimeout(timer);
            peer.removeEventListener("icegatheringstatechange", changed);
            if (error)
                reject(error);
            else
                resolve();
        };
        const timer = setTimeout(() => finish(new ConnectError("timeout", "ICE gathering timed out")), timeoutMs);
        const changed = () => {
            if (peer.iceGatheringState === "complete")
                finish();
        };
        peer.addEventListener("icegatheringstatechange", changed);
        // Handle a transition between the initial check and listener registration.
        changed();
    });
}
