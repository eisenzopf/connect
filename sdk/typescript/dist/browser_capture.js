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
    if (media.length !== 2 ||
        kinds.filter((kind) => kind[0] === "m=audio").length !== 1 ||
        kinds.filter((kind) => kind[0] === "m=video").length !== 1 ||
        kinds.some((kind) => !["m=audio", "m=video"].includes(kind[0])))
        fail();
    for (const [index, section] of media.entries()) {
        const kind = kinds[index];
        const direction = section.match(/^a=(sendrecv|sendonly|recvonly|inactive)$/m)?.[1] ??
            inherited;
        if (!/^\d+$/.test(kind[1]) ||
            Number(kind[1]) === 0 ||
            direction !== "sendrecv")
            fail();
        if (kind[0] === "m=video") {
            const tokens = kind.slice(3);
            const payloads = tokens.map(Number);
            if (!tokens.length ||
                tokens.some((pt) => !/^\d+$/.test(pt) || Number(pt) > 127) ||
                new Set(payloads).size !== payloads.length)
                fail();
            const codecs = new Map();
            for (const match of section.matchAll(/^a=rtpmap:(\d+)[ \t]+([^\n]+)$/gim)) {
                const pt = Number(match[1]);
                if (!payloads.includes(pt))
                    continue;
                if (codecs.has(pt))
                    fail();
                codecs.set(pt, match[2].trim().toLowerCase());
            }
            const primary = payloads.filter((pt) => codecs.get(pt) === "vp8/90000");
            if (!primary.length)
                fail();
            for (const pt of payloads) {
                if (primary.includes(pt))
                    continue;
                if (codecs.get(pt) !== "rtx/90000")
                    fail();
                const parameters = [
                    ...section.matchAll(/^a=fmtp:(\d+)[ \t]+([^\n]+)$/gim),
                ].filter((match) => Number(match[1]) === pt);
                if (parameters.length !== 1)
                    fail();
                const associations = parameters[0][2]
                    .split(";")
                    .map((value) => value.trim())
                    .filter((value) => /^apt(?:\s*=|$)/i.test(value));
                if (associations.length !== 1)
                    fail();
                const apt = /^apt\s*=\s*(\d+)$/i.exec(associations[0]);
                if (!apt || !primary.includes(Number(apt[1])))
                    fail();
            }
        }
    }
}
/** The public fixed Opus/VP8 program must be accepted in full, without extra channels. */
export function validateBrowserAnswer(sdp, video) {
    if (video)
        validateVideoAnswer(sdp);
    const sections = sdp.replace(/\r/g, "").split(/\n(?=m=)/);
    const inherited = sections[0].match(/^a=(sendrecv|sendonly|recvonly|inactive)$/m)?.[1] ??
        "sendrecv";
    const media = sections.slice(1), audio = media.filter((section) => section.startsWith("m=audio "));
    const fail = () => {
        throw new ConnectError("unsupported-media-profile", "The peer did not accept the original duplex Opus audio/video program");
    };
    if (media.length !== (video ? 2 : 1) || audio.length !== 1)
        fail();
    const section = audio[0], kind = section.split("\n", 1)[0].split(/\s+/);
    if (!/^\d+$/.test(kind[1]) ||
        Number(kind[1]) === 0 ||
        (section.match(/^a=(sendrecv|sendonly|recvonly|inactive)$/m)?.[1] ??
            inherited) !== "sendrecv")
        fail();
    const offered = new Set(kind.slice(3));
    if (![
        ...section.matchAll(/^a=rtpmap:(\d+)[ \t]+opus\/48000\/2[ \t]*$/gim),
    ].some((match) => offered.has(match[1])))
        fail();
}
/** Capture owns late permission results after deadline or caller cancellation. */
export async function microphoneStream(timeoutMs, video, devices = {}, signal) {
    if (!Number.isFinite(timeoutMs) || timeoutMs <= 0 || timeoutMs > 60000)
        throw new ConnectError("invalid-request", "Capture deadline must be positive and at most 60000 ms");
    const cancelled = () => new ConnectError("cancelled", "Capture wait cancelled");
    if (signal?.aborted)
        throw cancelled();
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
        let settled = false;
        const cleanup = () => {
            clearTimeout(timer);
            signal?.removeEventListener("abort", abort);
        };
        const fail = (error) => {
            if (settled)
                return;
            settled = true;
            cleanup();
            reject(error);
        };
        const abort = () => fail(cancelled());
        const timer = setTimeout(() => {
            fail(new ConnectError("timeout", "Microphone access timed out"));
        }, timeoutMs);
        signal?.addEventListener("abort", abort, { once: true });
        // A synchronous getUserMedia implementation may have retired the owner.
        if (signal?.aborted)
            abort();
        pending.then((stream) => {
            if (settled) {
                for (const track of stream.getTracks())
                    track.stop();
                return;
            }
            settled = true;
            cleanup();
            resolve(stream);
        }, (error) => fail(error));
    });
}
